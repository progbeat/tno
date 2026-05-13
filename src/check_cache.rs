use crate::*;

pub(crate) struct CheckCacheHit {
    pub(crate) record: CheckRecord,
}

pub(crate) fn cached_record_for_expectation(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckCacheHit>, String> {
    reusable_history_record_for_source(
        root,
        agent,
        expectation,
        ScopeHashSource::Index,
        history_cache,
        scope_hash_cache,
    )
    .map(|record| record.map(|record| CheckCacheHit { record }))
}

pub(crate) fn cached_failure_for_expectation(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckCacheHit>, String> {
    Ok(
        cached_record_for_expectation(root, agent, expectation, history_cache, scope_hash_cache)?
            .filter(|hit| !hit.record.passed()),
    )
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    writer.write_event(
        "info",
        "cache.exact_hit",
        &[
            ("number", json!(hit.record.number)),
            ("result", json!(hit.record.result)),
            ("scope", json!(hit.record.scope)),
            ("scopeHash", json!(hit.record.scope_hash)),
        ],
    )?;
    writer.write_record(&hit.record)
}
