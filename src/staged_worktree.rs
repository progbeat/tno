use crate::*;

pub(crate) struct StagedWorktreeView {
    root: PathBuf,
    snapshot_root: PathBuf,
}

impl StagedWorktreeView {
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let snapshot_root = create_snapshot_root()?;
        if let Err(err) = materialize_staged_snapshot(root, &snapshot_root) {
            let _ = fs::remove_dir_all(&snapshot_root);
            return Err(err);
        }
        Ok(StagedWorktreeView {
            root: root.to_path_buf(),
            snapshot_root,
        })
    }

    pub(crate) fn root(&self) -> &Path {
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
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("checkout-index")
        .arg("--all")
        .arg("--force")
        .arg(format!("--prefix={}", prefix))
        .output()
        .map_err(|err| format!("failed to run git checkout-index: {}", err))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to materialize staged snapshot: {}",
            command_output_trimmed(&output.stderr, "git checkout-index stderr")?
        ))
    }
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

fn create_snapshot_root() -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    for parent in snapshot_parent_candidates() {
        match create_snapshot_root_in(&parent) {
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
