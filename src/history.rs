use crate::*;

pub(crate) fn history_path(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<PathBuf, String> {
    Ok(git_path(root, GIT_CANON_CACHE_DIR)?
        .join(&expectation.id)
        .join(history_file_name()))
}

pub(crate) fn history_file_name() -> &'static str {
    "history.jsonl"
}

pub(crate) fn active_expectation_ids(config: &CheckConfig) -> BTreeSet<String> {
    config
        .expectations
        .iter()
        .map(|expectation| expectation_id(&expectation.q, &expectation.a))
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CacheCleanupStats {
    pub(crate) sampled: bool,
    pub(crate) removed: usize,
    pub(crate) kept: usize,
}

pub(crate) fn maybe_cleanup_stale_cache_dirs(
    root: &Path,
    config: &CheckConfig,
) -> Result<CacheCleanupStats, String> {
    if !sample_approximately_one_in(CACHE_CLEANUP_SAMPLE_INTERVAL, "cache-cleanup", root)? {
        return Ok(CacheCleanupStats::default());
    }
    let mut stats = cleanup_stale_cache_dirs(root, &active_expectation_ids(config))?;
    stats.sampled = true;
    Ok(stats)
}

pub(crate) fn cleanup_stale_cache_dirs(
    root: &Path,
    active_ids: &BTreeSet<String>,
) -> Result<CacheCleanupStats, String> {
    let cache_dir = git_path(root, GIT_CANON_CACHE_DIR)?;
    if !cache_dir.exists() {
        return Ok(CacheCleanupStats {
            sampled: true,
            removed: 0,
            kept: 0,
        });
    }
    let mut stats = CacheCleanupStats {
        sampled: true,
        removed: 0,
        kept: 0,
    };
    for entry in fs::read_dir(&cache_dir)
        .map_err(|err| format!("failed to read {}: {}", cache_dir.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", cache_dir.display(), err))?;
        let file_name = entry.file_name();
        let Some(id) = file_name.to_str() else {
            remove_cache_entry(&entry.path())?;
            stats.removed += 1;
            continue;
        };
        if active_ids.contains(id) {
            stats.kept += 1;
        } else {
            remove_cache_entry(&entry.path())?;
            stats.removed += 1;
        }
    }
    Ok(stats)
}

pub(crate) fn remove_cache_entry(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    } else {
        fs::remove_file(path).map_err(|err| format!("failed to remove {}: {}", path.display(), err))
    }
}

#[cfg(test)]
pub(crate) fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    read_history_records_from_path(&path)
}

