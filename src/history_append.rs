use crate::*;

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
