use crate::history_cache_key::history_cache_key;
use crate::scope_hash::ScopeHashCache;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::types::{AgentConfig, CheckRecord, CheckResult, SelectedExpectation};
use crate::UNPARSEABLE_OBSERVED;
use std::path::Path;

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
        timestamp: format_record_timestamp(unix_timestamp()?),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
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
