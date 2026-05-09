use crate::*;

pub(crate) fn run_init(root: &Path) -> Result<(), String> {
    let check_path = root.join(CHECK_PATH);
    if check_path.exists() {
        return Err(format!("{} already exists", CHECK_PATH));
    }

    if let Some(parent) = check_path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(&check_path, DEFAULT_CHECK_TEMPLATE)
        .map_err(|err| format!("failed to write {}: {}", check_path.display(), err))?;
    println!("Created {}", CHECK_PATH);
    Ok(())
}

pub(crate) fn run_hook_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: canon hook install".to_string());
    }
    let action = arg_to_string(&args[0])?;
    match action.as_str() {
        "install" => run_hook_install(root),
        _ => Err(format!("unknown hook command: {}", action)),
    }
}

pub(crate) fn run_hook_install(root: &Path) -> Result<(), String> {
    preflight_pre_commit_hook(root)?;
    preflight_git_hooks_path(root)?;
    install_pre_commit_hook(root)
}

pub(crate) fn preflight_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if !hook_path.exists() {
        return Ok(());
    }

    let existing = fs::read_to_string(&hook_path)
        .map_err(|err| format!("failed to read {}: {}", hook_path.display(), err))?;
    if !pre_commit_hook_is_reusable(&existing) {
        return Err(format!(
            "{} already exists with different content",
            PRE_COMMIT_HOOK_PATH
        ));
    }
    Ok(())
}

pub(crate) fn preflight_git_hooks_path(root: &Path) -> Result<(), String> {
    if let Some(existing) = current_git_hooks_path(root)? {
        if existing == GIT_HOOKS_PATH {
            return Ok(());
        }
        if existing == LEGACY_GIT_HOOKS_PATH && legacy_pre_commit_hook_is_reusable(root)? {
            return Ok(());
        }
        if existing != GIT_HOOKS_PATH {
            return Err(format!(
                "git core.hooksPath is already set to {}; set it to {} manually if desired",
                existing, GIT_HOOKS_PATH
            ));
        }
    }
    Ok(())
}

pub(crate) fn legacy_pre_commit_hook_is_reusable(root: &Path) -> Result<bool, String> {
    let hook_path = root.join(LEGACY_PRE_COMMIT_HOOK_PATH);
    if !hook_path.exists() {
        return Ok(true);
    }
    let existing = fs::read_to_string(&hook_path)
        .map_err(|err| format!("failed to read {}: {}", hook_path.display(), err))?;
    Ok(pre_commit_hook_is_reusable(&existing))
}

pub(crate) fn pre_commit_hook_is_reusable(content: &str) -> bool {
    content == DEFAULT_PRE_COMMIT_HOOK
        || (content.contains("canon pre-commit:")
            && content.contains("canon gate")
            && content.contains("git status --porcelain -- .canon/"))
}

pub(crate) fn install_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = hook_path.parent() {
        ensure_dir(parent)?;
    }
    let hook_needs_write = !hook_path.exists()
        || fs::read_to_string(&hook_path).ok().as_deref() != Some(DEFAULT_PRE_COMMIT_HOOK);
    if hook_needs_write {
        fs::write(&hook_path, DEFAULT_PRE_COMMIT_HOOK)
            .map_err(|err| format!("failed to write {}: {}", hook_path.display(), err))?;
        println!("Installed {}", PRE_COMMIT_HOOK_PATH);
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path(root)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

#[cfg(not(unix))]
pub(crate) fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn configure_git_hooks_path(root: &Path) -> Result<(), String> {
    if !is_git_worktree(root)? {
        println!(
            "Git worktree not detected; {} was created but core.hooksPath was not set.",
            PRE_COMMIT_HOOK_PATH
        );
        return Ok(());
    }

    if current_git_hooks_path(root)?.as_deref() == Some(GIT_HOOKS_PATH) {
        println!("Git core.hooksPath already = {}", GIT_HOOKS_PATH);
        return Ok(());
    }

    set_git_hooks_path(root)?;
    println!("Configured git core.hooksPath = {}", GIT_HOOKS_PATH);
    Ok(())
}

pub(crate) fn current_git_hooks_path(root: &Path) -> Result<Option<String>, String> {
    if !is_git_worktree(root)? {
        return Ok(None);
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("--get")
        .arg("core.hooksPath")
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(Some(
            command_output_trimmed(&output.stdout, "git config stdout")?.to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "failed to read git core.hooksPath: {}",
        command_output_trimmed(&output.stderr, "git config stderr")?
    ))
}

pub(crate) fn set_git_hooks_path(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("core.hooksPath")
        .arg(GIT_HOOKS_PATH)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "failed to set git core.hooksPath: {}",
        command_output_trimmed(&output.stderr, "git config stderr")?
    ))
}

pub(crate) fn is_git_worktree(root: &Path) -> Result<bool, String> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to run git rev-parse: {}", err)),
    };
    Ok(output.status.success()
        && command_output_trimmed(&output.stdout, "git rev-parse stdout")? == "true")
}
