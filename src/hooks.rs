fn run_init(root: &Path) -> Result<(), String> {
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

fn run_hook_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: canon hook install".to_string());
    }
    let action = arg_to_string(&args[0])?;
    match action.as_str() {
        "install" => run_hook_install(root),
        _ => Err(format!("unknown hook command: {}", action)),
    }
}

fn run_hook_install(root: &Path) -> Result<(), String> {
    preflight_pre_commit_hook(root)?;
    preflight_git_hooks_path(root)?;
    install_pre_commit_hook(root)
}

fn preflight_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if !hook_path.exists() {
        return Ok(());
    }

    let existing = fs::read_to_string(&hook_path)
        .map_err(|err| format!("failed to read {}: {}", hook_path.display(), err))?;
    if existing != DEFAULT_PRE_COMMIT_HOOK {
        return Err(format!(
            "{} already exists with different content",
            PRE_COMMIT_HOOK_PATH
        ));
    }
    Ok(())
}

fn preflight_git_hooks_path(root: &Path) -> Result<(), String> {
    if let Some(existing) = current_git_hooks_path(root)? {
        if existing != GIT_HOOKS_PATH {
            return Err(format!(
                "git core.hooksPath is already set to {}; set it to {} manually if desired",
                existing, GIT_HOOKS_PATH
            ));
        }
    }
    Ok(())
}

fn install_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = hook_path.parent() {
        ensure_dir(parent)?;
    }
    if !hook_path.exists() {
        fs::write(&hook_path, DEFAULT_PRE_COMMIT_HOOK)
            .map_err(|err| format!("failed to write {}: {}", hook_path.display(), err))?;
        println!("Created {}", PRE_COMMIT_HOOK_PATH);
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path(root)?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn configure_git_hooks_path(root: &Path) -> Result<(), String> {
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

fn current_git_hooks_path(root: &Path) -> Result<Option<String>, String> {
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
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "failed to read git core.hooksPath: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn set_git_hooks_path(root: &Path) -> Result<(), String> {
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
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn is_git_worktree(root: &Path) -> Result<bool, String> {
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
    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}
