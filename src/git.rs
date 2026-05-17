use crate::project::{command_output_trimmed, path_from_git_stdout};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    Ok(root.join(path_from_git_stdout(output.stdout)))
}

pub(crate) fn git_head_tree_exists(root: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", "HEAD^{tree}"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    Ok(output.status.success())
}

pub(crate) fn read_staged_file_content(
    root: &Path,
    path: impl AsRef<Path>,
) -> Result<String, String> {
    let path = path.as_ref();
    let output = read_staged_file_bytes_from_raw_path(root, &git_path_bytes(path)?)?;
    String::from_utf8(output).map_err(|_| format!("staged {} must be valid UTF-8", path.display()))
}

pub(crate) fn read_staged_file_bytes_from_raw_path(
    root: &Path,
    path: &[u8],
) -> Result<Vec<u8>, String> {
    let mut revision = Vec::with_capacity(path.len() + 3);
    // Use explicit stage-0 index syntax so literal paths that begin with `:`
    // are not parsed as another Git revision form.
    revision.extend_from_slice(b":0:");
    revision.extend_from_slice(path);
    let revision = git_path_from_raw_bytes(&revision)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("show")
        .arg(revision)
        .output()
        .map_err(|err| format!("failed to run git show: {}", err))?;
    if !output.status.success() {
        let path = String::from_utf8_lossy(path);
        return Err(format!(
            "failed to read staged {}: {}",
            path,
            command_output_trimmed(&output.stderr, "git show stderr")?
        ));
    }
    Ok(output.stdout)
}

pub(crate) fn staged_tracked_path_bytes(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|err| format!("failed to run git ls-files: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect staged files: {}",
            command_output_trimmed(&output.stderr, "git ls-files stderr")?
        ));
    }
    let mut paths = Vec::new();
    for path in output.stdout.split(|byte| *byte == 0) {
        if path.is_empty() {
            continue;
        }
        paths.push(path.to_vec());
    }
    Ok(paths)
}

#[cfg(unix)]
fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::ffi::OsStrExt;

    Ok(path.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
fn git_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    Ok(path
        .to_str()
        .ok_or_else(|| format!("git path must be valid UTF-8: {}", path.display()))?
        .as_bytes()
        .to_vec())
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
