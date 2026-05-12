use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScopeHashSource {
    Index,
    Head,
}

type ScopeCacheKey = (PathBuf, ScopeHashSource, Vec<String>);

#[derive(Default)]
pub(crate) struct ScopeHashCache {
    values: BTreeMap<ScopeCacheKey, Option<String>>,
    entries: BTreeMap<ScopeCacheKey, Option<Vec<String>>>,
    head_exists: BTreeMap<PathBuf, bool>,
}

impl ScopeHashCache {
    pub(crate) fn new() -> ScopeHashCache {
        ScopeHashCache::default()
    }

    pub(crate) fn staged_scope_hash(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        self.scope_hash_for_source(root, agent, scope, ScopeHashSource::Index)?
            .ok_or("failed to hash staged scope".to_string())
    }

    pub(crate) fn scope_hash_for_source(
        &mut self,
        root: &Path,
        _agent: &AgentConfig,
        scope: &[String],
        source: ScopeHashSource,
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, &scope, source);
        if let Some(hash) = self.values.get(&key) {
            return Ok(hash.clone());
        }
        let hash = self
            .scope_entries_for_key(root, &scope, source, &key)?
            .map(|entries| hash_120(entries.join("\n").as_bytes()));
        self.values.insert(key, hash.clone());
        Ok(hash)
    }

    fn scope_entries_for_key(
        &mut self,
        root: &Path,
        scope: &[String],
        source: ScopeHashSource,
        key: &ScopeCacheKey,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(entries) = self.entries.get(key) {
            return Ok(entries.clone());
        }
        let entries = match source {
            ScopeHashSource::Index => staged_scope_entries(root, scope).map(Some)?,
            ScopeHashSource::Head => self.head_scope_entries(root, scope)?,
        };
        self.entries.insert(key.clone(), entries.clone());
        Ok(entries)
    }

    fn head_scope_entries(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        if !self.git_has_head(root)? {
            return Ok(None);
        }
        head_scope_entries_for_existing_head(root, scope).map(Some)
    }

    fn git_has_head(&mut self, root: &Path) -> Result<bool, String> {
        if let Some(has_head) = self.head_exists.get(root) {
            return Ok(*has_head);
        }
        let has_head = git_has_head(root)?;
        self.head_exists.insert(root.to_path_buf(), has_head);
        Ok(has_head)
    }
}

fn scope_cache_key(root: &Path, scope: &[String], source: ScopeHashSource) -> ScopeCacheKey {
    (root.to_path_buf(), source, scope.to_vec())
}

#[cfg(test)]
pub(crate) fn staged_scope_hash(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<String, String> {
    scope_hash_for_source(root, agent, scope, ScopeHashSource::Index)?
        .ok_or("failed to hash staged scope".to_string())
}

#[cfg(test)]
pub(crate) fn scope_hash_for_source(
    root: &Path,
    _agent: &AgentConfig,
    scope: &[String],
    source: ScopeHashSource,
) -> Result<Option<String>, String> {
    let scope = sanitize_scope_for_hash(scope)?;
    scope_hash_for_canonical_scope(root, &scope, source)
}

#[cfg(test)]
pub(crate) fn scope_hash_for_canonical_scope(
    root: &Path,
    scope: &[String],
    source: ScopeHashSource,
) -> Result<Option<String>, String> {
    let entries = match source {
        ScopeHashSource::Index => staged_scope_entries(root, scope).map(Some)?,
        ScopeHashSource::Head => head_scope_entries(root, scope)?,
    };
    Ok(entries.map(|entries| hash_120(entries.join("\n").as_bytes())))
}

pub(crate) fn staged_scope_entries(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("-s")
        .arg("--");
    if scope != full_scope() {
        for path in scope {
            command.arg(path);
        }
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git ls-files: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect staged scope: {}",
            command_output_trimmed(&output.stderr, "git ls-files stderr")?
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git ls-files output must be valid UTF-8".to_string())?;
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if let Some((metadata, path)) = line.split_once('\t') {
            entries.push(normalize_index_metadata(metadata, path)?);
        }
    }
    entries.sort();
    entries.dedup();
    Ok(entries)
}

#[cfg(test)]
pub(crate) fn head_scope_entries(
    root: &Path,
    scope: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !git_has_head(root)? {
        return Ok(None);
    }
    head_scope_entries_for_existing_head(root, scope).map(Some)
}

pub(crate) fn head_scope_entries_for_existing_head(
    root: &Path,
    scope: &[String],
) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("ls-tree")
        .arg("-r")
        .arg("HEAD")
        .arg("--");
    if scope != full_scope() {
        for path in scope {
            command.arg(path);
        }
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git ls-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect HEAD scope: {}",
            command_output_trimmed(&output.stderr, "git ls-tree stderr")?
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git ls-tree output must be valid UTF-8".to_string())?;
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if let Some((metadata, path)) = line.split_once('\t') {
            entries.push(normalize_head_metadata(metadata, path)?);
        }
    }
    entries.sort();
    entries.dedup();
    Ok(entries)
}

pub(crate) fn git_has_head(root: &Path) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "-q", "HEAD^{tree}"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    Ok(output.status.success())
}

pub(crate) fn normalize_index_metadata(metadata: &str, path: &str) -> Result<String, String> {
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| format!("malformed git index entry for {}", path))?;
    let object = fields
        .next()
        .ok_or_else(|| format!("malformed git index entry for {}", path))?;
    let stage = fields
        .next()
        .ok_or_else(|| format!("malformed git index entry for {}", path))?;
    if stage == "0" {
        Ok(format!("{} {}\t{}", mode, object, path))
    } else {
        Ok(format!("{} {} {}\t{}", mode, object, stage, path))
    }
}

pub(crate) fn normalize_head_metadata(metadata: &str, path: &str) -> Result<String, String> {
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| format!("malformed git tree entry for {}", path))?;
    let _kind = fields
        .next()
        .ok_or_else(|| format!("malformed git tree entry for {}", path))?;
    let object = fields
        .next()
        .ok_or_else(|| format!("malformed git tree entry for {}", path))?;
    Ok(format!("{} {}\t{}", mode, object, path))
}
