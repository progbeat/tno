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
    let mut skipped = 0usize;
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
    for expectation in &options.selected {
        // Each branch that produces a non-skipped CheckRecord writes and
        // flushes that record before moving to the next expectation. Reused
        // passing cache hits are the only intentionally silent per-expectation
        // results and are counted as skipped.
        if check_interrupted() {
            return Err(check_run_error(
                &records,
                skipped,
                narrowing,
                "interrupted".to_string(),
            ));
        }
        if !options.ignore_cache {
            if let Some(hit) = cached_record_for_expectation(
                root,
                &config.agent,
                expectation,
                &mut history_cache,
                &mut scope_hash_cache,
            )
            .map_err(|err| check_run_error(&records, skipped, narrowing, err))?
            {
                let should_stop = options.fail_fast && !hit.record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    write_cache_hit(writer, &hit)
                        .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
                }
                if hit.record.passed() {
                    // Reused passing records are skipped: they affect only the
                    // final summary and intentionally emit no per-expectation stdout.
                    skipped += 1;
                } else {
                    write_and_flush_result_output(&mut result_output, &hit.record)
                        .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
                }
                records.push(hit.record);
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

        // `--ignore-cache` bypasses reusable answer records in the branch above,
        // but it does not erase the interrogation-policy scope seed: a fresh
        // evaluator turn still starts from the latest reusable history scope.
        let mut enforced_scope =
            latest_history_scope_with_cache(root, &config.agent, expectation, &mut history_cache)
                .map_err(|err| check_run_error(&records, skipped, narrowing, err))?
                .unwrap_or_else(full_scope);
        // Response-format problems are handled inside this call: malformed,
        // unparseable, and empty-evidence evaluator responses get their one
        // same-interrogation retry; response parsing rejects extra JSON keys,
        // non-single-line answers, and non-normalized scope entries before
        // finalization can return a human-review record as Ok(...). Err here
        // means a technical runner/model/logging failure.
        let mut interrogation = match interrogate_expectation_with_response_repairs(
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
                )
                .map_err(|err| check_run_error(&records, skipped, narrowing, err))?,
            },
        };
        if should_retry_full_scope_after_restricted_idk(&interrogation.record, &enforced_scope) {
            // `idk` is a non-answer, not a cache-spec "same answer" that can
            // prove a narrower scope. The interrogation policy requires a
            // separate full-scope retry, and that final record replaces the
            // restricted non-answer.
            enforced_scope = full_scope();
            interrogation = match interrogate_expectation_with_response_repairs(
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
                    )
                    .map_err(|err| check_run_error(&records, skipped, narrowing, err))?,
                },
            };
        }

        let record_scope = interrogation.record.scope.clone();
        let mut write_reusable_history = true;
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
            // only if an independent interrogation with that same canonical
            // scope preserves the answer.
            let initial_record = interrogation.record.clone();
            let narrowed = match interrogate_expectation_with_response_repairs(
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
                    )
                    .map_err(|err| check_run_error(&records, skipped, narrowing, err))?,
                },
            };
            if narrowed.record.observed == interrogation.record.observed {
                narrowing.accepted += 1;
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer
                        .write_event(
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
                        )
                        .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
                }
                interrogation = narrowed;
            } else {
                narrowing.rejected += 1;
                write_reusable_history = false;
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer
                        .write_event(
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
                        )
                        .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
                }
                let enforced_scope_hash = scope_hash_cache
                    .staged_scope_hash(root, &config.agent, &enforced_scope)
                    .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
                interrogation.record.scope = enforced_scope.clone();
                interrogation.record.scope_hash = enforced_scope_hash;
            }
        }

        // Only verified yes/no/option answer records are reusable. Human-review
        // states such as idk, malformed, rejected widened scopes, and rejected
        // narrowing attempts are not written to history.
        if write_reusable_history && is_verified_record(&interrogation.record) {
            append_history_record_with_cache(
                root,
                expectation,
                &interrogation.record,
                &mut history_cache,
            )
            .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
        }
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer
                .write_record(&interrogation.record)
                .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
        }
        let should_stop = options.fail_fast && !interrogation.record.passed();
        write_and_flush_result_output(&mut result_output, &interrogation.record)
            .map_err(|err| check_run_error(&records, skipped, narrowing, err))?;
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
