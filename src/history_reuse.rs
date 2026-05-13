use crate::*;

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
    scan_latest_history_records(root, expectation, history_cache, |mut record| {
        let Some(scope) = sanitized_reusable_history_scope(&record) else {
            return Ok(HistoryRecordScan::Continue);
        };
        let Some(current_hash) =
            scope_hash_cache.scope_hash_for_source(root, agent, &scope, source)?
        else {
            return Ok(HistoryRecordScan::Continue);
        };
        if current_hash == record.scope_hash {
            record.id = expectation.id.clone();
            record.display_id = expectation.display_id.clone();
            record.scope = scope;
            record.number = expectation.number;
            record.prompt = expectation.q.clone();
            record.expected = expectation.a.clone();
            return Ok(HistoryRecordScan::Done(Some(record)));
        }
        Ok(HistoryRecordScan::Continue)
    })
}

pub(crate) fn cooldown_history_record(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Option<CheckRecord>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    scan_latest_history_records(root, expectation, history_cache, |record| {
        let Some(timestamp) = parse_log_record_timestamp(&record.timestamp) else {
            return Ok(HistoryRecordScan::Continue);
        };
        // Cooldown keys off the latest valid history record, not the latest
        // reusable answer record. This is why a newer fail or human-review-style
        // record blocks cooldown reuse of an older pass: after a failed check,
        // the old pass is no longer the spec-defined fresh cooldown pass.
        if !record.passed() {
            return Ok(HistoryRecordScan::Done(None));
        }
        if timestamp > now {
            return Ok(HistoryRecordScan::Done(None));
        }
        if now - timestamp >= cooldown.seconds {
            return Ok(HistoryRecordScan::Done(None));
        }
        // Cooldown is deliberately independent of scopeHash. It is the
        // spec-defined exception to exact cache lookup for expensive checks.
        Ok(HistoryRecordScan::Done(Some(record)))
    })
}

pub(crate) fn latest_history_scope_with_cache(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    // This returns only an enforced-scope seed for a fresh interrogation. It is
    // not a cached check result and does not let callers skip evaluator work.
    scan_latest_history_records(root, expectation, history_cache, |record| {
        Ok(match sanitized_reusable_history_scope(&record) {
            Some(scope) => HistoryRecordScan::Done(Some(scope)),
            None => HistoryRecordScan::Continue,
        })
    })
}

enum HistoryRecordScan<T> {
    Continue,
    Done(Option<T>),
}

fn scan_latest_history_records<T>(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    mut scan: impl FnMut(CheckRecord) -> Result<HistoryRecordScan<T>, String>,
) -> Result<Option<T>, String> {
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        match scan(record)? {
            HistoryRecordScan::Continue => {}
            HistoryRecordScan::Done(value) => return Ok(value),
        }
    }
    Ok(None)
}

fn sanitized_reusable_history_scope(record: &CheckRecord) -> Option<Vec<String>> {
    if !is_reusable_history_record(record) {
        return None;
    }
    sanitize_scope_for_hash(&record.scope).ok()
}

pub(crate) fn is_reusable_history_record(record: &CheckRecord) -> bool {
    record.observed != OBSERVED_IDK
        && record.observed != OBSERVED_MALFORMED
        && record.observed != UNPARSEABLE_OBSERVED
        && record.observed != EMPTY_EVIDENCE_OBSERVED
}
