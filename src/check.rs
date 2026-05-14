use crate::check_cache::{
    cached_failure_for_expectation, final_selected_after_current_pass_cache, write_cache_hit,
};
use crate::check_interrogation_policy::{
    interrogate_or_error_record, interrogate_with_full_scope_retry, narrowed_scope_is_accepted,
    write_scope_narrowing_event, InterrogationCall, ScopedInterrogation,
};
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_order_state::write_latest_non_pass_record;
use crate::check_output::{record_requires_human_review, write_and_flush_result_output};
use crate::check_preflight::check_interrupted;
use crate::check_selection::{
    final_selected_expectations, initial_non_selected_expectations,
    order_expectations_by_latest_non_pass,
};
use crate::evaluator_turn::is_verified_record;
use crate::hash::full_scope;
use crate::history::HistoryCache;
use crate::history_append::append_history_record_with_cache;
use crate::history_reuse::latest_history_scope_with_cache;
use crate::logging::DiagnosticLogWriter;
use crate::scope::{is_strict_scope_subset, scope_is_within};
use crate::scope_hash::ScopeHashCache;
use crate::time::unix_timestamp;
use crate::types::{
    check_run_error, CheckConfig, CheckOptions, CheckRunError, CheckRunReport, EvaluatorRunner,
    NarrowingStats,
};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

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
    let mut non_selected = match initial_non_selected_expectations(config, &options.selected) {
        Ok(non_selected) => non_selected,
        Err(err) => {
            return Err(check_run_error(
                &records,
                &[],
                selected,
                skipped,
                silent,
                narrowing,
                err,
            ));
        }
    };
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
                check_run_error(
                    &records,
                    &non_selected,
                    selected,
                    skipped,
                    silent,
                    narrowing,
                    err,
                )
            })?
        };
    }
    let final_selection = match final_selected_expectations(
        root,
        &config.agent,
        options.selected.clone(),
        &mut history_cache,
        run_try!(unix_timestamp()),
    ) {
        Ok(final_selection) => final_selection,
        Err(err) => {
            let skipped_now = err.skipped.len();
            non_selected.extend(err.skipped);
            skipped += skipped_now;
            silent += skipped_now;
            selected = selected.saturating_sub(skipped_now);
            return Err(check_run_error(
                &records,
                &non_selected,
                selected,
                skipped,
                silent,
                narrowing,
                err.error,
            ));
        }
    };
    let final_selected_ids = final_selection
        .selected
        .iter()
        .map(|expectation| expectation.id.clone())
        .collect::<BTreeSet<_>>();
    for expectation in &options.selected {
        if !final_selected_ids.contains(&expectation.id) {
            non_selected.push(expectation.clone());
        }
    }
    skipped += final_selection.skipped.len();
    silent += final_selection.skipped.len();
    let final_selected = if options.ignore_cache {
        final_selection.selected
    } else {
        // Passing exact-cache hits satisfy candidates before the
        // selected-expectation loop below. They are final-selection
        // deselections, so they contribute to skipped/silent and are not
        // selected checks. Failed exact-cache hits stay selected and are
        // reported below.
        let cache_selection = run_try!(final_selected_after_current_pass_cache(
            root,
            &config.agent,
            final_selection.selected,
            &mut history_cache,
            &mut scope_hash_cache,
        ));
        for (expectation, hit) in cache_selection.skipped_passes {
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                run_try!(write_cache_hit(writer, &hit));
            }
            non_selected.push(expectation);
            skipped += 1;
            silent += 1;
        }
        cache_selection.selected
    };
    let final_selected = run_try!(order_expectations_by_latest_non_pass(
        root,
        final_selected,
        &mut history_cache
    ));
    selected = final_selected.len();
    let stop_after_non_pass = !options.check_all;
    for expectation in &final_selected {
        // Each branch that produces a selected CheckRecord writes and flushes
        // that record before moving to the next expectation. Silent passing
        // cache hits have already been removed from `final_selected`.
        if check_interrupted() {
            return Err(check_run_error(
                &records,
                &non_selected,
                selected,
                skipped,
                silent,
                narrowing,
                "interrupted".to_string(),
            ));
        }
        if !options.ignore_cache {
            if let Some(hit) = run_try!(cached_failure_for_expectation(
                root,
                &config.agent,
                expectation,
                &mut history_cache,
                &mut scope_hash_cache,
            )) {
                let should_stop = stop_after_non_pass && !hit.record.passed();
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    run_try!(write_cache_hit(writer, &hit));
                }
                run_try!(write_and_flush_result_output(
                    &mut result_output,
                    &hit.record
                ));
                records.push(hit.record);
                if should_stop {
                    return Ok(CheckRunReport {
                        records,
                        non_selected,
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
        // evaluator turn still starts from the latest answer-history scope.
        let mut enforced_scope = run_try!(latest_history_scope_with_cache(
            root,
            &config.agent,
            expectation,
            &mut history_cache
        ))
        .unwrap_or_else(full_scope);
        // Response-format problems are handled inside this call: malformed,
        // unparseable, and empty-evidence evaluator responses become
        // human-review records. Response parsing rejects extra JSON keys,
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
        // cache records and are handled by the review-required/idk policy above.
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
                    &expectation.id,
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
                    &expectation.id,
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
        run_try!(write_latest_non_pass_record(
            root,
            expectation,
            &interrogation.record
        ));
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            run_try!(writer.write_record(&interrogation.record));
        }
        let should_stop = stop_after_non_pass && !interrogation.record.passed();
        run_try!(write_and_flush_result_output(
            &mut result_output,
            &interrogation.record
        ));
        records.push(interrogation.record);
        if should_stop {
            return Ok(CheckRunReport {
                records,
                non_selected,
                selected,
                skipped,
                silent,
                narrowing,
            });
        }
    }
    Ok(CheckRunReport {
        records,
        non_selected,
        selected,
        skipped,
        silent,
        narrowing,
    })
}
