use crate::*;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    let started = Instant::now();
    install_sigint_handler();
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
    let command = parse_check_command_args(args)?;
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, &command.config_path)?;
    if let Some(question) = command.query.as_deref() {
        return run_check_query_command(root, &config, question, &mut repo_cache);
    }
    // `canon check` accepts `--fail-fast` through `parse_check_options`; the
    // `canon gate` rejection below is gate-specific and does not apply here.
    let options = parse_check_options(&config, &command.option_args)?;
    fail_on_mixed_canon_changes(root)?;
    // Apply the staged Git snapshot as an in-place index view: unstaged and
    // untracked worktree changes are preserved away, so the evaluator sees the
    // index contents at the real project root. This creates no copied
    // repository, copied tree, or copied snapshot directory. File visibility is
    // enforced by app-server permissions, not by copying the repository to a
    // filtered view.
    let _staged_view = StagedWorktreeView::apply(root)?;
    let mut runner = LazyAppServerRunner::new(check_config_loads_plugins(&config), &config.agent);
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    diagnostic_log.write_event(
        "info",
        "check.start",
        &[(
            "selected",
            json!(options
                .selected
                .iter()
                .map(|expectation| expectation.number)
                .collect::<Vec<_>>()),
        )],
    )?;
    let cleanup = maybe_cleanup_stale_cache_dirs(root, &config)?;
    if cleanup.sampled {
        diagnostic_log.write_event(
            "info",
            "cache.cleanup",
            &[
                ("removed", json!(cleanup.removed)),
                ("kept", json!(cleanup.kept)),
            ],
        )?;
    }
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut result_output: &mut dyn Write = &mut stdout;
    // `run_check_with_runner` calls `write_and_flush_result_output` after each
    // selected expectation, so stdout observers receive each JSONL result before
    // the next expectation starts.
    let records_result = run_check_with_runner(
        root,
        root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
    );
    result_output
        .flush()
        .map_err(|err| format!("failed to flush check result to stdout: {}", err))?;
    runner.drain_token_usage_updates();
    let usage = runner.token_usage().unwrap_or_default();
    diagnostic_log.write_event("info", "token.usage", &token_usage_log_fields(usage))?;
    print_token_usage_summary(Some(usage));
    let report = match records_result {
        Ok(report) => report,
        Err(err) => {
            let report = CheckRunReport {
                records: Vec::new(),
                skipped: 0,
                narrowing: NarrowingStats::default(),
            };
            diagnostic_log.write_event(
                "info",
                "check.finish",
                &[
                    ("passed", json!(0)),
                    ("failed", json!(0)),
                    ("errors", json!(0)),
                    ("skipped", json!(0)),
                    ("narrowingAttempted", json!(0)),
                    ("narrowingAccepted", json!(0)),
                    ("narrowingRejected", json!(0)),
                    ("error", json!(err)),
                ],
            )?;
            write_summary_line(&mut result_output, &report, started.elapsed())?;
            return Err(CHECK_FAILED_EXIT.to_string());
        }
    };
    diagnostic_log.write_event(
        "info",
        "check.finish",
        &[
            ("passed", json!(report_passed_count(&report))),
            ("failed", json!(report_failed_count(&report))),
            ("errors", json!(report_error_count(&report))),
            ("skipped", json!(report.skipped)),
            ("narrowingAttempted", json!(report.narrowing.attempted)),
            ("narrowingAccepted", json!(report.narrowing.accepted)),
            ("narrowingRejected", json!(report.narrowing.rejected)),
        ],
    )?;
    write_summary_line(&mut result_output, &report, started.elapsed())?;
    if report.records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err(CHECK_FAILED_EXIT.to_string())
    }
}

pub(crate) fn run_check_query_command(
    root: &Path,
    config: &CheckConfig,
    question: &str,
    repo_cache: &mut RepoInspectionCache,
) -> Result<(), String> {
    // `canon check -q` is an ad-hoc interrogation mode. It loads the active
    // evaluator config, but it does not select or run expectations and is not a
    // per-expectation check run governed by the normal check-output summary.
    install_sigint_handler();
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
    fail_on_mixed_canon_changes(root)?;
    let _staged_view = StagedWorktreeView::apply(root)?;
    let mut runner = LazyAppServerRunner::new(check_config_loads_plugins(config), &config.agent);
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, repo_cache)?;
    diagnostic_log.write_event(
        "info",
        "check.start",
        &[
            ("query", json!(true)),
            ("selected", json!(Vec::<usize>::new())),
        ],
    )?;
    let runtime = CheckRuntime {
        root,
        snapshot_root: root,
        config,
    };
    let mut interrogation_state = InterrogationState::new();
    let result = run_query_with_runner(
        &runtime,
        question,
        &mut runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    );
    runner.drain_token_usage_updates();
    let usage = runner.token_usage().unwrap_or_default();
    diagnostic_log.write_event("info", "token.usage", &token_usage_log_fields(usage))?;
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            print_token_usage_summary(Some(usage));
            diagnostic_log.write_event(
                "info",
                "check.finish",
                &[
                    ("query", json!(true)),
                    ("passed", json!(0)),
                    ("failed", json!(0)),
                    ("errors", json!(1)),
                    ("skipped", json!(0)),
                    ("narrowingAttempted", json!(0)),
                    ("narrowingAccepted", json!(0)),
                    ("narrowingRejected", json!(0)),
                ],
            )?;
            return Err(err);
        }
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_query_output(&mut stdout, &result.answer)?;
    print_token_usage_summary(Some(usage));
    diagnostic_log.write_event(
        "info",
        "check.finish",
        &[
            ("query", json!(true)),
            ("passed", json!(0)),
            ("failed", json!(0)),
            ("errors", json!(0)),
            ("skipped", json!(0)),
            ("narrowingAttempted", json!(0)),
            ("narrowingAccepted", json!(0)),
            ("narrowingRejected", json!(0)),
        ],
    )
}

