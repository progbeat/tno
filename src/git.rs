fn git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
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
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let resolved = String::from_utf8(output.stdout)
        .map_err(|_| "git rev-parse output must be valid UTF-8".to_string())?;
    Ok(root.join(resolved.trim()))
}

fn staged_file_content(root: &Path, path: &str) -> Result<String, String> {
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
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("staged {} must be valid UTF-8", path))
}

fn git_write_tree(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("write-tree")
        .output()
        .map_err(|err| format!("failed to run git write-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to write staged git tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

struct StagedSnapshot {
    path: PathBuf,
}

impl StagedSnapshot {
    fn create(root: &Path, tree: &str) -> Result<StagedSnapshot, String> {
        let path = unique_temp_dir("canon-staged-snapshot")?;
        ensure_dir(&path)?;
        let mut archive = Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("archive")
            .arg("--format=tar")
            .arg(tree)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start git archive: {}", err))?;
        let archive_stdout = archive
            .stdout
            .take()
            .ok_or("failed to capture git archive stdout".to_string())?;
        let tar_status = Command::new("tar")
            .arg("-x")
            .arg("-C")
            .arg(&path)
            .stdin(Stdio::from(archive_stdout))
            .status()
            .map_err(|err| format!("failed to run tar: {}", err))?;
        let archive_status = archive
            .wait()
            .map_err(|err| format!("failed to wait for git archive: {}", err))?;
        if !archive_status.success() {
            return Err("git archive failed".to_string());
        }
        if !tar_status.success() {
            return Err("failed to extract staged git snapshot".to_string());
        }
        Ok(StagedSnapshot { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagedSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))?
        .as_nanos();
    Ok(env::temp_dir().join(format!("{}-{}-{}", prefix, process::id(), nanos)))
}
