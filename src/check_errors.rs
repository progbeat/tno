use crate::*;

pub(crate) fn error_record_from_interrogation_error(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    scope: &[String],
    error: &str,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<CheckRecord, String> {
    let scope_hash = scope_hash_cache.staged_scope_hash(root, agent, scope)?;
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
        number: expectation.number,
        result: CheckResult::Fail,
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: UNPARSEABLE_OBSERVED.to_string(),
        evidence: error.to_string(),
        scope: scope.to_vec(),
        scope_hash,
        cache_key: Some(history_cache_key(agent, expectation)),
    })
}