pub(crate) fn print_token_usage_summary(usage: Option<TokenUsage>) {
    eprintln!("{}", render_token_usage_summary(usage.unwrap_or_default()));
}

pub(crate) fn install_sigint_handler() {
    SIGNAL_HANDLER_INIT.call_once(|| {
        #[cfg(unix)]
        unsafe {
            const SIGHUP: i32 = 1;
            const SIGINT: i32 = 2;
            const SIGTERM: i32 = 15;
            let _ = signal(SIGHUP, handle_sigint);
            let _ = signal(SIGINT, handle_sigint);
            let _ = signal(SIGTERM, handle_sigint);
        }
    });
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.iter().any(|arg| arg.to_str() == Some("--fail-fast")) {
        return Err("canon gate does not accept --fail-fast".to_string());
    }
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".to_string());
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
        if let Some(record) =
            cooldown_history_record(root, expectation, &mut history_cache, unix_timestamp()?)?
        {
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
    if !missing.is_empty() {
        eprintln!(
            "canon gate: missing cached answers for expectations: {}",
            join_numbers(&missing)
        );
        if failing.is_empty() {
            eprintln!("canon gate: run `canon check` before committing");
        } else {
            eprintln!("canon gate: fix cached failures before checking missing cached answers");
        }
    }
    if !failing.is_empty() {
        eprintln!("canon gate: cached failing expectation results:");
        for record in &failing {
            eprint!("{}", render_check_log_record(record));
        }
    }
    Err(GATE_FAILED_EXIT.to_string())
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

pub(crate) fn fail_on_mixed_canon_changes(root: &Path) -> Result<(), String> {
    // Policy-edit isolation happens before cache lookup or gate evaluation.
    // Cache behavior only applies after this staged-path preflight accepts the
    // change set.
    fail_on_mixed_canon_paths(&staged_changed_paths(root)?)
}

pub(crate) fn staged_changed_paths(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .arg("--diff-filter=ACDMRTUXB")
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git diff output must be valid UTF-8".to_string())?;
    let paths = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    Ok(paths)
}

pub(crate) fn fail_on_mixed_canon_paths(paths: &[String]) -> Result<(), String> {
    let has_canon = paths.iter().any(|path| is_canon_project_path(path));
    let has_other = paths.iter().any(|path| !is_canon_project_path(path));
    if has_canon && has_other {
        return Err(
            "canon check failed: .canon/** changes must not be mixed with non-.canon changes"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn is_canon_project_path(path: &str) -> bool {
    path == ".canon" || path.starts_with(".canon/")
}

pub(crate) fn is_canon_only_staged_change(paths: &[String]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_canon_project_path(path))
}

pub(crate) fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    mut result_output: Option<&mut dyn Write>,
) -> Result<CheckRunReport, String> {
    let mut records = Vec::new();
    let mut skipped = 0usize;
    let mut narrowing = NarrowingStats::default();
    let runtime = CheckRuntime {
        root,
        snapshot_root,
        config,
    };
    let mut interrogation_state = InterrogationState::new();
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    for expectation in &options.selected {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        if !options.ignore_cache {
            if let Some(record) =
                cooldown_history_record(root, expectation, &mut history_cache, unix_timestamp()?)?
            {
                let should_stop = options.fail_fast && !record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "info",
                        "cache.cooldown_hit",
                        &[
                            ("number", json!(record.number)),
                            ("scope", json!(record.scope)),
                            ("scopeHash", json!(record.scope_hash)),
                        ],
                    )?;
                    writer.write_record(&record)?;
                }
                if record.passed() {
                    skipped += 1;
                } else {
                    write_and_flush_result_output(&mut result_output, &record)?;
                }
                records.push(record);
                if should_stop {
                    return Ok(CheckRunReport {
                        records,
                        skipped,
                        narrowing,
                    });
                }
                continue;
            }
            // Cooldown is intentionally pass-only. A latest cached fail returns
            // no cooldown hit above, then falls through to this exact-cache
            // lookup where reusable pass and fail records are both valid hits.
            if let Some(record) = reusable_history_record_for_source(
                root,
                &config.agent,
                expectation,
                ScopeHashSource::Index,
                &mut history_cache,
                &mut scope_hash_cache,
            )? {
                let should_stop = options.fail_fast && !record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "info",
                        "cache.exact_hit",
                        &[
                            ("number", json!(record.number)),
                            ("result", json!(record.result)),
                            ("scope", json!(record.scope)),
                            ("scopeHash", json!(record.scope_hash)),
                        ],
                    )?;
                    writer.write_record(&record)?;
                }
                if record.passed() {
                    skipped += 1;
                } else {
                    write_and_flush_result_output(&mut result_output, &record)?;
                }
                records.push(record);
                if should_stop {
                    return Ok(CheckRunReport {
                        records,
                        skipped,
                        narrowing,
                    });
                }
                continue;
            }
        }

        let scope =
            latest_history_scope_with_cache(root, &config.agent, expectation, &mut history_cache)?
                .unwrap_or_else(full_scope);
        let mut enforced_scope = scope.clone();
        let mut interrogation = match interrogate_expectation(
            &runtime,
            expectation,
            runner,
            &mut diagnostic_log,
            &mut interrogation_state,
            &enforced_scope,
        ) {
            Ok(interrogation) => interrogation,
            Err(err) => InterrogationResult {
                record: error_record_from_interrogation_error(
                    root,
                    &config.agent,
                    expectation,
                    &enforced_scope,
                    &err,
                    &mut scope_hash_cache,
                )?,
            },
        };
        if should_retry_full_scope(&interrogation.record, &enforced_scope) {
            // Widening after a restricted idk is not narrowing verification:
            // it is a separate full-scope interrogation whose record replaces
            // the restricted non-answer.
            enforced_scope = full_scope();
            interrogation = match interrogate_expectation(
                &runtime,
                expectation,
                runner,
                &mut diagnostic_log,
                &mut interrogation_state,
                &enforced_scope,
            ) {
                Ok(interrogation) => interrogation,
                Err(err) => InterrogationResult {
                    record: error_record_from_interrogation_error(
                        root,
                        &config.agent,
                        expectation,
                        &enforced_scope,
                        &err,
                        &mut scope_hash_cache,
                    )?,
                },
            };
        }

        let record_scope = interrogation.record.scope.clone();
        if !record_requires_human_review(&interrogation.record)
            && is_strict_scope_subset(&record_scope, &enforced_scope)
        {
            narrowing.attempted += 1;
            // A narrower scope from one evaluator response becomes reusable
            // only if an independent interrogation with that same canonical
            // scope preserves the answer.
            let initial_record = interrogation.record.clone();
            let narrowed = match interrogate_expectation(
                &runtime,
                expectation,
                runner,
                &mut diagnostic_log,
                &mut interrogation_state,
                &record_scope,
            ) {
                Ok(interrogation) => interrogation,
                Err(err) => InterrogationResult {
                    record: error_record_from_interrogation_error(
                        root,
                        &config.agent,
                        expectation,
                        &record_scope,
                        &err,
                        &mut scope_hash_cache,
                    )?,
                },
            };
            if narrowed.record.observed == interrogation.record.observed {
                narrowing.accepted += 1;
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "info",
                        "scope.narrowing",
                        &scope_narrowing_log_fields(
                            expectation.number,
                            &enforced_scope,
                            &record_scope,
                            true,
                            &initial_record,
                            &narrowed.record,
                        ),
                    )?;
                }
                interrogation = narrowed;
            } else {
                narrowing.rejected += 1;
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "info",
                        "scope.narrowing",
                        &scope_narrowing_log_fields(
                            expectation.number,
                            &enforced_scope,
                            &record_scope,
                            false,
                            &initial_record,
                            &narrowed.record,
                        ),
                    )?;
                }
                let enforced_scope_hash =
                    scope_hash_cache.staged_scope_hash(root, &config.agent, &enforced_scope)?;
                interrogation.record.scope = enforced_scope.clone();
                interrogation.record.scope_hash = enforced_scope_hash;
            }
        }

        if is_verified_record(&interrogation.record) {
            append_history_record_with_cache(
                root,
                expectation,
                &interrogation.record,
                &mut history_cache,
            )?;
        }
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_record(&interrogation.record)?;
        }
        let should_stop = options.fail_fast && !interrogation.record.passed();
        write_and_flush_result_output(&mut result_output, &interrogation.record)?;
        records.push(interrogation.record);
        if should_stop {
            return Ok(CheckRunReport {
                records,
                skipped,
                narrowing,
            });
        }
    }
    Ok(CheckRunReport {
        records,
        skipped,
        narrowing,
    })
}

