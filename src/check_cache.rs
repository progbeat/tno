use crate::*;

pub(crate) enum CheckCacheHitKind {
    Cooldown,
    Exact,
}

pub(crate) struct CheckCacheHit {
    pub(crate) kind: CheckCacheHitKind,
    pub(crate) record: CheckRecord,
}

pub(crate) fn cached_record_for_expectation(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<Option<CheckCacheHit>, String> {
    // A "fresh cooldown pass" means the cooldown spec's latest valid history
    // record is a pass within its cooldown window. Older passes are not
    // cooldown hits after a newer fail; they can only participate in the exact
    // scopeHash lookup below.
    if let Some(record) =
        cooldown_history_record(root, agent, expectation, history_cache, unix_timestamp()?)?
    {
        return Ok(Some(CheckCacheHit {
            kind: CheckCacheHitKind::Cooldown,
            record,
        }));
    }
    // Cooldown is intentionally pass-only. A latest cached fail returns
    // no cooldown hit above, then falls through to this exact-cache
    // lookup where reusable pass and fail records are both valid hits.
    reusable_history_record_for_source(
        root,
        agent,
        expectation,
        ScopeHashSource::Index,
        history_cache,
        scope_hash_cache,
    )
    .map(|record| {
        record.map(|record| CheckCacheHit {
            kind: CheckCacheHitKind::Exact,
            record,
        })
    })
}

pub(crate) fn write_cache_hit(
    writer: &mut DiagnosticLogWriter,
    hit: &CheckCacheHit,
) -> Result<(), String> {
    match hit.kind {
        CheckCacheHitKind::Cooldown => writer.write_event(
            "info",
            "cache.cooldown_hit",
            &[
                ("number", json!(hit.record.number)),
                ("scope", json!(hit.record.scope)),
                ("scopeHash", json!(hit.record.scope_hash)),
            ],
        )?,
        CheckCacheHitKind::Exact => writer.write_event(
            "info",
            "cache.exact_hit",
            &[
                ("number", json!(hit.record.number)),
                ("result", json!(hit.record.result)),
                ("scope", json!(hit.record.scope)),
                ("scopeHash", json!(hit.record.scope_hash)),
            ],
        )?,
    }
    writer.write_record(&hit.record)
}
