use crate::hooks::current_git_hooks_path_for_worktree;
use crate::scope_hash::ScopeHashCache;
use crate::staged_worktree_git::run_git_command;
use crate::PRE_COMMIT_HOOK_PATH;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn copy_local_hook_metadata(
    root: &Path,
    snapshot_root: &Path,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<(), String> {
    if let Some(hooks_path) = current_git_hooks_path_for_worktree(root)? {
        run_git_command(
            Command::new("git").arg("-C").arg(snapshot_root).args([
                "config",
                "core.hooksPath",
                &hooks_path,
            ]),
            "git config",
            "failed to copy staged snapshot Git hook config",
        )?;
    }

    let Some(snapshot) =
        scope_hash_cache.local_git_file_snapshot(root, "canon/hooks/pre-commit")?
    else {
        return Ok(());
    };
    let target = snapshot_root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create staged snapshot hooks directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }
    fs::write(&target, &snapshot.content).map_err(|err| {
        format!(
            "failed to write {} in staged snapshot: {}",
            target.display(),
            err
        )
    })?;
    fs::set_permissions(&target, snapshot.permissions)
        .map_err(|err| format!("failed to chmod {}: {}", target.display(), err))
}