pub(crate) fn scope_narrowing_log_fields(
    number: usize,
    original_scope: &[String],
    proposed_scope: &[String],
    accepted: bool,
    initial: &CheckRecord,
    verification: &CheckRecord,
) -> Vec<(&'static str, Value)> {
    vec![
        ("number", json!(number)),
        ("originalScope", json!(original_scope)),
        ("proposedScope", json!(proposed_scope)),
        ("accepted", json!(accepted)),
        ("initialObserved", json!(initial.observed.clone())),
        ("initialEvidence", json!(initial.evidence.clone())),
        ("verificationObserved", json!(verification.observed.clone())),
        ("verificationEvidence", json!(verification.evidence.clone())),
    ]
}

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
        result: RESULT_FAIL.to_string(),
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: UNPARSEABLE_OBSERVED.to_string(),
        evidence: error.to_string(),
        scope: scope.to_vec(),
        scope_hash,
    })
}

pub(crate) fn report_passed_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| record.passed())
        .count()
        .saturating_sub(report.skipped)
}

pub(crate) fn report_failed_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| !record.passed() && !record_requires_human_review(record))
        .count()
}

pub(crate) fn report_error_count(report: &CheckRunReport) -> usize {
    report
        .records
        .iter()
        .filter(|record| record_requires_human_review(record))
        .count()
}

pub(crate) fn should_retry_full_scope(record: &CheckRecord, scope: &[String]) -> bool {
    scope != full_scope() && (record.observed == OBSERVED_IDK || is_scope_widening_record(record))
}

pub(crate) fn evaluator_session_key(model: Option<&str>, scope: &[String]) -> String {
    let mut key = app_server_model_key(model);
    key.push('\0');
    key.push_str(&scope.join("\n"));
    key
}

pub(crate) fn is_scope_widening_record(record: &CheckRecord) -> bool {
    record.observed == UNPARSEABLE_OBSERVED
        && record.evidence.starts_with("evaluator response scope ")
}

pub(crate) struct CheckRuntime<'a> {
    pub(crate) root: &'a Path,
    pub(crate) snapshot_root: &'a Path,
    pub(crate) config: &'a CheckConfig,
}

pub(crate) struct InterrogationState {
    sessions: BTreeMap<String, String>,
    scope_hash_cache: ScopeHashCache,
    parse_cache: EvaluatorResponseParseCache,
}

