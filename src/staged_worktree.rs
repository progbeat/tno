use crate::git::git_path_from_raw_bytes;
use crate::logging::append_runtime_log_event;
use crate::project::command_output_trimmed;
use crate::scope_hash::ScopeHashCache;
use crate::staged_worktree_git::run_git_command;
use crate::staged_worktree_hooks::copy_local_hook_metadata;
use crate::staged_worktree_paths::create_snapshot_root;
#[cfg(test)]
pub(crate) use crate::staged_worktree_paths::snapshot_parent_outside_worktree;
use crate::staged_worktree_validate::validate_snapshot_contains_no_symlinks;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct StagedWorktreeView {
    root: PathBuf,
    snapshot_root: PathBuf,
}

impl StagedWorktreeView {
    #[cfg(test)]
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        let mut scope_hash_cache = ScopeHashCache::new();
        StagedWorktreeView::apply_with_scope_hash_cache(root, &mut scope_hash_cache)
    }

    pub(crate) fn apply_with_scope_hash_cache(
        root: &Path,
        scope_hash_cache: &mut ScopeHashCache,
    ) -> Result<StagedWorktreeView, String> {
        let snapshot_root = create_snapshot_root(root)?;
        if let Err(err) = materialize_staged_snapshot(root, &snapshot_root, scope_hash_cache) {
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

fn materialize_staged_snapshot(
    root: &Path,
    snapshot_root: &Path,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<(), String> {
    initialize_snapshot_git_repo(root, snapshot_root, scope_hash_cache)?;
    clear_snapshot_worktree(snapshot_root)?;
    checkout_staged_index(root, snapshot_root)?;
    mark_staged_additions_intent_to_add(root, snapshot_root)?;
    validate_snapshot_contains_no_symlinks(snapshot_root)
}

fn checkout_staged_index(root: &Path, snapshot_root: &Path) -> Result<(), String> {
    // Keep staged symlinks as regular files containing the link target. That
    // prevents evaluator reads from following a tracked symlink out of the
    // staged snapshot while still showing the staged link target text.
    checkout_index_into_snapshot(
        root,
        snapshot_root,
        None,
        "failed to materialize staged snapshot",
    )
}

fn initialize_snapshot_git_repo(
    root: &Path,
    snapshot_root: &Path,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<(), String> {
    let template = snapshot_root.join(".canon-empty-git-template");
    fs::create_dir(&template).map_err(|err| {
        format!(
            "failed to create empty Git template directory {}: {}",
            template.display(),
            err
        )
    })?;
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .arg("init")
            .arg("--quiet")
            .arg(format!("--template={}", template.display())),
        "git init",
        "failed to initialize staged snapshot Git metadata",
    )?;
    let _ = fs::remove_dir(&template);
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.eol", "lf"),
        ("core.symlinks", "false"),
    ] {
        set_snapshot_git_config(snapshot_root, key, value)?;
    }

    if !scope_hash_cache.git_has_head(root)? {
        copy_local_hook_metadata(root, snapshot_root, scope_hash_cache)?;
        return Ok(());
    }

    checkout_head_tree(root, snapshot_root)?;
    commit_snapshot_baseline(snapshot_root)?;
    copy_local_hook_metadata(root, snapshot_root, scope_hash_cache)
}

fn set_snapshot_git_config(snapshot_root: &Path, key: &str, value: &str) -> Result<(), String> {
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .args(["config", key, value]),
        "git config",
        "failed to configure staged snapshot Git metadata",
    )
}

fn checkout_head_tree(root: &Path, snapshot_root: &Path) -> Result<(), String> {
    let temp_index = snapshot_root.join(".git").join("canon-head.index");
    let read_tree = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["read-tree", "HEAD"])
        .env("GIT_INDEX_FILE", &temp_index)
        .output()
        .map_err(|err| format!("failed to run git read-tree: {}", err))?;
    if !read_tree.status.success() {
        let _ = fs::remove_file(&temp_index);
        return Err(format!(
            "failed to prepare staged snapshot Git baseline: {}",
            command_output_trimmed(&read_tree.stderr, "git read-tree stderr")?
        ));
    }

    let checkout = checkout_index_into_snapshot(
        root,
        snapshot_root,
        Some(&temp_index),
        "failed to materialize staged snapshot Git baseline",
    );
    let _ = fs::remove_file(&temp_index);
    checkout
}

fn checkout_index_into_snapshot(
    root: &Path,
    snapshot_root: &Path,
    index_file: Option<&Path>,
    failure_message: &str,
) -> Result<(), String> {
    let prefix = checkout_index_prefix(snapshot_root)?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("-c")
        .arg("core.symlinks=false")
        .arg("checkout-index")
        .arg("--all")
        .arg("--force")
        .arg(format!("--prefix={}", prefix));
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git checkout-index: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            failure_message,
            command_output_trimmed(&output.stderr, "git checkout-index stderr")?
        ));
    }
    Ok(())
}

fn commit_snapshot_baseline(snapshot_root: &Path) -> Result<(), String> {
    run_git_command(
        Command::new("git")
            .arg("-C")
            .arg(snapshot_root)
            .args(["add", "--all"]),
        "git add",
        "failed to stage snapshot Git baseline",
    )?;
    run_git_command(
        Command::new("git").arg("-C").arg(snapshot_root).args([
            "-c",
            "user.name=Canon Snapshot",
            "-c",
            "user.email=canon@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "canon snapshot baseline",
        ]),
        "git commit",
        "failed to commit snapshot Git baseline",
    )
}

fn clear_snapshot_worktree(snapshot_root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(snapshot_root).map_err(|err| {
        format!(
            "failed to read staged snapshot directory {}: {}",
            snapshot_root.display(),
            err
        )
    })? {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read staged snapshot directory {}: {}",
                snapshot_root.display(),
                err
            )
        })?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            format!(
                "failed to inspect staged snapshot entry {}: {}",
                path.display(),
                err
            )
        })?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|err| format!("failed to remove {}: {}", path.display(), err))?;
        } else {
            fs::remove_file(&path)
                .map_err(|err| format!("failed to remove {}: {}", path.display(), err))?;
        }
    }
    Ok(())
}

fn mark_staged_additions_intent_to_add(root: &Path, snapshot_root: &Path) -> Result<(), String> {
    let paths = staged_new_path_bytes(root)?;
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(snapshot_root)
        .arg("--literal-pathspecs")
        .args(["add", "-N", "-f", "--"]);
    for path in paths {
        command.arg(git_path_from_raw_bytes(&path)?);
    }
    run_git_command(
        &mut command,
        "git add",
        "failed to mark staged additions in snapshot Git metadata",
    )
}

fn staged_new_path_bytes(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "diff",
            "--cached",
            "--name-only",
            "--diff-filter=ACR",
            "-z",
            "--",
        ])
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect staged additions: {}",
            command_output_trimmed(&output.stderr, "git diff stderr")?
        ));
    }
    let mut paths = Vec::new();
    for path in output.stdout.split(|byte| *byte == 0) {
        if !path.is_empty() {
            paths.push(path.to_vec());
        }
    }
    Ok(paths)
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
