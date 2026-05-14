use crate::check_errors::error_record_from_interrogation_error;
use crate::check_interrogation_state::{
    should_retry_full_scope_after_restricted_idk, CheckRuntime, InterrogationState,
};
use crate::check_model_fallback::interrogate_expectation_with_model_fallbacks;
use crate::check_narrowing::scope_narrowing_log_fields;
use crate::evaluator_turn::is_verified_record;
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;
use crate::scope_hash::ScopeHashCache;
use crate::types::{CheckRecord, EvaluatorRunner, InterrogationResult, SelectedExpectation};
use std::path::Path;

pub(crate) struct InterrogationCall<'a> {
    pub(crate) root: &'a Path,
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) scope: &'a [String],
}

pub(crate) struct ScopedInterrogation<'a> {
    pub(crate) root: &'a Path,
    pub(crate) runtime: &'a CheckRuntime<'a>,
    pub(crate) expectation: &'a SelectedExpectation,
    pub(crate) enforced_scope: &'a mut Vec<String>,
}

pub(crate) fn interrogate_with_full_scope_retry<R: EvaluatorRunner>(
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

pub(crate) fn interrogate_or_error_record<R: EvaluatorRunner>(
    call: InterrogationCall<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    interrogation_state: &mut InterrogationState,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<InterrogationResult, String> {
    match interrogate_expectation_with_model_fallbacks(
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

pub(crate) fn write_scope_narrowing_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    id: &str,
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
            id,
            enforced_scope,
            record_scope,
            accepted,
            initial_record,
            narrowed_record,
        ),
    )
}