impl InterrogationState {
    pub(crate) fn new() -> InterrogationState {
        InterrogationState {
            sessions: BTreeMap::new(),
            scope_hash_cache: ScopeHashCache::new(),
            parse_cache: EvaluatorResponseParseCache::new(),
        }
    }
}

pub(crate) fn interrogate_expectation<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    let mut failures = Vec::new();
    let models = evaluator_models(&runtime.config.agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match interrogate_expectation_with_model(
            runtime,
            expectation,
            runner,
            diagnostic_log,
            state,
            enforced_scope,
            model.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "warn",
                        "model.failure",
                        &[
                            ("number", json!(expectation.number)),
                            ("model", json!(model_label(model.as_deref()))),
                            ("error", json!(err)),
                        ],
                    )?;
                    if let Some(next_model) = models.get(model_index + 1) {
                        writer.write_event(
                            "warn",
                            "model.fallback",
                            &[
                                ("number", json!(expectation.number)),
                                ("from", json!(model_label(model.as_deref()))),
                                ("to", json!(model_label(next_model.as_deref()))),
                                ("reason", json!(err.clone())),
                            ],
                        )?;
                    }
                }
                failures.push(format!("{}: {}", model_label(model.as_deref()), err));
            }
            Err(err) => return Err(err),
        }
    }
    Err(format!(
        "all evaluator models failed: {}",
        failures.join("; ")
    ))
}

pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, String> {
    let config = runtime.config;
    let enforced_scope = sanitize_scope(enforced_scope, &config.agent)?;
    // Threads are reused only for the same canonical enforced scope within one
    // model-specific app-server runner. Fallback models live in separate
    // app-server processes, so their session IDs cannot be shared.
    let session_key = evaluator_session_key(model, &enforced_scope);
    let existing_session = state.sessions.get(&session_key).cloned();
    let had_existing_session = existing_session.is_some();
    let mut session_id = match existing_session {
        Some(existing) => {
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.reuse",
                    &[
                        ("threadId", json!(existing.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                    ],
                )?;
            }
            existing
        }
        None => {
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            let created = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                effective_thinking(&config.agent, expectation),
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(created.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            created
        }
    };
    let prompt = question_prompt(&expectation.q, &enforced_scope)?;
    let thinking = effective_thinking(&config.agent, expectation);
    let turn = EvaluatorTurnContext {
        session_id: &session_id,
        model,
        thinking,
    };
    let response = match ask_with_repairs(
        runner,
        &turn,
        &prompt,
        &config.agent,
        &mut state.parse_cache,
        diagnostic_log,
        expectation.number,
    ) {
        Ok(response) => response,
        Err(err) if had_existing_session && is_context_window_failure(&err) => {
            state.sessions.remove(&session_key);
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "warn",
                    "thread.restart",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("number", json!(expectation.number)),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("reason", json!(err)),
                    ],
                )?;
            }
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            session_id = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                effective_thinking(&config.agent, expectation),
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            let turn = EvaluatorTurnContext {
                session_id: &session_id,
                model,
                thinking,
            };
            match ask_with_repairs(
                runner,
                &turn,
                &prompt,
                &config.agent,
                &mut state.parse_cache,
                diagnostic_log,
                expectation.number,
            ) {
                Ok(response) => response,
                Err(err) => {
                    if is_model_technical_failure(&err) {
                        state.sessions.remove(&session_key);
                    }
                    return Err(err);
                }
            }
        }
        Err(err) => {
            if is_model_technical_failure(&err) {
                state.sessions.remove(&session_key);
            }
            return Err(err);
        }
    };
    state.sessions.insert(session_key, session_id.clone());
    let mut response = response;
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope.to_vec();
    } else {
        if !scope_is_within(&response.scope, &enforced_scope) {
            response = ParsedAnswer {
                answer: UNPARSEABLE_OBSERVED.to_string(),
                evidence: format!(
                    "evaluator response scope {:?} widens enforced scope {:?}",
                    response.scope, enforced_scope
                ),
                scope: enforced_scope.to_vec(),
            };
        }
    }
    let record_scope = response.scope.clone();
    let scope_hash =
        state
            .scope_hash_cache
            .staged_scope_hash(runtime.root, &config.agent, &record_scope)?;
    let record = record_from_response(expectation, response, record_scope, scope_hash)?;
    if record.observed == OBSERVED_MALFORMED {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[
                    ("number", json!(expectation.number)),
                    ("reason", json!(MALFORMED_REVIEW_WARNING)),
                ],
            )?;
        }
    }
    if record.observed == OBSERVED_IDK && enforced_scope == full_scope() {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[
                    ("number", json!(expectation.number)),
                    ("reason", json!("full-scope idk")),
                ],
            )?;
        }
    }
    if record.evidence.trim().is_empty() {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "evidence.empty",
                &[("number", json!(expectation.number))],
            )?;
        }
    }
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_interrogation_record(&record)?;
    }
    Ok(InterrogationResult { record })
}

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
) -> Result<QueryInterrogationResult, String> {
    let mut diagnostic_log = diagnostic_log;
    let mut failures = Vec::new();
    let models = evaluator_models(&runtime.config.agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match interrogate_query_with_model(
            runtime,
            question,
            runner,
            &mut diagnostic_log,
            state,
            model.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "warn",
                        "model.failure",
                        &[
                            ("number", json!(0)),
                            ("model", json!(model_label(model.as_deref()))),
                            ("error", json!(err)),
                        ],
                    )?;
                    if let Some(next_model) = models.get(model_index + 1) {
                        writer.write_event(
                            "warn",
                            "model.fallback",
                            &[
                                ("number", json!(0)),
                                ("from", json!(model_label(model.as_deref()))),
                                ("to", json!(model_label(next_model.as_deref()))),
                                ("reason", json!(err.clone())),
                            ],
                        )?;
                    }
                }
                failures.push(format!("{}: {}", model_label(model.as_deref()), err));
            }
            Err(err) => return Err(err),
        }
    }
    Err(format!(
        "all evaluator models failed: {}",
        failures.join("; ")
    ))
}

