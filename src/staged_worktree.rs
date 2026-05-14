use crate::logging::append_runtime_log_event;
use crate::project::command_output_trimmed;
use serde_json::json;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

pub(crate) struct StagedWorktreeView {
    root: PathBuf,
    snapshot_root: PathBuf,
}

impl StagedWorktreeView {
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let snapshot_root = create_snapshot_root(root)?;
        if let Err(err) = materialize_staged_snapshot(root, &snapshot_root) {
            let _ = fs::remove_dir_all(&snapshot_root);
            return Err(err);
        }
        Ok(StagedWorktreeView {
            root: root.to_path_buf(),
            snapshot_root,
        })
    }

    pub(crate) fn snapshot_root(&self) -> &Path {
        &self.snapshot_root
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_dir_all(&self.snapshot_root) {
            let _ = append_runtime_log_event(
                &self.root,
                "warn",
                "snapshot.cleanup.failed",
                &[
                    ("path", json!(self.snapshot_root.display().to_string())),
                    ("error", json!(err.to_string())),
                ],
            );
        }
    }
}

fn materialize_staged_snapshot(root: &Path, snapshot_root: &Path) -> Result<(), String> {
    let prefix = checkout_index_prefix(snapshot_root)?;
    // Keep staged symlinks as regular files containing the link target. That
    // prevents evaluator reads from following a tracked symlink out of the
    // staged snapshot while still showing the staged link target text.
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("-c")
        .arg("core.symlinks=false")
        .arg("checkout-index")
        .arg("--all")
        .arg("--force")
        .arg(format!("--prefix={}", prefix))
        .output()
        .map_err(|err| format!("failed to run git checkout-index: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to materialize staged snapshot: {}",
            command_output_trimmed(&output.stderr, "git checkout-index stderr")?
        ));
    }
    validate_snapshot_contains_no_symlinks(snapshot_root)
}

fn validate_snapshot_contains_no_symlinks(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "staged snapshot contains symlink {}; refusing to expose symlinks to evaluator sessions",
            path.display()
        ));
    }
    if file_type.is_dir() {
        for entry in fs::read_dir(path).map_err(|err| {
            format!(
                "failed to read snapshot directory {}: {}",
                path.display(),
                err
            )
        })? {
            let entry = entry.map_err(|err| {
                format!(
                    "failed to read snapshot directory {}: {}",
                    path.display(),
                    err
                )
            })?;
            validate_snapshot_contains_no_symlinks(&entry.path())?;
        }
    }
    Ok(())
}

fn checkout_index_prefix(snapshot_root: &Path) -> Result<String, String> {
    let mut prefix = snapshot_root
        .to_str()
        .ok_or_else(|| {
            format!(
                "staged snapshot path must be valid UTF-8: {}",
                snapshot_root.display()
            )
        })?
        .to_string();
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    Ok(prefix)
}

fn create_snapshot_root(root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize project root {}: {}",
            root.display(),
            err
        )
    })?;
    let mut errors = Vec::new();
    for parent in snapshot_parent_candidates() {
        match snapshot_parent_outside_worktree(&root, &parent)
            .and_then(|()| create_snapshot_root_in(&parent))
            .and_then(|path| verify_snapshot_root_outside_worktree(&root, path))
        {
            Ok(path) => return Ok(path),
            Err(err) => errors.push(err),
        }
    }
    Err(format!(
        "failed to create staged snapshot directory: {}",
        errors.join("; ")
    ))
}

fn snapshot_parent_candidates() -> Vec<PathBuf> {
    let mut parents = Vec::new();
    if cfg!(target_os = "linux") {
        parents.push(PathBuf::from("/dev/shm"));
    }
    let temp_dir = env::temp_dir();
    if !parents.iter().any(|parent| parent == &temp_dir) {
        parents.push(temp_dir);
    }
    parents
}

pub(crate) fn snapshot_parent_outside_worktree(root: &Path, parent: &Path) -> Result<(), String> {
    let parent = parent.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize staged snapshot parent {}: {}",
            parent.display(),
            err
        )
    })?;
    if parent == root || parent.starts_with(root) {
        Err(format!(
            "staged snapshot parent {} is inside project root {}",
            parent.display(),
            root.display()
        ))
    } else {
        Ok(())
    }
}

fn verify_snapshot_root_outside_worktree(root: &Path, path: PathBuf) -> Result<PathBuf, String> {
    let snapshot_root = path.canonicalize().map_err(|err| {
        format!(
            "failed to canonicalize staged snapshot root {}: {}",
            path.display(),
            err
        )
    })?;
    if snapshot_root == root || snapshot_root.starts_with(root) {
        let _ = fs::remove_dir_all(&path);
        Err(format!(
            "staged snapshot root {} is inside project root {}",
            snapshot_root.display(),
            root.display()
        ))
    } else {
        Ok(path)
    }
}

fn create_snapshot_root_in(parent: &Path) -> Result<PathBuf, String> {
    if !parent.is_dir() {
        return Err(format!("{} is not a directory", parent.display()));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..1000 {
        let path = parent.join(format!(
            "canon-check-snapshot-{}-{}-{}",
            process::id(),
            stamp,
            attempt
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!("failed to create {}: {}", path.display(), err));
            }
        }
    }
    Err(format!(
        "failed to allocate a unique staged snapshot directory under {}",
        parent.display()
    ))
}
