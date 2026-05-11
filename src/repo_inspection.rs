use crate::*;

type GitPathCacheKey = (PathBuf, String);
type GeneratorPathsCacheKey = (PathBuf, PathBuf, String, bool);
type StagedFileContentCacheKey = (PathBuf, String);
type CheckConfigCacheKey = (PathBuf, PathBuf, String);

#[derive(Default)]
pub(crate) struct RepoInspectionCache {
    git_paths: BTreeMap<GitPathCacheKey, Result<PathBuf, String>>,
    generator_paths: BTreeMap<GeneratorPathsCacheKey, Result<Vec<String>, String>>,
    staged_file_contents: BTreeMap<StagedFileContentCacheKey, Result<String, String>>,
    filesystem_text: BTreeMap<PathBuf, Result<String, String>>,
    check_configs: BTreeMap<CheckConfigCacheKey, Result<CheckConfig, String>>,
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

    pub(crate) fn generator_paths(
        &mut self,
        root: &Path,
        config_path: &Path,
        path: &str,
        staged: bool,
    ) -> Result<Vec<String>, String> {
        let key = (
            root.to_path_buf(),
            config_path.to_path_buf(),
            path.to_string(),
            staged,
        );
        if let Some(cached) = self.generator_paths.get(&key) {
            return cached.clone();
        }
        let expanded = expand_generator_paths(root, config_path, path, staged);
        self.generator_paths.insert(key, expanded.clone());
        expanded
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
        let parsed = parse_check_config_content_with_root(root, config_path, &content, self);
        self.check_configs.insert(key, parsed.clone());
        parsed
    }
}
