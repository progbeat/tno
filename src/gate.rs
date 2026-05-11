use crate::*;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
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
    let selected = select_expectations(&config, args)?;
    let changed_paths = staged_changed_paths(root)?;
    fail_on_mixed_canon_paths(&changed_paths)?;

    let mut scope_hash_cache = ScopeHashCache::new();
    if should_skip_gate_for_canon_only_unchanged_visible_content(
        root,
        &config.agent,
        &changed_paths,
        &mut scope_hash_cache,
    )? {
        return Ok(());
    }

    // From this point on, `canon gate` is a cache-only decision. The only gate
    // failures after command/config/staged-change preflight are missing cache
    // records and new cached failures that were not already failing at HEAD.
    let mut history_cache = HistoryCache::new();
    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected {
        // Gate is normally exact-cache only, but `cooldown` is the explicit
        // spec exception: a fresh pass can satisfy the gate without comparing
        // the record's scopeHash to the current staged tree.
        if let Some(record) = cooldown_history_record(
            root,
            &config.agent,
            expectation,
            &mut history_cache,
            unix_timestamp()?,
        )? {
            if record.passed() {
                continue;
            }
        }
        match reusable_history_record_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Index,
            &mut history_cache,
            &mut scope_hash_cache,
        )? {
            Some(record) if record.passed() => {}
            Some(record)
                if !record.passed()
                    && has_reusable_head_failure(
                        root,
                        &config.agent,
                        expectation,
                        &mut history_cache,
                        &mut scope_hash_cache,
                    )? => {}
            Some(record) => failing.push(record),
            None => missing.push(expectation.number),
        }
    }

    if missing.is_empty() && failing.is_empty() {
        return Ok(());
    }
    if !failing.is_empty() {
        eprintln!("canon gate: cached failing expectation results:");
        for record in &failing {
            eprint!("{}", render_check_log_record(record));
        }
    }
    if !missing.is_empty() {
        eprintln!(
            "canon gate: missing cached answers for expectations: {}",
            join_numbers(&missing)
        );
        if failing.is_empty() {
            eprintln!("canon gate: run `canon check` before committing");
        } else {
            eprintln!(
                "canon gate: after fixing cached failures, run `canon check` to populate missing cache records"
            );
        }
    }
    Err(CommandError::GateFailed)
}

pub(crate) fn should_skip_gate_for_canon_only_unchanged_visible_content(
    root: &Path,
    agent: &AgentConfig,
    changed_paths: &[String],
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<bool, String> {
    if !is_canon_only_staged_change(changed_paths) {
        return Ok(false);
    }
    let index_hash = scope_hash_cache.staged_scope_hash(root, agent, &full_scope())?;
    let head_hash = scope_hash_cache.scope_hash_for_source(
        root,
        agent,
        &full_scope(),
        ScopeHashSource::Head,
    )?;
    Ok(head_hash.as_deref() == Some(index_hash.as_str()))
}

pub(crate) fn has_reusable_head_failure(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<bool, String> {
    Ok(matches!(
        reusable_history_record_for_source(
            root,
            agent,
            expectation,
            ScopeHashSource::Head,
            history_cache,
            scope_hash_cache
        )?,
        Some(record) if !record.passed()
    ))
}
