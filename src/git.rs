use crate::*;

#[derive(Default)]
pub(crate) struct RepoInspectionCache {
    git_paths: BTreeMap<(PathBuf, String), Result<PathBuf, String>>,
    staged_file_contents: BTreeMap<(PathBuf, String), Result<String, String>>,
    filesystem_text: BTreeMap<PathBuf, Result<String, String>>,
    check_configs: BTreeMap<(PathBuf, PathBuf, String), Result<CheckConfig, String>>,
}

impl RepoInspectionCache {
    pub(crate) fn new() -> RepoInspectionCache {
        RepoInspectionCache::default()
    }

    pub(crate) fn git_path(&mut self, root: &Path, path: &str) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), path.to_string());
        if let Some(cached) = self.git_paths.get(&key) {
            return cached.clone();
        }
        let resolved = resolve_git_path(root, path);
        self.git_paths.insert(key, resolved.clone());
        resolved
    }

    pub(crate) fn staged_file_content(
        &mut self,
        root: &Path,
        path: &str,
    ) -> Result<String, String> {
        let key = (root.to_path_buf(), path.to_string());
        if let Some(cached) = self.staged_file_contents.get(&key) {
            return cached.clone();
        }
        let content = read_staged_file_content(root, path);
        self.staged_file_contents.insert(key, content.clone());
        content
    }

    pub(crate) fn read_to_string(&mut self, path: &Path) -> Result<String, String> {
        let key = path.to_path_buf();
        if let Some(cached) = self.filesystem_text.get(&key) {
            return cached.clone();
        }
        let content = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err));
        self.filesystem_text.insert(key, content.clone());
        content
    }

    pub(crate) fn load_check_config(
        &mut self,
        root: &Path,
        config_path: &Path,
    ) -> Result<CheckConfig, String> {
        let content = if config_path == Path::new(CHECK_PATH) {
            self.staged_file_content(root, CHECK_PATH).or_else(|_| {
                let path = root.join(CHECK_PATH);
                self.read_to_string(&path)
            })
        } else {
            let path = root.join(config_path);
            self.read_to_string(&path)
        }?;
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            content.clone(),
        );
        if let Some(cached) = self.check_configs.get(&key) {
            return cached.clone();
        }
        let parsed = parse_check_config_content(config_path, &content);
        self.check_configs.insert(key, parsed.clone());
        parsed
    }
}

pub(crate) fn git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    resolve_git_path(root, path)
}

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

pub(crate) struct StagedWorktreeView {
    root: PathBuf,
    stash_ref: Option<String>,
}

impl StagedWorktreeView {
    pub(crate) fn apply(root: &Path) -> Result<StagedWorktreeView, String> {
        if !has_unstaged_or_untracked_changes(root)? {
            return Ok(StagedWorktreeView {
                root: root.to_path_buf(),
                stash_ref: None,
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
        let stash_ref = if after.is_some() && after != before {
            Some("stash@{0}".to_string())
        } else {
            None
        };
        Ok(StagedWorktreeView {
            root: root.to_path_buf(),
            stash_ref,
        })
    }
}

impl Drop for StagedWorktreeView {
    fn drop(&mut self) {
        let Some(stash_ref) = self.stash_ref.as_deref() else {
            return;
        };
        match restore_staged_worktree_view(&self.root, stash_ref) {
            Ok(()) => {
                let _ = Command::new("git")
                    .arg("-C")
                    .arg(&self.root)
                    .args(["stash", "drop", "--quiet", stash_ref])
                    .status();
            }
            Err(err) => {
                eprintln!(
                    "canon: failed to restore worktree changes preserved in {}: {}",
                    stash_ref, err
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
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "untracked stash path must be valid UTF-8".to_string())
        })
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
    let stderr = command_output_utf8(&output.stderr, "git rev-parse stderr")?;
    if stderr.trim().is_empty() || stderr.contains("Needed a single revision") {
        Ok(false)
    } else {
        Err(format!(
            "failed to inspect git revision {}: {}",
            revision,
            stderr.trim()
        ))
    }
}

pub(crate) fn has_unstaged_or_untracked_changes(root: &Path) -> Result<bool, String> {
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

pub(crate) fn current_stash_oid(root: &Path) -> Result<Option<String>, String> {
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
