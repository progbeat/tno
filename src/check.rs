use crate::*;

pub(crate) fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    mut result_output: Option<&mut dyn Write>,
) -> Result<CheckRunReport, CheckRunError> {
    let mut records = Vec::new();
    // Start from the CLI candidate count, then shrink this as final-selection
    // rules remove candidates. The report's `selected` field is the final
    // selected count, not the raw command-line expansion.
    let mut selected = options.selected.len();
    let mut skipped = options.skipped;
    let mut silent = 0usize;
    let mut narrowing = NarrowingStats::default();
    let runtime = CheckRuntime {
        root,
        snapshot_root,
        config,
    };
    // Per-run state is shared so equal canonical enforced scopes can reuse one
    // ephemeral evaluator thread; InterrogationState stores thread IDs by scope,
    // so different enforced scopes still start separate threads within the run.
    let mut interrogation_state = InterrogationState::new();
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    macro_rules! run_try {
        ($expr:expr) => {
            $expr.map_err(|err| {
                check_run_error(&records, selected, skipped, silent, narrowing, err)
            })?
        };
    }
    let final_selection = run_try!(final_selected_expectations(
        root,
        &config.agent,
        options.selected.clone(),
        &mut history_cache,
        run_try!(unix_timestamp()),
    ));
    skipped += final_selection.skipped;
    silent += final_selection.skipped;
    selected = final_selection.selected.len();
    for expectation in &final_selection.selected {
        // Each branch that produces a selected CheckRecord writes and flushes
        // that record before moving to the next expectation. Reused passing
        // cache hits are final-selection deselections: once a cached pass has
        // satisfied the candidate, it is no longer a selected expectation and
        // is counted in the public "skipped" total as a non-selected one.
        if check_interrupted() {
            return Err(check_run_error(
                &records,
                selected,
                skipped,
                silent,
                narrowing,
                "interrupted".to_string(),
            ));
        }
        if !options.ignore_cache {
            if let Some(hit) = run_try!(cached_record_for_expectation(
                root,
                &config.agent,
                expectation,
                &mut history_cache,
                &mut scope_hash_cache,
            )) {
                let should_stop = options.fail_fast && !hit.record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    run_try!(write_cache_hit(writer, &hit));
                }
                if hit.record.passed() {
                    // A reusable passing exact-cache hit satisfies the
                    // candidate without leaving it in the final selected set.
                    // This keeps `selected + skipped == all expectations` and
                    // matches the check-output definition of skipped as the
                    // final non-selected expectation count.
                    selected = selected.saturating_sub(1);
                    skipped += 1;
                    silent += 1;
                    continue;
                } else {
                    run_try!(write_and_flush_result_output(
                        &mut result_output,
                        &hit.record
                    ));
                }
                records.push(hit.record);
                if should_stop {
                    return Ok(CheckRunReport {
                        records,
                        selected,
                        skipped,
                        silent,
                        narrowing,
                    });
                }
                continue;
            }
        }

        // `--ignore-cache` bypasses reusable answer records in the branch above,
        // but it does not erase the interrogation-policy scope seed: a fresh
        // evaluator turn still starts from the latest reusable history scope.
        let mut enforced_scope = run_try!(latest_history_scope_with_cache(
            root,
            &config.agent,
            expectation,
            &mut history_cache
        ))
        .unwrap_or_else(full_scope);
        // Response-format problems are handled inside this call: malformed,
        // unparseable, and empty-evidence evaluator responses get their one
        // same-interrogation retry; response parsing rejects extra JSON keys,
        // non-single-line answers, and non-normalized scope entries before
        // finalization can return a human-review record as Ok(...). Err here
        // means a technical runner/model/logging failure.
        let mut interrogation = run_try!(interrogate_with_full_scope_retry(
            ScopedInterrogation {
                root,
                runtime: &runtime,
                expectation,
                enforced_scope: &mut enforced_scope,
            },
            runner,
            &mut diagnostic_log,
            &mut interrogation_state,
            &mut scope_hash_cache,
        ));

        let record_scope = interrogation.record.scope.clone();
        // Interrogation finalization rejects evaluator-proposed widening before
        // this point: non-idk widening becomes an unparseable review record,
        // while restricted idk rejects the proposed scope and returns an
        // enforced-scope non-answer so full-scope retry can decide whether the
        // restricted context was insufficient.
        debug_assert!(scope_is_within(&record_scope, &enforced_scope));
        // Cache-spec narrowing verification applies only to verified answers.
        // Non-answer states (`idk`, `malformed`, unparseable) are never reusable
        // cache records and are handled by the response-repair/idk policy above.
        if !record_requires_human_review(&interrogation.record)
            && is_strict_scope_subset(&record_scope, &enforced_scope)
        {
            narrowing.attempted += 1;
            // A narrower scope from one evaluator response becomes reusable
            // when an independent interrogation with that same canonical scope
            // either preserves the answer or still finds the expectation
            // failing. A changed failing answer is safe to keep because it
            // remains an actionable incorrect result for the narrower scope.
            let initial_record = interrogation.record.clone();
            let narrowed = run_try!(interrogate_or_error_record(
                InterrogationCall {
                    root,
                    runtime: &runtime,
                    expectation,
                    scope: &record_scope,
                },
                runner,
                &mut diagnostic_log,
                &mut interrogation_state,
                &mut scope_hash_cache,
            ));
            if narrowed_scope_is_accepted(&interrogation.record, &narrowed.record) {
                narrowing.accepted += 1;
                run_try!(write_scope_narrowing_event(
                    &mut diagnostic_log,
                    expectation.number,
                    &enforced_scope,
                    &record_scope,
                    true,
                    &initial_record,
                    &narrowed.record,
                ));
                interrogation = narrowed;
            } else {
                narrowing.rejected += 1;
                run_try!(write_scope_narrowing_event(
                    &mut diagnostic_log,
                    expectation.number,
                    &enforced_scope,
                    &record_scope,
                    false,
                    &initial_record,
                    &narrowed.record,
                ));
                let enforced_scope_hash = run_try!(scope_hash_cache.staged_scope_hash(
                    root,
                    &config.agent,
                    &enforced_scope
                ));
                interrogation.record.scope = enforced_scope.clone();
                interrogation.record.scope_hash = enforced_scope_hash;
            }
        }
        // Only verified yes/no/option answer records are reusable. Human-review
        // states such as idk, malformed, and rejected widened scopes are not
        // written to history.
        if is_verified_record(&interrogation.record) {
            run_try!(append_history_record_with_cache(
                root,
                expectation,
                &interrogation.record,
                &mut history_cache,
            ));
        }
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            run_try!(writer.write_record(&interrogation.record));
        }
        let should_stop = options.fail_fast && !interrogation.record.passed();
        run_try!(write_and_flush_result_output(
            &mut result_output,
            &interrogation.record
        ));
        records.push(interrogation.record);
        if should_stop {
            return Ok(CheckRunReport {
                records,
                selected,
                skipped,
                silent,
                narrowing,
            });
        }
    }
    Ok(CheckRunReport {
        records,
        selected,
        skipped,
        silent,
        narrowing,
    })
}

