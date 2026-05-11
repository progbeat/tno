use crate::*;

pub(crate) fn print_root(config: &Config) -> Result<(), String> {
    ensure_dir(&config.root)?;
    println!("{}", config.root.display());
    Ok(())
}

impl Config {
    pub(crate) fn from_env() -> Result<Config, String> {
        let thread_id = env::var("CODEX_THREAD_ID")
            .map_err(|_| "CODEX_THREAD_ID is required in v1".to_string())?;
        if thread_id.trim().is_empty() {
            return Err("CODEX_THREAD_ID is empty".to_string());
        }
        if thread_id == "."
            || thread_id == ".."
            || thread_id.contains('/')
            || thread_id.contains('\\')
        {
            return Err("CODEX_THREAD_ID must be a single path segment".to_string());
        }

        if let Some(value) = env::var_os("CANON_HOME") {
            if !value.is_empty() {
                // This directory holds user-retained notes for the active
                // Codex thread. It is bounded by the retained note set; cache
                // and log state use separate project-local bounded stores.
                return Ok(Config {
                    root: PathBuf::from(value).join("codex").join(thread_id),
                });
            }
        }

        let temp_root = env::var_os("TMPDIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);

        Ok(Config {
            // TMPDIR fallback has the same retained-note semantics as
            // CANON_HOME, but lives under the OS temporary hierarchy.
            root: temp_root.join("canon").join("codex").join(thread_id),
        })
    }
}

pub(crate) fn project_root_or_current(start: &Path) -> Result<PathBuf, String> {
    match git_project_root(start) {
        Ok(root) => Ok(root),
        Err(_) => env::current_dir().map_err(|err| format!("failed to read current dir: {}", err)),
    }
}

pub(crate) fn command_output_utf8<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    std::str::from_utf8(bytes)
        .map_err(|err| format!("{} must be valid UTF-8: {}", description, err))
}

pub(crate) fn command_output_trimmed<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    Ok(command_output_utf8(bytes, description)?.trim())
}

pub(crate) fn git_project_root(start: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to find git project root: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    Ok(path_from_git_stdout(output.stdout))
}

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> PathBuf {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&bytes).to_string())
    }
}