pub(crate) fn read_history_records_from_path(path: &Path) -> Result<Vec<CheckRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<CheckRecord>(&line).map_err(|err| {
            format!(
                "invalid history JSON in {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

#[derive(Default)]
pub(crate) struct HistoryCache {
    paths: BTreeMap<(PathBuf, String), PathBuf>,
    records: BTreeMap<PathBuf, Vec<CheckRecord>>,
}

impl HistoryCache {
    pub(crate) fn new() -> HistoryCache {
        HistoryCache::default()
    }

    pub(crate) fn read_records(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<Vec<CheckRecord>, String> {
        let path = self.path(root, expectation)?;
        if let Some(records) = self.records.get(&path) {
            return Ok(records.clone());
        }
        let records = read_history_records_from_path(&path)?;
        self.records.insert(path, records.clone());
        Ok(records)
    }

    pub(crate) fn path(
        &mut self,
        root: &Path,
        expectation: &SelectedExpectation,
    ) -> Result<PathBuf, String> {
        let key = (root.to_path_buf(), expectation.id.clone());
        if let Some(path) = self.paths.get(&key) {
            return Ok(path.clone());
        }
        let path = history_path(root, expectation)?;
        self.paths.insert(key, path.clone());
        Ok(path)
    }
}

#[cfg(test)]
pub(crate) fn reusable_history_record(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<CheckRecord>, String> {
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    reusable_history_record_for_source(
        root,
        agent,
        expectation,
        ScopeHashSource::Index,
        &mut history_cache,
        &mut scope_hash_cache,
    )
}

pub(crate) fn reusable_history_record_for_source(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    source: ScopeHashSource,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckRecord>, String> {
    let records = history_cache.read_records(root, expectation)?;
    for mut record in records.into_iter().rev() {
        if !is_reusable_history_record(&record) {
            continue;
        }
        let scope = match sanitize_scope(&record.scope, agent) {
            Ok(scope) => scope,
            Err(_) => continue,
        };
        let Some(current_hash) =
            scope_hash_cache.scope_hash_for_source(root, agent, &scope, source)?
        else {
            continue;
        };
        if current_hash == record.scope_hash {
            record.scope = scope;
            record.number = expectation.number;
            record.prompt = expectation.q.clone();
            record.expected = expectation.a.clone();
            return Ok(Some(record));
        }
    }
    Ok(None)
}

pub(crate) fn cooldown_history_record(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Option<CheckRecord>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        if !is_reusable_history_record(&record) {
            continue;
        }
        let Some(timestamp) = parse_log_record_timestamp(&record.timestamp) else {
            continue;
        };
        // A recent failure is not a cooldown hit. Callers must continue with
        // exact-cache lookup, which can still reuse this fail when scopeHash
        // matches, or otherwise interrogate from the latest reusable scope.
        if !record.passed() {
            return Ok(None);
        }
        if now.saturating_sub(timestamp) >= cooldown.seconds {
            return Ok(None);
        }
        return Ok(Some(record));
    }
    Ok(None)
}

pub(crate) fn latest_history_scope_with_cache(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        if !is_reusable_history_record(&record) {
            continue;
        }
        if let Ok(scope) = sanitize_scope(&record.scope, agent) {
            return Ok(Some(scope));
        }
    }
    Ok(None)
}

pub(crate) fn is_reusable_history_record(record: &CheckRecord) -> bool {
    matches!(record.result.as_str(), RESULT_PASS | RESULT_FAIL)
        && record.observed != OBSERVED_IDK
        && record.observed != OBSERVED_MALFORMED
        && record.observed != UNPARSEABLE_OBSERVED
}

#[cfg(test)]
pub(crate) fn append_history_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    let mut cache = HistoryCache::new();
    append_history_record_with_cache(root, expectation, record, &mut cache)
}

pub(crate) fn append_history_record_with_cache(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    let path = history_cache.path(root, expectation)?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let line = render_check_log_record(record);
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))?;
    let had_cached_records = history_cache.records.contains_key(&path);
    let compacted = should_compact_history(&path)?;
    if compacted {
        compact_history(&path)?;
    }
    if had_cached_records {
        if compacted {
            let records = read_history_records_from_path(&path)?;
            history_cache.records.insert(path, records);
        } else if let Some(records) = history_cache.records.get_mut(&path) {
            records.push(record.clone());
        }
    }
    Ok(())
}

pub(crate) fn should_compact_history(path: &Path) -> Result<bool, String> {
    sample_approximately_one_in(HISTORY_COMPACT_SAMPLE_INTERVAL, "history-compact", path)
}

pub(crate) fn sample_approximately_one_in(
    interval: u64,
    label: &str,
    path: &Path,
) -> Result<bool, String> {
    if interval == 0 {
        return Err("sample interval must be greater than zero".to_string());
    }
    let counter = COMPACTION_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system clock is before UNIX_EPOCH: {}", err))?
        .as_nanos();
    let seed = format!(
        "{}\0{}\0{}\0{}\0{}",
        label,
        process::id(),
        counter,
        now,
        path.display()
    );
    Ok(
        fnv64_with_seed(FNV_OFFSET ^ 0xa24b_aed4_963e_e407, seed.as_bytes())
            .is_multiple_of(interval),
    )
}

pub(crate) fn compact_history(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let mut total_lines = 0usize;
    let mut lines = std::collections::VecDeque::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        validate_json_object_line(&line).map_err(|err| {
            format!(
                "invalid history JSON in {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        total_lines += 1;
        lines.push_back(line);
        if lines.len() > HISTORY_COMPACT_KEEP_RECORDS {
            lines.pop_front();
        }
    }
    if total_lines <= HISTORY_COMPACT_KEEP_RECORDS {
        return Ok(());
    }
    let temp_path = compact_history_temp_path(path)?;
    let mut file = fs::File::create(&temp_path)
        .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    for line in lines {
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        file.write_all(b"\n")
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    }
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", temp_path.display(), err))?;
    drop(file);
    replace_history_file(&temp_path, path)
}

pub(crate) fn replace_history_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path).map_err(|err| {
        format!(
            "failed to replace {} with {}: {}",
            path.display(),
            temp_path.display(),
            err
        )
    })
}

pub(crate) fn validate_json_object_line(line: &str) -> Result<(), String> {
    match serde_json::from_str::<Value>(line) {
        Ok(value) if value.is_object() => Ok(()),
        Ok(_) => Err("history line must be a JSON object".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

pub(crate) fn compact_history_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("history path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(".tmp");
    Ok(path.with_file_name(temp_name))
}