struct InterrogationCall<'a> {
    root: &'a Path,
    runtime: &'a CheckRuntime<'a>,
    expectation: &'a SelectedExpectation,
    scope: &'a [String],
}

struct ScopedInterrogation<'a> {
    root: &'a Path,
    runtime: &'a CheckRuntime<'a>,
    expectation: &'a SelectedExpectation,
    enforced_scope: &'a mut Vec<String>,
}

fn interrogate_with_full_scope_retry<R: EvaluatorRunner>(
    call: ScopedInterrogation<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_state: &mut InterrogationState,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<InterrogationResult, String> {
    let mut interrogation = interrogate_or_error_record(
        InterrogationCall {
            root: call.root,
            runtime: call.runtime,
            expectation: call.expectation,
            scope: call.enforced_scope,
        },
        runner,
        diagnostic_log,
        interrogation_state,
        scope_hash_cache,
    )?;
    if should_retry_full_scope_after_restricted_idk(&interrogation.record, call.enforced_scope) {
        // `idk` is a non-answer, not a cache-spec "same answer" that can prove
        // a narrower scope. The interrogation policy requires a separate
        // full-scope retry, and that final record replaces the restricted
        // non-answer.
        *call.enforced_scope = full_scope();
        interrogation = interrogate_or_error_record(
            InterrogationCall {
                root: call.root,
                runtime: call.runtime,
                expectation: call.expectation,
                scope: call.enforced_scope,
            },
            runner,
            diagnostic_log,
            interrogation_state,
            scope_hash_cache,
        )?;
    }
    Ok(interrogation)
}

fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_state: &mut InterrogationState,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_response_repairs(
        call.runtime,
        call.expectation,
        runner,
        diagnostic_log,
        interrogation_state,
        call.scope,
    ) {
        Ok(interrogation) => Ok(interrogation),
        Err(err) => Ok(InterrogationResult {
            record: error_record_from_interrogation_error(
                call.root,
                &call.runtime.config.agent,
                call.expectation,
                call.scope,
                &err,
                scope_hash_cache,
            )?,
        }),
    }
}

pub(crate) fn narrowed_scope_is_accepted(wide: &CheckRecord, narrowed: &CheckRecord) -> bool {
    narrowed.observed == wide.observed || (is_verified_record(narrowed) && !narrowed.passed())
}

fn write_scope_narrowing_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    number: usize,
    enforced_scope: &[String],
    record_scope: &[String],
    accepted: bool,
    initial_record: &CheckRecord,
    narrowed_record: &CheckRecord,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer.write_event(
        "info",
        "scope.narrowing",
        &scope_narrowing_log_fields(
            number,
            enforced_scope,
            record_scope,
            accepted,
            initial_record,
            narrowed_record,
        ),
    )
}
