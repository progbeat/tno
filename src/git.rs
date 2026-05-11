use crate::*;

pub(crate) fn resolve_git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--git-path")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve git path {}: {}",
            path,
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    let resolved = String::from_utf8(output.stdout)
        .map_err(|_| "git rev-parse output must be valid UTF-8".to_string())?;
    Ok(root.join(resolved.trim()))
}

pub(crate) fn read_staged_file_content(root: &Path, path: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("show")
        .arg(format!(":{}", path))
        .output()
        .map_err(|err| format!("failed to run git show: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to read staged {}: {}",
            path,
            command_output_trimmed(&output.stderr, "git show stderr")?
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("staged {} must be valid UTF-8", path))
}

#[cfg(unix)]
pub(crate) fn git_path_from_raw_bytes(path: &[u8]) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStrExt;

    Ok(std::ffi::OsStr::from_bytes(path).to_os_string())
}

#[cfg(not(unix))]
pub(crate) fn git_path_from_raw_bytes(path: &[u8]) -> Result<OsString, String> {
    String::from_utf8(path.to_vec())
        .map(OsString::from)
        .map_err(|_| "git path must be valid UTF-8 on this platform".to_string())
}

pub(crate) fn git_revision_exists(root: &Path, revision: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", revision])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    Err(format!(
        "failed to inspect git revision {}: {}",
        revision,
        command_output_trimmed(&output.stderr, "git rev-parse stderr")?
    ))
}