pub(crate) fn interrogate_query_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    model: Option<&str>,
) -> Result<QueryInterrogationResult, String> {
    let config = runtime.config;
    let enforced_scope = full_scope();
    let session_key = evaluator_session_key(model, &enforced_scope);
    let existing_session = state.sessions.get(&session_key).cloned();
    let had_existing_session = existing_session.is_some();
    let mut session_id = match existing_session {
        Some(existing) => {
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.reuse",
                    &[
                        ("threadId", json!(existing.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("thinking", json!(config.agent.thinking.clone())),
                    ],
                )?;
            }
            existing
        }
        None => {
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            let created = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                &config.agent.thinking,
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(created.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("thinking", json!(config.agent.thinking.clone())),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            created
        }
    };
    let prompt = question_prompt(question, &enforced_scope)?;
    let turn = EvaluatorTurnContext {
        session_id: &session_id,
        model,
        thinking: &config.agent.thinking,
    };
    let response = match ask_with_repairs(
        runner,
        &turn,
        &prompt,
        &config.agent,
        &mut state.parse_cache,
        diagnostic_log,
        0,
    ) {
        Ok(response) => response,
        Err(err) if had_existing_session && is_context_window_failure(&err) => {
            state.sessions.remove(&session_key);
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "warn",
                    "thread.restart",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("number", json!(0)),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("reason", json!(err)),
                    ],
                )?;
            }
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            session_id = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                &config.agent.thinking,
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("thinking", json!(config.agent.thinking.clone())),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            let turn = EvaluatorTurnContext {
                session_id: &session_id,
                model,
                thinking: &config.agent.thinking,
            };
            match ask_with_repairs(
                runner,
                &turn,
                &prompt,
                &config.agent,
                &mut state.parse_cache,
                diagnostic_log,
                0,
            ) {
                Ok(response) => response,
                Err(err) => {
                    if is_model_technical_failure(&err) {
                        state.sessions.remove(&session_key);
                    }
                    return Err(err);
                }
            }
        }
        Err(err) => {
            if is_model_technical_failure(&err) {
                state.sessions.remove(&session_key);
            }
            return Err(err);
        }
    };
    state.sessions.insert(session_key, session_id.clone());
    let mut response = response;
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope.to_vec();
    } else {
        if !scope_is_within(&response.scope, &enforced_scope) {
            response = ParsedAnswer {
                answer: UNPARSEABLE_OBSERVED.to_string(),
                evidence: format!(
                    "evaluator response scope {:?} widens enforced scope {:?}",
                    response.scope, enforced_scope
                ),
                scope: enforced_scope.to_vec(),
            };
        }
    }
    let scope_hash =
        state
            .scope_hash_cache
            .staged_scope_hash(runtime.root, &config.agent, &response.scope)?;
    if response.answer == OBSERVED_MALFORMED {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[
                    ("number", json!(0)),
                    ("reason", json!(MALFORMED_REVIEW_WARNING)),
                ],
            )?;
        }
    }
    if response.answer == OBSERVED_IDK {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[("number", json!(0)), ("reason", json!("full-scope idk"))],
            )?;
        }
    }
    if response.evidence.trim().is_empty() {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event("warn", "evidence.empty", &[("number", json!(0))])?;
        }
    }
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "query.result",
            &[
                ("prompt", json!(question)),
                ("observed", json!(response.answer.clone())),
                ("evidence", json!(response.evidence.clone())),
                ("scope", json!(response.scope.clone())),
                ("scopeHash", json!(scope_hash.clone())),
            ],
        )?;
    }
    Ok(QueryInterrogationResult { answer: response })
}

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    let mut models = vec![agent.model.primary.clone()];
    models.extend(agent.model.fallbacks.iter().cloned().map(Some));
    models
}

pub(crate) fn effective_thinking<'a>(
    agent: &'a AgentConfig,
    expectation: &'a SelectedExpectation,
) -> &'a str {
    expectation.thinking.as_deref().unwrap_or(&agent.thinking)
}

pub(crate) fn model_label(model: Option<&str>) -> &str {
    model.unwrap_or("<default>")
}

pub(crate) fn token_usage_log_fields(usage: TokenUsage) -> Vec<(&'static str, Value)> {
    vec![
        ("total", json!(usage.total_tokens)),
        ("input", json!(usage.input_tokens)),
        ("cached_input", json!(usage.cached_input_tokens)),
        ("output", json!(usage.output_tokens)),
        ("reasoning_output", json!(usage.reasoning_output_tokens)),
    ]
}

pub(crate) fn is_model_technical_failure(err: &str) -> bool {
    err.contains("usageLimitExceeded")
        || err.contains("usage limit")
        || err.contains("rate limit")
        || err.contains("model unavailable")
        || err.contains("model is unavailable")
        || err.contains("timed out")
        || is_context_window_failure(err)
}

pub(crate) fn is_context_window_failure(err: &str) -> bool {
    err.contains("context window") || err.contains("ran out of room")
}

pub(crate) fn record_from_response(
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    let result = if response.answer == expectation.a {
        RESULT_PASS
    } else {
        RESULT_FAIL
    };
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
        number: expectation.number,
        result: result.to_string(),
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: response.answer,
        evidence: response.evidence,
        scope: enforced_scope,
        scope_hash,
    })
}

