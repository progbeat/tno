use crate::config_types::AgentConfig;
use crate::git::{git_head_tree_exists, resolve_git_path};
use crate::hash::{full_scope, hash_120};
use crate::project::command_output_trimmed;
use crate::scope::sanitize_scope_for_hash;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const HEX: &[u8; 16] = b"0123456789abcdef";

type ScopeCacheKey = (PathBuf, Vec<String>);

#[derive(Default)]
pub(crate) struct ScopeHashCache {
    values: BTreeMap<ScopeCacheKey, Option<String>>,
    entries: BTreeMap<ScopeCacheKey, Option<Vec<String>>>,
    gate_head_values: BTreeMap<ScopeCacheKey, Option<String>>,
    gate_head_entries: BTreeMap<ScopeCacheKey, Option<Vec<String>>>,
    head_exists: BTreeMap<PathBuf, bool>,
    // Staged scope hashes intentionally use only tracked staged Git entries.
    // Local Git files are still cached here because staged snapshot creation
    // copies hook metadata, but they are not part of scopeHash cache keys.
    local_git_files: BTreeMap<LocalGitFileCacheKey, Result<Option<LocalGitFileSnapshot>, String>>,
}

type LocalGitFileCacheKey = (PathBuf, String);

#[derive(Clone)]
pub(crate) struct LocalGitFileSnapshot {
    pub(crate) content: Vec<u8>,
    pub(crate) permissions: fs::Permissions,
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
        self.staged_scope_hash_option(root, agent, scope)?
            .ok_or("failed to hash staged scope".to_string())
    }

    fn staged_scope_hash_option(
        &mut self,
        root: &Path,
        _agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, &scope);
        if let Some(hash) = self.values.get(&key) {
            return Ok(hash.clone());
        }
        let hash = self
            .staged_scope_entries_for_key(root, &scope, &key)?
            .map(|entries| hash_scope_entries(&entries));
        self.values.insert(key, hash.clone());
        Ok(hash)
    }

    fn staged_scope_entries_for_key(
        &mut self,
        root: &Path,
        scope: &[String],
        key: &ScopeCacheKey,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(entries) = self.entries.get(key) {
            return Ok(entries.clone());
        }
        let entries = staged_scope_entries_for_scope(root, scope).map(Some)?;
        self.entries.insert(key.clone(), entries.clone());
        Ok(entries)
    }

    #[cfg(test)]
    fn staged_scope_entries(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        staged_scope_entries_for_scope(root, scope)
    }

    pub(crate) fn gate_head_tree_fingerprint(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, &scope);
        if let Some(hash) = self.gate_head_values.get(&key) {
            return Ok(hash.clone());
        }
        // Gate compares staged cache records with the committed HEAD tree to
        // tell whether a cached failure is a new regression. This value is not
        // written to answer history as a `scopeHash`; answer-history scopeHash
        // records are produced only by `staged_scope_hash`.
        let hash = self
            .gate_head_tree_entries_for_key(root, &scope, &key)?
            .map(|entries| hash_scope_entries(&entries));
        self.gate_head_values.insert(key, hash.clone());
        Ok(hash)
    }

    fn gate_head_tree_entries_for_key(
        &mut self,
        root: &Path,
        scope: &[String],
        key: &ScopeCacheKey,
    ) -> Result<Option<Vec<String>>, String> {
        if let Some(entries) = self.gate_head_entries.get(key) {
            return Ok(entries.clone());
        }
        if !self.git_has_head(root)? {
            self.gate_head_entries.insert(key.clone(), None);
            return Ok(None);
        }
        let entries = head_scope_entries_for_existing_head(root, scope).map(Some)?;
        self.gate_head_entries.insert(key.clone(), entries.clone());
        Ok(entries)
    }

    pub(crate) fn git_has_head(&mut self, root: &Path) -> Result<bool, String> {
        if let Some(has_head) = self.head_exists.get(root) {
            return Ok(*has_head);
        }
        let has_head = git_head_tree_exists(root)?;
        self.head_exists.insert(root.to_path_buf(), has_head);
        Ok(has_head)
    }

    pub(crate) fn local_git_file_snapshot(
        &mut self,
        root: &Path,
        git_path: &str,
    ) -> Result<Option<LocalGitFileSnapshot>, String> {
        let key = (root.to_path_buf(), git_path.to_string());
        if let Some(cached) = self.local_git_files.get(&key) {
            return cached.clone();
        }
        let path = resolve_git_path(root, git_path)?;
        let snapshot = match fs::read(&path) {
            Ok(content) => {
                let metadata = fs::metadata(&path)
                    .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
                Ok(Some(LocalGitFileSnapshot {
                    content,
                    permissions: metadata.permissions(),
                }))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("failed to read {}: {}", path.display(), err)),
        };
        self.local_git_files.insert(key, snapshot.clone());
        snapshot
    }
}

