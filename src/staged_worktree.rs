use crate::*;

pub(crate) struct StagedWorktreeView {
    root: PathBuf,
    stash_oid: Option<String>,
}

impl StagedWorktreeView {
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        if !has_unstaged_or_untracked_changes(root)? {
            return Ok(StagedWorktreeView {
                root: root.to_path_buf(),
                stash_oid: None,
            });
        }

        let before = current_stash_oid(root)?;
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "stash",
                "push",
                "--keep-index",
                "--include-untracked",
                "-m",
                "canon check: preserve unstaged changes while checking the index",
            ])
            .output()
            .map_err(|err| format!("failed to run git stash: {}", err))?;
        if !output.status.success() {
            return Err(format!(
                "failed to prepare staged worktree view: {}",
                command_output_trimmed(&output.stderr, "git stash stderr")?
            ));
        }

        let after = current_stash_oid(root)?;
        let stash_oid = if after.is_some() && after != before {
            after
        } else {
            None
        };
        Ok(StagedWorktreeView {
            root: root.to_path_buf(),
            stash_oid,
        })
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        let Some(stash_oid) = self.stash_oid.as_deref() else {
            return;
        };
        match restore_staged_worktree_view(&self.root, stash_oid) {
            Ok(()) => {
                if let Err(err) = drop_stash_by_oid(&self.root, stash_oid) {
                    let _ = append_runtime_log_event(
                        &self.root,
                        "warn",
                        "worktree.stash.drop.failed",
                        &[("stashOid", json!(stash_oid)), ("error", json!(err))],
                    );
                }
            }
            Err(err) => {
                eprint!("{}", stash_recovery_message(&self.root, stash_oid, &err));
                let _ = append_runtime_log_event(
                    &self.root,
                    "error",
                    "worktree.restore.failed",
                    &[("stashOid", json!(stash_oid)), ("error", json!(err))],
                );
            }
        }
    }
}

pub(crate) fn restore_staged_worktree_view(root: &Path, stash_ref: &str) -> Result<(), String> {
    let restore_tracked = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["restore", "--worktree", "--source", stash_ref, "--", "."])
        .status()
        .map_err(|err| format!("failed to run git restore: {}", err))?;
    if !restore_tracked.success() {
        return Err(format!(
            "failed to restore tracked working tree changes (exit {})",
            restore_tracked
        ));
    }
    restore_untracked_from_stash(root, stash_ref)
}

pub(crate) fn restore_untracked_from_stash(root: &Path, stash_ref: &str) -> Result<(), String> {
    let source = format!("{}^3", stash_ref);
    if !git_revision_exists(root, &source)? {
        return Ok(());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-tree", "-rz", "--name-only", &source])
        .output()
        .map_err(|err| format!("failed to run git ls-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect untracked stash tree {}: {}",
            source,
            command_output_trimmed(&output.stderr, "git ls-tree stderr")?
        ));
    }
    let paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(git_path_from_raw_bytes)
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Ok(());
    }
    let mut restore = Command::new("git");
    restore
        .arg("-C")
        .arg(root)
        .args(["restore", "--worktree", "--source", &source, "--"]);
    restore.args(paths);
    let status = restore
        .status()
        .map_err(|err| format!("failed to run git restore: {}", err))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to restore untracked working tree changes (exit {})",
            status
        ))
    }
}

fn has_unstaged_or_untracked_changes(root: &Path) -> Result<bool, String> {
    let diff = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--quiet"])
        .status()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !diff.success() {
        match diff.code() {
            Some(1) => return Ok(true),
            _ => {
                return Err(format!(
                    "failed to inspect unstaged changes (exit {})",
                    diff
                ));
            }
        }
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .map_err(|err| format!("failed to run git ls-files: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect untracked changes: {}",
            command_output_trimmed(&output.stderr, "git ls-files stderr")?
        ));
    }
    Ok(!command_output_trimmed(&output.stdout, "git ls-files stdout")?.is_empty())
}

fn current_stash_oid(root: &Path) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", "refs/stash"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if output.status.success() {
        return Ok(Some(
            command_output_trimmed(&output.stdout, "git rev-parse stdout")?.to_string(),
        ));
    }
    Ok(None)
}

fn drop_stash_by_oid(root: &Path, stash_oid: &str) -> Result<(), String> {
    let Some(stash_ref) = stash_ref_for_oid(root, stash_oid)? else {
        return Ok(());
    };
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["stash", "drop", "--quiet", &stash_ref])
        .status()
        .map_err(|err| format!("failed to run git stash drop: {}", err))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to drop stash {} (exit {})",
            stash_ref, status
        ))
    }
}

fn stash_ref_for_oid(root: &Path, stash_oid: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["stash", "list", "--format=%H"])
        .output()
        .map_err(|err| format!("failed to run git stash list: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect stash list: {}",
            command_output_trimmed(&output.stderr, "git stash list stderr")?
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git stash list output must be valid UTF-8".to_string())?;
    Ok(stdout
        .lines()
        .position(|oid| oid == stash_oid)
        .map(|index| format!("stash@{{{}}}", index)))
}

pub(crate) fn stash_recovery_message(root: &Path, stash_oid: &str, error: &str) -> String {
    format!(
        "canon: failed to restore unstaged changes from stash {stash_oid}: {error}\n\
canon: the preserved changes were not dropped. Inspect them with:\n\
canon:   git -C {} stash list --format='%H %gd %s'\n\
canon: recover them with:\n\
canon:   git -C {} stash apply {stash_oid}\n\
canon: after recovery, drop only the stash whose %H is {stash_oid}.\n",
        root.display(),
        root.display()
    )
}