pub(crate) fn is_verified_record(record: &CheckRecord) -> bool {
    is_reusable_history_record(record)
}

pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) fn ask_with_repairs<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_number: usize,
) -> Result<ParsedAnswer, String> {
    let first = ask_and_log(
        runner,
        turn,
        prompt,
        diagnostic_log,
        expectation_number,
        1,
        "initial",
    )?;
    let mut next_attempt = 2;
    let mut format_retried = false;
    let mut parsed = match parser_cache.parse(&first, agent) {
        Ok(answer) => answer,
        Err(_err) => {
            let first_excerpt = response_excerpt(&first);
            format_retried = true;
            let repaired = ask_and_log(
                runner,
                turn,
                prompt,
                diagnostic_log,
                expectation_number,
                next_attempt,
                "parse-retry",
            )?;
            next_attempt += 1;
            match parser_cache.parse(&repaired, agent) {
                Ok(answer) => answer,
                Err(err) => ParsedAnswer {
                    answer: UNPARSEABLE_OBSERVED.to_string(),
                    evidence: format!(
                        "evaluator response could not be parsed after retry: {}\nfirst response: {}\nrepair response: {}",
                        err,
                        first_excerpt,
                        response_excerpt(&repaired)
                    ),
                    scope: full_scope(),
                },
            }
        }
    };

    if parsed.answer == OBSERVED_MALFORMED && !format_retried {
        let repaired = ask_and_log(
            runner,
            turn,
            prompt,
            diagnostic_log,
            expectation_number,
            next_attempt,
            "malformed-retry",
        )?;
        next_attempt += 1;
        if let Ok(answer) = parser_cache.parse(&repaired, agent) {
            parsed = answer;
        }
    }

    if parsed.evidence.trim().is_empty() {
        let repaired = ask_and_log(
            runner,
            turn,
            prompt,
            diagnostic_log,
            expectation_number,
            next_attempt,
            "evidence-retry",
        )?;
        if let Ok(answer) = parser_cache.parse(&repaired, agent) {
            parsed = answer;
        }
    }

    Ok(parsed)
}

pub(crate) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_number: usize,
    attempt: usize,
    reason: &str,
) -> Result<String, String> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "agent.request",
            &[
                ("number", json!(expectation_number)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("raw", json!(prompt)),
            ],
        )?;
    }
    let response = runner.ask(turn.session_id, prompt, turn.model, turn.thinking)?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "agent.response",
            &[
                ("number", json!(expectation_number)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("raw", json!(response.clone())),
            ],
        )?;
    }
    Ok(response)
}

#[derive(Default)]
pub(crate) struct EvaluatorResponseParseCache {
    values: BTreeMap<(String, Vec<String>), Result<ParsedAnswer, String>>,
}

impl EvaluatorResponseParseCache {
    pub(crate) fn new() -> EvaluatorResponseParseCache {
        EvaluatorResponseParseCache::default()
    }

    pub(crate) fn parse(
        &mut self,
        text: &str,
        agent: &AgentConfig,
    ) -> Result<ParsedAnswer, String> {
        let key = (text.to_string(), effective_ignore_patterns(agent));
        if let Some(parsed) = self.values.get(&key) {
            return parsed.clone();
        }
        let parsed = parse_evaluator_response(text, agent);
        self.values.insert(key, parsed.clone());
        parsed
    }
}

pub(crate) fn response_excerpt(text: &str) -> String {
    const LIMIT: usize = 600;
    let text = text.trim();
    if text.is_empty() {
        return "<empty>".to_string();
    }
    let mut excerpt = text.chars().take(LIMIT).collect::<String>();
    if text.chars().count() > LIMIT {
        excerpt.push_str("...");
    }
    excerpt
}

pub(crate) fn question_prompt(question: &str, _scope: &[String]) -> Result<String, String> {
    Ok(question.to_string())
}

pub(crate) fn developer_instructions(agent: &AgentConfig, scope: &[String]) -> String {
    format!(
        "{}\n\nEnforced scope: {}\n\nAnswer-selection policy:\n{}\n\nWhen the answer-selection policy says to answer exactly `yes`, `no`, `idk`, `malformed`, or an option letter, put that exact string in the JSON `answer` field. Never output the raw answer as the whole response.",
        response_format_block(),
        compact_json_string_array(scope),
        agent.instructions.trim(),
    )
}

