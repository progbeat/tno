use crate::fs_util::ensure_dir;
use crate::notes_cli::arg_to_string;
use crate::output::write_stdout_line;
use crate::project::command_output_trimmed;
use crate::{
    AGENTS_PATH, CHECK_PATH, DEFAULT_AGENTS_TEMPLATE, DEFAULT_CHECK_TEMPLATE,
    DEFAULT_PRE_COMMIT_HOOK, GIT_HOOKS_PATH, PRE_COMMIT_HOOK_PATH,
};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

pub(crate) fn run_init(root: &Path) -> Result<(), String> {
    let check_path = root.join(CHECK_PATH);
    if check_path.exists() {
        return Err(format!("{} already exists", CHECK_PATH));
    }

    // These are user-owned project configuration files, not canon runtime
    // state: they live in the worktree so humans can review and version them.
    if let Some(parent) = check_path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(&check_path, DEFAULT_CHECK_TEMPLATE)
        .map_err(|err| format!("failed to write {}: {}", check_path.display(), err))?;
    write_stdout_line(&format!("Created {}", CHECK_PATH))?;
    ensure_agents_file(root)?;
    Ok(())
}

pub(crate) fn ensure_agents_file(root: &Path) -> Result<(), String> {
    let agents_path = root.join(AGENTS_PATH);
    if agents_path.exists() {
        write_stdout_line(&format!(
            "{} already exists; merge canon's AGENTS.md rules into it if they are missing:\n{}",
            AGENTS_PATH,
            DEFAULT_AGENTS_TEMPLATE.trim_end()
        ))?;
        return Ok(());
    }

    fs::write(&agents_path, DEFAULT_AGENTS_TEMPLATE)
        .map_err(|err| format!("failed to write {}: {}", agents_path.display(), err))?;
    write_stdout_line(&format!("Created {}", AGENTS_PATH))
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
    let state = HookInstallState::load(root)?;
    preflight_pre_commit_hook_content(state.pre_commit_hook.as_deref())?;
    preflight_git_hooks_path_state(&state)?;
    install_pre_commit_hook_with_state(root, &state)
}

pub(crate) fn preflight_pre_commit_hook_content(content: Option<&str>) -> Result<(), String> {
    if let Some(existing) = content {
        if pre_commit_hook_is_reusable(existing) {
            return Ok(());
        }
        return Err(format!(
            "{} already exists with different content",
            PRE_COMMIT_HOOK_PATH
        ));
    }
    Ok(())
}

pub(crate) fn preflight_git_hooks_path_state(state: &HookInstallState) -> Result<(), String> {
    if let Some(existing) = state.current_git_hooks_path.as_deref() {
        if existing == GIT_HOOKS_PATH {
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

pub(crate) fn pre_commit_hook_is_reusable(content: &str) -> bool {
    content == DEFAULT_PRE_COMMIT_HOOK
}

pub(crate) fn install_pre_commit_hook_with_state(
    root: &Path,
    state: &HookInstallState,
) -> Result<(), String> {
    // Hook installation writes Git integration configuration, not canon-owned
    // persistent state. Internal cache, logs, and note state stay under the
    // repository's `git rev-parse --git-path canon` directory.
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = hook_path.parent() {
        ensure_dir(parent)?;
    }
    if state.pre_commit_hook.as_deref() != Some(DEFAULT_PRE_COMMIT_HOOK) {
        fs::write(&hook_path, DEFAULT_PRE_COMMIT_HOOK)
            .map_err(|err| format!("failed to write {}: {}", hook_path.display(), err))?;
        write_stdout_line(&format!("Installed {}", PRE_COMMIT_HOOK_PATH))?;
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path_with_state(root, state)?;
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

pub(crate) fn configure_git_hooks_path_with_state(
    root: &Path,
    state: &HookInstallState,
) -> Result<(), String> {
    if !state.is_git_worktree {
        write_stdout_line(&format!(
            "Git worktree not detected; {} was created but core.hooksPath was not set.",
            PRE_COMMIT_HOOK_PATH
        ))?;
        return Ok(());
    }

    if state.current_git_hooks_path.as_deref() == Some(GIT_HOOKS_PATH) {
        write_stdout_line(&format!("Git core.hooksPath already = {}", GIT_HOOKS_PATH))?;
        return Ok(());
    }

    set_git_hooks_path(root)?;
    write_stdout_line(&format!(
        "Configured git core.hooksPath = {}",
        GIT_HOOKS_PATH
    ))
}

pub(crate) fn current_git_hooks_path_for_worktree(root: &Path) -> Result<Option<String>, String> {
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

pub(crate) struct HookInstallState {
    pre_commit_hook: Option<String>,
    current_git_hooks_path: Option<String>,
    is_git_worktree: bool,
}

impl HookInstallState {
    pub(crate) fn load(root: &Path) -> Result<HookInstallState, String> {
        let is_git_worktree = is_git_worktree(root)?;
        Ok(HookInstallState {
            pre_commit_hook: read_optional_file(&root.join(PRE_COMMIT_HOOK_PATH))?,
            current_git_hooks_path: if is_git_worktree {
                current_git_hooks_path_for_worktree(root)?
            } else {
                None
            },
            is_git_worktree,
        })
    }
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {}", path.display(), err)),
    }
}
