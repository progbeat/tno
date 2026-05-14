use crate::hash::{full_scope, hash_120};
use crate::project::command_output_trimmed;
use crate::scope::sanitize_scope_for_hash;
use crate::types::AgentConfig;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const HEX: &[u8; 16] = b"0123456789abcdef";

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
            .map(|entries| hash_scope_entries(&entries));
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
    Ok(entries.map(|entries| hash_scope_entries(&entries)))
}

pub(crate) fn hash_scope_entries(entries: &[String]) -> String {
    if entries
        .iter()
        .all(|entry| !entry.contains('\n') && !entry.contains('\0'))
    {
        return hash_120(entries.join("\n").as_bytes());
    }
    let mut input = Vec::new();
    input.extend_from_slice(b"scope-hash-v2\0");
    for entry in entries {
        input.extend_from_slice(entry.len().to_string().as_bytes());
        input.push(0);
        input.extend_from_slice(entry.as_bytes());
        input.push(0);
    }
    hash_120(&input)
}

pub(crate) fn staged_scope_entries(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("-s")
        .arg("-z")
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
    let mut entries = Vec::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if let Some((metadata, path)) = split_raw_scope_record(record, "git ls-files")? {
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
        .arg("-z")
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
    let mut entries = Vec::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if let Some((metadata, path)) = split_raw_scope_record(record, "git ls-tree")? {
            entries.push(normalize_head_metadata_bytes(metadata, path)?);
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

fn split_raw_scope_record<'a>(
    record: &'a [u8],
    command: &str,
) -> Result<Option<(&'a str, &'a [u8])>, String> {
    let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
        return Ok(None);
    };
    let metadata = std::str::from_utf8(&record[..tab])
        .map_err(|_| format!("{} metadata must be valid UTF-8", command))?;
    Ok(Some((metadata, &record[tab + 1..])))
}

pub(crate) fn normalize_index_metadata(metadata: &str, path: &[u8]) -> Result<String, String> {
    let path = scope_entry_path(path);
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

pub(crate) fn normalize_head_metadata_bytes(metadata: &str, path: &[u8]) -> Result<String, String> {
    let path = scope_entry_path(path);
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

fn scope_entry_path(path: &[u8]) -> String {
    match std::str::from_utf8(path) {
        Ok(path) => path.to_string(),
        Err(_) => {
            let mut output = String::from("\0raw-path-hex:");
            for byte in path {
                output.push(HEX[(byte >> 4) as usize] as char);
                output.push(HEX[(byte & 0x0f) as usize] as char);
            }
            output
        }
    }
}