pub(crate) fn response_format_block() -> &'static str {
    concat!(
        "Response format:\n",
        "Return exactly one valid JSON object and no markdown, code fences, or surrounding prose.\n",
        "Schema: {\"answer\":\"<single-line answer>\",\"evidence\":\"<free-form evidence citing supporting files or code>\",\"scope\":[\"<normalized repository-relative path>\"]}\n",
        "The `answer` field is where the exact yes/no/idk/malformed/option-letter answer goes; do not write that answer outside the JSON object. ",
        "`scope` is the smallest allowed project context sufficient to determine the correct answer among all valid answers; it is not the list of evidence citations. ",
        "Use [\".\"] when the answer depends on project-wide absence, consistency, duplication, garbage, overall quality, or denied/inaccessible paths. ",
        "If the enforced task `scope` is narrower than [\".\"] and the question requires repository-wide or cross-module evidence, answer `idk` instead of drawing a positive or negative conclusion from incomplete context. ",
        "Never include denied or inaccessible paths in `scope`. Denied paths are intentionally outside the allowed evidence boundary; do not answer `idk` solely because a denied path is unreadable. ",
        "The current project state is the staged Git snapshot exposed at the working directory; do not treat files that exist only in `HEAD`, cache history, or diagnostic logs as current project files. ",
        "The user-provided `agent.instructions` above are active project policy loaded from `.canon/check.yml`, not hardcoded implementation text and not necessarily the embedded default template shown in README; do not cite those instructions as `src/check.rs` contents. ",
        "For questions about copying the staged Git snapshot, `copy` means creating a duplicate repository/tree/snapshot directory; an in-place index view that uses `git stash --keep-index` only to preserve unstaged worktree changes is not a copy. ",
        "A reusable cache hit is not an evaluator interrogation; questions about every evaluator interrogation concern only turns where `canon check` actually asks the evaluator model. ",
        "For `.canon/check.yml` schema/configuration questions, do not try to answer by opening `.canon/check.yml`; that path is denied by design. Use the fact that `canon check` has already loaded and validated the active config before starting the evaluator, plus the visible README, parser, validation, and template code; do not answer `idk` solely because `.canon/check.yml` itself is denied. ",
        "For absence and quality questions, answer `yes` only when there is a concrete removable file, code path, hack, or idiom violation with evidence; answer `no` when repository-wide inspection finds no concrete candidate, because absolute proof of absence is not required. ",
        "Treat behavior required by the active check contract, such as staged-snapshot restoration, process-tree cleanup, and configured log rolling, as not avoidable by itself.\n",
    )
}

pub(crate) fn parse_evaluator_response(
    text: &str,
    agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    if response.answer.contains('\n') || response.answer.contains('\r') {
        return Err("answer must be a single-line string".to_string());
    }
    Ok(ParsedAnswer {
        answer: response.answer,
        evidence: response.evidence,
        scope: parse_scope_strings(&response.scope, agent)?,
    })
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let payload = evaluator_response_json_payload(text)?;
    serde_json::from_str::<EvaluatorResponseJson>(payload)
        .map_err(|err| format!("failed to parse evaluator JSON response: {}", err))
}

pub(crate) fn evaluator_response_json_payload(text: &str) -> Result<&str, String> {
    let trimmed = text.trim();
    validate_evaluator_response_key_order(trimmed)?;
    Ok(trimmed)
}

// The evaluator protocol intentionally makes the top-level key order part of
// the response format so logs, stdout, and human review stay predictable.
// serde_json validates field names and types below, but it does not expose
// source object order through the normal typed-deserialization path, so this
// small scanner only checks the top-level object envelope before serde parses
// the actual field values.
pub(crate) fn validate_evaluator_response_key_order(text: &str) -> Result<(), String> {
    let keys = top_level_json_object_keys(text)?;
    if keys == ["answer", "evidence", "scope"] {
        Ok(())
    } else {
        Err(format!(
            "evaluator JSON response must contain keys in order answer, evidence, scope; got {}",
            keys.join(", ")
        ))
    }
}

pub(crate) fn top_level_json_object_keys(text: &str) -> Result<Vec<String>, String> {
    let bytes = text.as_bytes();
    let mut index = skip_json_ws(bytes, 0);
    if bytes.get(index) != Some(&b'{') {
        return Err("evaluator response must be a JSON object".to_string());
    }
    index += 1;
    let mut keys = Vec::new();
    loop {
        index = skip_json_ws(bytes, index);
        if bytes.get(index) == Some(&b'}') {
            index += 1;
            break;
        }
        let (key, next) = parse_json_string_at(bytes, index)?;
        keys.push(key);
        index = skip_json_ws(bytes, next);
        if bytes.get(index) != Some(&b':') {
            return Err("evaluator JSON object key must be followed by ':'".to_string());
        }
        index = skip_json_value(bytes, index + 1)?;
        index = skip_json_ws(bytes, index);
        match bytes.get(index) {
            Some(b',') => index += 1,
            Some(b'}') => {
                index += 1;
                break;
            }
            _ => return Err("evaluator JSON object contains trailing content".to_string()),
        }
    }
    if skip_json_ws(bytes, index) != bytes.len() {
        return Err("evaluator response must not contain surrounding prose".to_string());
    }
    Ok(keys)
}

pub(crate) fn skip_json_ws(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        index += 1;
    }
    index
}

pub(crate) fn parse_json_string_at(
    bytes: &[u8],
    mut index: usize,
) -> Result<(String, usize), String> {
    if bytes.get(index) != Some(&b'"') {
        return Err("expected JSON string key".to_string());
    }
    index += 1;
    let mut output = String::new();
    while let Some(byte) = bytes.get(index).copied() {
        index += 1;
        match byte {
            b'"' => return Ok((output, index)),
            b'\\' => {
                let escaped = bytes
                    .get(index)
                    .copied()
                    .ok_or_else(|| "unterminated JSON escape".to_string())?;
                index += 1;
                output.push(escaped as char);
            }
            byte => output.push(byte as char),
        }
    }
    Err("unterminated JSON string".to_string())
}