fn scope_cache_key(root: &Path, scope: &[String]) -> ScopeCacheKey {
    (root.to_path_buf(), scope.to_vec())
}

#[cfg(test)]
pub(crate) fn staged_scope_hash(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<String, String> {
    ScopeHashCache::new().staged_scope_hash(root, agent, scope)
}

#[cfg(test)]
pub(crate) fn gate_head_tree_fingerprint(
    root: &Path,
    scope: &[String],
) -> Result<Option<String>, String> {
    let scope = sanitize_scope_for_hash(scope)?;
    head_scope_entries(root, &scope)
        .map(|entries| entries.map(|entries| hash_scope_entries(&entries)))
}

pub(crate) fn hash_scope_entries(entries: &[String]) -> String {
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

#[cfg(test)]
pub(crate) fn staged_scope_entries(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    ScopeHashCache::new().staged_scope_entries(root, scope)
}

#[cfg(test)]
pub(crate) fn head_scope_entries(
    root: &Path,
    scope: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !git_head_tree_exists(root)? {
        return Ok(None);
    }
    head_scope_entries_for_existing_head(root, scope).map(Some)
}

pub(crate) fn head_scope_entries_for_existing_head(
    root: &Path,
    scope: &[String],
) -> Result<Vec<String>, String> {
    git_scope_entries(root, scope, GitScopeListing::Head)
}

fn staged_scope_entries_for_scope(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    git_scope_entries(root, scope, GitScopeListing::Index)
}

#[derive(Clone, Copy)]
enum GitScopeListing {
    Index,
    Head,
}

fn git_scope_entries(
    root: &Path,
    scope: &[String],
    listing: GitScopeListing,
) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).arg("--literal-pathspecs");
    match listing {
        GitScopeListing::Index => {
            command.args(["ls-files", "-s", "-z", "--"]);
        }
        GitScopeListing::Head => {
            command.args(["ls-tree", "-z", "-r", "HEAD", "--"]);
        }
    }
    if scope != full_scope() {
        for path in scope {
            command.arg(path);
        }
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run {}: {}", listing.command_name(), err))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            listing.inspect_error(),
            command_output_trimmed(&output.stderr, listing.stderr_label())?
        ));
    }
    let mut entries = Vec::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if let Some((metadata, path)) = split_raw_scope_record(record, listing.command_name())? {
            entries.push(normalize_git_scope_metadata(metadata, path, listing)?);
        }
    }
    sort_scope_entries(&mut entries);
    Ok(entries)
}

impl GitScopeListing {
    fn command_name(self) -> &'static str {
        match self {
            GitScopeListing::Index => "git ls-files",
            GitScopeListing::Head => "git ls-tree",
        }
    }

    fn stderr_label(self) -> &'static str {
        match self {
            GitScopeListing::Index => "git ls-files stderr",
            GitScopeListing::Head => "git ls-tree stderr",
        }
    }

    fn inspect_error(self) -> &'static str {
        match self {
            GitScopeListing::Index => "failed to inspect staged scope",
            GitScopeListing::Head => "failed to inspect HEAD scope",
        }
    }

    fn malformed_entry(self) -> &'static str {
        match self {
            GitScopeListing::Index => "git index entry",
            GitScopeListing::Head => "git tree entry",
        }
    }
}

fn sort_scope_entries(entries: &mut Vec<String>) {
    entries.sort();
    entries.dedup();
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

#[cfg(test)]
pub(crate) fn normalize_index_metadata(metadata: &str, path: &[u8]) -> Result<String, String> {
    normalize_git_scope_metadata(metadata, path, GitScopeListing::Index)
}

fn normalize_git_scope_metadata(
    metadata: &str,
    path: &[u8],
    listing: GitScopeListing,
) -> Result<String, String> {
    let path = scope_entry_path(path);
    let mut fields = metadata.split_whitespace();
    let mode = next_scope_metadata_field(&mut fields, listing, &path)?;
    match listing {
        GitScopeListing::Index => {
            let object = next_scope_metadata_field(&mut fields, listing, &path)?;
            let stage = next_scope_metadata_field(&mut fields, listing, &path)?;
            if stage == "0" {
                Ok(format!("{} {}\t{}", mode, object, path))
            } else {
                Ok(format!("{} {} {}\t{}", mode, object, stage, path))
            }
        }
        GitScopeListing::Head => {
            let _kind = next_scope_metadata_field(&mut fields, listing, &path)?;
            let object = next_scope_metadata_field(&mut fields, listing, &path)?;
            Ok(format!("{} {}\t{}", mode, object, path))
        }
    }
}

fn next_scope_metadata_field<'a>(
    fields: &mut std::str::SplitWhitespace<'a>,
    listing: GitScopeListing,
    path: &str,
) -> Result<&'a str, String> {
    fields
        .next()
        .ok_or_else(|| format!("malformed {} for {}", listing.malformed_entry(), path))
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
