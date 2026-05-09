use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScopeHashSource {
    Index,
    Head,
}

type ScopeCacheKey = (PathBuf, ScopeHashSource, Vec<String>, Vec<String>);

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
        agent: &AgentConfig,
        scope: &[String],
        source: ScopeHashSource,
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope(scope, agent)?;
        let key = scope_cache_key(root, agent, &scope, source);
        if let Some(hash) = self.values.get(&key) {
            return Ok(hash.clone());
        }
        let hash = self
            .scope_entries_for_key(root, agent, &scope, source, &key)?
            .map(|entries| hash_120(entries.join("\n").as_bytes()));
        self.values.insert(key, hash.clone());
        Ok(hash)
    }

    fn scope_entries_for_key(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
        source: ScopeHashSource,
        key: &ScopeCacheKey,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(entries) = self.entries.get(key) {
            return Ok(entries.clone());
        }
        let entries = match source {
            ScopeHashSource::Index => staged_scope_entries(root, agent, scope).map(Some)?,
            ScopeHashSource::Head => self.head_scope_entries(root, agent, scope)?,
        };
        self.entries.insert(key.clone(), entries.clone());
        Ok(entries)
    }

    fn head_scope_entries(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        if !self.git_has_head(root)? {
            return Ok(None);
        }
        head_scope_entries_for_existing_head(root, agent, scope).map(Some)
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

pub(crate) fn scope_hash_agent_key(agent: &AgentConfig) -> Vec<String> {
    effective_ignore_patterns(agent)
}

fn scope_cache_key(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
    source: ScopeHashSource,
) -> ScopeCacheKey {
    (
        root.to_path_buf(),
        source,
        scope.to_vec(),
        scope_hash_agent_key(agent),
    )
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
    agent: &AgentConfig,
    scope: &[String],
    source: ScopeHashSource,
) -> Result<Option<String>, String> {
    let scope = sanitize_scope(scope, agent)?;
    scope_hash_for_canonical_scope(root, agent, &scope, source)
}

#[cfg(test)]
pub(crate) fn scope_hash_for_canonical_scope(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
    source: ScopeHashSource,
) -> Result<Option<String>, String> {
    let entries = match source {
        ScopeHashSource::Index => staged_scope_entries(root, agent, &scope).map(Some)?,
        ScopeHashSource::Head => head_scope_entries(root, agent, &scope)?,
    };
    Ok(entries.map(|entries| hash_120(entries.join("\n").as_bytes())))
}

pub(crate) fn staged_scope_entries(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<Vec<String>, String> {
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
            if !is_denied_path(agent, path) {
                entries.push(normalize_index_metadata(metadata, path)?);
            }
        }
    }
    entries.sort();
    entries.dedup();
    Ok(entries)
}

#[cfg(test)]
pub(crate) fn head_scope_entries(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !git_has_head(root)? {
        return Ok(None);
    }
    head_scope_entries_for_existing_head(root, agent, scope).map(Some)
}

pub(crate) fn head_scope_entries_for_existing_head(
    root: &Path,
    agent: &AgentConfig,
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
            if !is_denied_path(agent, path) {
                entries.push(normalize_head_metadata(metadata, path)?);
            }
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

pub(crate) fn sanitize_scope(scope: &[String], agent: &AgentConfig) -> Result<Vec<String>, String> {
    if scope.is_empty() {
        return Ok(full_scope());
    }
    let mut normalized = Vec::new();
    for path in scope {
        let path = normalize_repo_path(path)?;
        if path != "." && (path.contains('*') || path.contains('?')) {
            return Err(format!("scope paths must not be globs: {}", path));
        }
        if path != "." && is_denied_path(agent, &path) {
            return Err(format!("scope path is denied: {}", path));
        }
        if path == "." {
            return Ok(full_scope());
        }
        normalized.push(path);
    }
    if normalized.is_empty() {
        Ok(full_scope())
    } else {
        Ok(canonicalize_scope_paths(normalized))
    }
}

pub(crate) fn canonicalize_scope_paths(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    let mut canonical: Vec<String> = Vec::new();
    for path in paths {
        if canonical.iter().any(|parent| scope_contains(parent, &path)) {
            continue;
        }
        canonical.push(path);
    }
    if canonical.is_empty() {
        full_scope()
    } else {
        canonical
    }
}

pub(crate) fn normalize_repo_path(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("path must not be empty".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!("path must be relative: {}", value));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("path must be valid UTF-8: {}", value))?;
                parts.push(part.to_string());
            }
            std::path::Component::ParentDir => {
                return Err(format!("path must not contain '..': {}", value));
            }
            _ => return Err(format!("unsupported path component in {}", value)),
        }
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

pub(crate) fn is_denied_path(agent: &AgentConfig, path: &str) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern(path, pattern))
}

pub(crate) fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    let path = path.trim_start_matches("./");
    let pattern = pattern.trim_start_matches("./");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{}/", prefix));
    }
    path == pattern
}

pub(crate) fn is_strict_scope_subset(proposed: &[String], current: &[String]) -> bool {
    if proposed == current {
        return false;
    }
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

pub(crate) fn scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

pub(crate) fn format_log_record_timestamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts_from_unix_seconds(seconds);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

pub(crate) fn utc_parts_from_unix_seconds(seconds: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    (year, month, day, hour, minute, second)
}

pub(crate) fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let days = days_since_unix_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = MANDATORY_EVALUATOR_DENY_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    for pattern in &agent.ignore {
        if !patterns.iter().any(|existing| existing == pattern) {
            patterns.push(pattern.clone());
        }
    }
    patterns
}
pub(crate) const MANDATORY_EVALUATOR_DENY_PATTERNS: &[&str] =
    &[".canon", ".canon/**", ".git/canon", ".git/canon/**"];
