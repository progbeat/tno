use crate::*;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    // CLI validation happens before the gate pass/fail decision. These
    // unsupported-option errors are usage errors, not `GateFailed` outcomes.
    if args.iter().any(|arg| arg.to_str() == Some("--fail-fast")) {
        return Err("canon gate does not accept --fail-fast".into());
    }
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".into());
    }
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH))?;
    let changed_paths = staged_changed_paths(root)?;
    let has_canon_change = changed_paths.iter().any(|path| is_canon_project_path(path));
    if has_canon_change && !is_canon_only_staged_change(&changed_paths) {
        eprintln!("canon gate: .canon/** changes must not be mixed with non-.canon changes");
        return Err(CommandError::GateFailed);
    }
    if has_canon_change {
        return Ok(());
    }

    let mut scope_hash_cache = ScopeHashCache::new();
    // From this point on, `canon gate` is a cache-only decision. The only gate
    // failures after command/config/staged-change preflight are missing cache
    // records and new cached failures that were not already failing at HEAD.
    let mut history_cache = HistoryCache::new();
    let now = unix_timestamp()?;
    let selected_expectations =
        select_expectations_for_gate(root, &config, args, &mut history_cache, now)?;
    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected_expectations {
        let previous = exact_gate_cache_result_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Head,
            &mut history_cache,
            &mut scope_hash_cache,
        )?;
        let current = exact_gate_cache_result_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Index,
            &mut history_cache,
            &mut scope_hash_cache,
        )?;

        match current {
            GateCacheResult::Pass => {}
            GateCacheResult::Fail(_) if previous.is_fail() => {}
            GateCacheResult::Fail(record) => failing.push(*record),
            GateCacheResult::Missing => missing.push(expectation.number),
        }
    }

    if missing.is_empty() && failing.is_empty() {
        return Ok(());
    }
    if !failing.is_empty() {
        eprintln!("canon gate: expectations regressed to cached fail:");
        for record in &failing {
            eprint!("{}", render_check_log_record(record));
        }
    }
    if !missing.is_empty() {
        eprintln!(
            "canon gate: missing cached answers for expectations: {}",
            join_numbers(&missing)
        );
        if let Some(advice) = gate_missing_cache_advice(!failing.is_empty()) {
            eprintln!("{advice}");
        }
    }
    Err(CommandError::GateFailed)
}

pub(crate) fn gate_missing_cache_advice(has_regressions: bool) -> Option<&'static str> {
    // Regressions are the blocking action. When regressions and missing cache
    // records coexist, do not spend tokens filling unrelated missing records.
    if has_regressions {
        Some("canon gate: fix staged regressions before filling missing cache")
    } else {
        Some("canon gate: run `canon check` before committing")
    }
}

pub(crate) fn select_expectations_for_gate(
    root: &Path,
    config: &CheckConfig,
    args: &[OsString],
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<Vec<SelectedExpectation>, String> {
    // This constructs the spec-level `selected_expectations` argument passed to
    // gate(...). This is the cooldown spec's deselection rule: "When cooldown
    // applies, the expectation is removed from the selected expectation set."
    let mut remaining = Vec::new();
    for expectation in select_expectations(config, args)? {
        if cooldown_history_record(root, &config.agent, &expectation, history_cache, now)?.is_none()
        {
            remaining.push(expectation);
        }
    }
    Ok(remaining)
}

#[derive(Debug, Clone)]
pub(crate) enum GateCacheResult {
    Pass,
    Fail(Box<CheckRecord>),
    Missing,
}

impl GateCacheResult {
    pub(crate) fn is_fail(&self) -> bool {
        matches!(self, GateCacheResult::Fail(_))
    }
}

pub(crate) fn exact_gate_cache_result_for_source(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    source: ScopeHashSource,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<GateCacheResult, String> {
    match reusable_history_record_for_source(
        root,
        agent,
        expectation,
        source,
        history_cache,
        scope_hash_cache,
    )? {
        Some(record) if record.passed() => Ok(GateCacheResult::Pass),
        Some(record) => Ok(GateCacheResult::Fail(Box::new(record))),
        None => Ok(GateCacheResult::Missing),
    }
}