pub(crate) fn skip_json_value(bytes: &[u8], mut index: usize) -> Result<usize, String> {
    index = skip_json_ws(bytes, index);
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut saw_scalar = false;
    while let Some(byte) = bytes.get(index).copied() {
        if in_string {
            index += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                saw_scalar = true;
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                index += 1;
            }
            b'{' | b'[' => {
                stack.push(byte);
                index += 1;
            }
            b'}' => {
                if stack.last() == Some(&b'{') {
                    stack.pop();
                    index += 1;
                    if stack.is_empty() {
                        saw_scalar = true;
                    }
                } else if stack.is_empty() && saw_scalar {
                    return Ok(index);
                } else {
                    return Err("unbalanced JSON object".to_string());
                }
            }
            b']' => {
                if stack.last() == Some(&b'[') {
                    stack.pop();
                    index += 1;
                    if stack.is_empty() {
                        saw_scalar = true;
                    }
                } else {
                    return Err("unbalanced JSON array".to_string());
                }
            }
            b',' if stack.is_empty() && saw_scalar => return Ok(index),
            b' ' | b'\n' | b'\r' | b'\t' if stack.is_empty() && saw_scalar => return Ok(index),
            _ => {
                saw_scalar = true;
                index += 1;
            }
        }
        if stack.is_empty() && saw_scalar && !in_string {
            let next = skip_json_ws(bytes, index);
            if matches!(bytes.get(next), Some(b',' | b'}')) {
                return Ok(next);
            }
        }
    }
    if saw_scalar && stack.is_empty() && !in_string {
        Ok(index)
    } else {
        Err("unterminated JSON value".to_string())
    }
}

#[cfg(test)]
pub(crate) fn parse_scope_json(text: &str, agent: &AgentConfig) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("failed to parse SCOPE JSON: {}", err))?;
    let array = value
        .as_array()
        .ok_or("SCOPE must be a JSON array".to_string())?;
    let mut scope = Vec::new();
    for item in array {
        let raw = item
            .as_str()
            .ok_or("SCOPE entries must be strings".to_string())?;
        let normalized = normalize_repo_path(raw)?;
        if normalized != raw.trim() {
            return Err(format!("SCOPE entry must be normalized: {}", raw));
        }
        scope.push(normalized);
    }
    sanitize_scope(&scope, agent)
}

pub(crate) fn parse_scope_strings(
    scope: &[String],
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    let mut parsed = Vec::new();
    for raw in scope {
        let normalized = normalize_repo_path(raw)?;
        if normalized != raw.trim() {
            return Err(format!("scope entry must be normalized: {}", raw));
        }
        if normalized != "." && is_denied_path(agent, &normalized) {
            return Err(format!("scope entry is denied: {}", raw));
        }
        parsed.push(normalized);
    }
    sanitize_scope(&parsed, agent)
}

pub(crate) fn write_and_flush_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record(record);
        writer
            .write_all(line.as_bytes())
            .map_err(|err| format!("failed to write check result to stdout: {}", err))?;
        writer
            .flush()
            .map_err(|err| format!("failed to flush check result to stdout: {}", err))?;
    }
    Ok(())
}

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    result_output
        .write_all(line.as_bytes())
        .map_err(|err| format!("failed to write check summary to stdout: {}", err))?;
    result_output
        .flush()
        .map_err(|err| format!("failed to flush check summary to stdout: {}", err))
}

pub(crate) fn write_query_output(
    result_output: &mut dyn Write,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // Query output is intentionally separate from the selected-expectation
    // check output contract because query mode has no expectation number,
    // expected answer, reusable history write, or final check summary.
    let output = render_query_output(answer);
    result_output
        .write_all(output.as_bytes())
        .map_err(|err| format!("failed to write query result to stdout: {}", err))?;
    result_output
        .flush()
        .map_err(|err| format!("failed to flush query result to stdout: {}", err))
}

pub(crate) fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    output.push_str("Observed: ");
    output.push_str(&escape_check_output_text(&answer.answer));
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&answer.evidence));
    output.push('\n');
    output.push_str("Scope: ");
    output.push_str(&compact_json_string_array(&answer.scope));
    output.push('\n');
    output
}

pub(crate) fn render_check_output_record(record: &CheckRecord) -> String {
    if record.passed() {
        return format!("{}. OK\n", record.number);
    }
    let status = if record_requires_human_review(record) {
        "ERROR"
    } else {
        "FAILED"
    };
    let mut output = String::new();
    output.push_str(&format!("{}. {}\n", record.number, status));
    output.push_str(&escape_check_output_text(&record.prompt));
    output.push('\n');
    output.push_str("Expected: ");
    output.push_str(&escape_check_output_text(&record.expected));
    output.push('\n');
    output.push_str("Observed: ");
    output.push_str(&escape_check_output_text(&record.observed));
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&record.evidence));
    output.push('\n');
    if status == "FAILED" {
        output.push_str("Scope: ");
        output.push_str(&compact_json_string_array(&record.scope));
        output.push('\n');
    }
    output
}

pub(crate) fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut errors = 0usize;
    for record in &report.records {
        if record.passed() {
            passed += 1;
        } else if record_requires_human_review(record) {
            errors += 1;
        } else {
            failed += 1;
        }
    }
    passed = passed.saturating_sub(report.skipped);
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if errors > 0 {
        outcomes.push(format!(
            "{} {}",
            errors,
            if errors == 1 { "error" } else { "errors" }
        ));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    if report.skipped > 0 {
        outcomes.push(format!("{} skipped", report.skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    if inner.len() >= WIDTH {
        return format!("={inner}=");
    }
    let padding = WIDTH - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}

pub(crate) fn record_requires_human_review(record: &CheckRecord) -> bool {
    record.observed == OBSERVED_MALFORMED
        || record.observed == UNPARSEABLE_OBSERVED
        || record.observed == OBSERVED_IDK
}

pub(crate) fn compact_json_string_array(values: &[String]) -> String {
    let mut output = String::new();
    output.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        push_json_string(&mut output, value);
    }
    output.push(']');
    output
}

pub(crate) fn escape_check_output_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch <= '\u{1f}' => {
                let mut escaped = String::new();
                push_json_control_escape(&mut escaped, ch);
                output.push_str(&escaped);
            }
            ch => output.push(ch),
        }
    }
    output
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == RESULT_PASS
    }
}
