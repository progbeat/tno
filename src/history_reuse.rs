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
    let records = history_cache.read_records(root, expectation)?;
    for mut record in records.into_iter().rev() {
        if !is_reusable_history_record(&record) {
            continue;
        }
        let scope = match sanitize_scope_for_hash(&record.scope) {
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
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Option<CheckRecord>, String> {
    let Some(cooldown) = expectation.cooldown else {
        return Ok(None);
    };
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        let Some(timestamp) = parse_log_record_timestamp(&record.timestamp) else {
            continue;
        };
        // Cooldown keys off the latest valid history record, not the latest
        // reusable answer record. A newer fail or human-review-style record
        // blocks cooldown reuse of an older pass; callers can still continue
        // with exact-cache lookup or evaluator interrogation.
        if !record.passed() {
            return Ok(None);
        }
        if now.saturating_sub(timestamp) >= cooldown.seconds {
            return Ok(None);
        }
        // Cooldown is deliberately independent of scopeHash. It is the
        // spec-defined exception to exact cache lookup for expensive checks.
        return Ok(Some(record));
    }
    Ok(None)
}

pub(crate) fn latest_history_scope_with_cache(
    root: &Path,
    _agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<Option<Vec<String>>, String> {
    // This returns only an enforced-scope seed for a fresh interrogation. It is
    // not a cached check result and does not let callers skip evaluator work.
    let records = history_cache.read_records(root, expectation)?;
    for record in records.into_iter().rev() {
        if !is_reusable_history_record(&record) {
            continue;
        }
        if let Ok(scope) = sanitize_scope_for_hash(&record.scope) {
            return Ok(Some(scope));
        }
    }
    Ok(None)
}

pub(crate) fn is_reusable_history_record(record: &CheckRecord) -> bool {
    record.observed != OBSERVED_IDK
        && record.observed != OBSERVED_MALFORMED
        && record.observed != UNPARSEABLE_OBSERVED
}
