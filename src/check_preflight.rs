use crate::*;

pub(crate) fn install_sigint_handler() {
    SIGNAL_HANDLER_INIT.call_once(|| {
        #[cfg(unix)]
        unsafe {
            const SIGHUP: i32 = 1;
            const SIGINT: i32 = 2;
            const SIGTERM: i32 = 15;
            let _ = signal(SIGHUP, handle_sigint);
            let _ = signal(SIGINT, handle_sigint);
            let _ = signal(SIGTERM, handle_sigint);
        }
    });
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) fn staged_changed_paths(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-status")
        .arg("-z")
        .arg("--diff-filter=ACDMRTUXB")
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    staged_changed_paths_from_name_status_z(&output.stdout)
}

pub(crate) fn staged_changed_paths_from_name_status_z(
    stdout: &[u8],
) -> Result<Vec<String>, String> {
    let mut fields = stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else {
            return Err("git diff name-status output ended before path".to_string());
        };
        paths.push(String::from_utf8_lossy(path).into_owned());
        if status.starts_with(b"R") || status.starts_with(b"C") {
            let Some(path) = fields.next() else {
                return Err("git diff name-status output ended before rename/copy path".to_string());
            };
            paths.push(String::from_utf8_lossy(path).into_owned());
        }
    }
    Ok(paths)
}

pub(crate) fn fail_on_mixed_canon_paths(paths: &[String]) -> Result<(), String> {
    let has_canon = paths.iter().any(|path| is_canon_project_path(path));
    let has_other = paths.iter().any(|path| !is_canon_project_path(path));
    if has_canon && has_other {
        return Err(
            "canon check failed: .canon/** changes must not be mixed with non-.canon changes"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn is_canon_project_path(path: &str) -> bool {
    path == ".canon" || path.starts_with(".canon/")
}

pub(crate) fn is_canon_only_staged_change(paths: &[String]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_canon_project_path(path))
}
