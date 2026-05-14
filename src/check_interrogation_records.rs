use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::evaluator_turn::record_from_response;
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;
use crate::scope::scope_is_within;
use crate::types::{
    CheckRecord, EvaluatorError, InterrogationResult, ObservedAnswerState, ParsedAnswer,
    QueryInterrogationResult, SelectedExpectation,
};
use crate::{
    EMPTY_EVIDENCE_OBSERVED, MALFORMED_REVIEW_WARNING, OBSERVED_IDK, OBSERVED_MALFORMED,
    UNPARSEABLE_OBSERVED,
};
use serde_json::json;

pub(crate) fn finalize_interrogation_response(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    response: ParsedAnswer,
) -> Result<InterrogationResult, EvaluatorError> {
    let mut response = enforce_response_scope(response, enforced_scope);
    response = normalize_empty_evidence_response(response, enforced_scope);
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope.to_vec();
    }
    let record_scope = response.scope.clone();
    let scope_hash = state.scope_hash_cache.staged_scope_hash(
        runtime.root,
        &runtime.config.agent,
        &record_scope,
    )?;
    let record = record_from_response(
        &runtime.config.agent,
        expectation,
        response,
        record_scope,
        scope_hash,
    )?;
    write_review_events(
        diagnostic_log,
        Some(&expectation.id),
        enforced_scope,
        &record,
    )?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_interrogation_record(&record)?;
    }
    Ok(InterrogationResult { record })
}

pub(crate) fn finalize_query_response(
    runtime: &CheckRuntime<'_>,
    question: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    response: ParsedAnswer,
) -> Result<QueryInterrogationResult, EvaluatorError> {
    let enforced_scope = full_scope();
    let mut response = enforce_response_scope(response, &enforced_scope);
    response = normalize_empty_evidence_response(response, &enforced_scope);
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope;
    }
    let scope_hash = state.scope_hash_cache.staged_scope_hash(
        runtime.root,
        &runtime.config.agent,
        &response.scope,
    )?;
    write_parsed_answer_review_events(
        diagnostic_log,
        None,
        &full_scope(),
        &response.answer,
        &response.evidence,
    )?;
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

pub(crate) fn enforce_response_scope(
    response: ParsedAnswer,
    enforced_scope: &[String],
) -> ParsedAnswer {
    if response.answer == UNPARSEABLE_OBSERVED || scope_is_within(&response.scope, enforced_scope) {
        return response;
    }
    if response.answer == OBSERVED_IDK {
        // Reject the evaluator-proposed widened scope, but keep the restricted
        // `idk` non-answer so the caller can perform the interrogation-policy
        // full-scope retry instead of treating the rejected scope as reusable
        // cache narrowing history.
        let evidence = rejected_widened_scope_evidence(&response, enforced_scope);
        return ParsedAnswer {
            answer: response.answer,
            evidence,
            scope: enforced_scope.to_vec(),
        };
    }
    ParsedAnswer {
        answer: UNPARSEABLE_OBSERVED.to_string(),
        evidence: rejected_widened_scope_message(&response.scope, enforced_scope),
        scope: enforced_scope.to_vec(),
    }
}

fn rejected_widened_scope_evidence(response: &ParsedAnswer, enforced_scope: &[String]) -> String {
    let message = rejected_widened_scope_message(&response.scope, enforced_scope);
    if response.evidence.trim().is_empty() {
        message
    } else {
        format!("{}\n{}", response.evidence, message)
    }
}

fn rejected_widened_scope_message(response_scope: &[String], enforced_scope: &[String]) -> String {
    format!(
        "evaluator response scope {:?} widens enforced scope {:?}",
        response_scope, enforced_scope
    )
}

fn normalize_empty_evidence_response(
    response: ParsedAnswer,
    enforced_scope: &[String],
) -> ParsedAnswer {
    if response.evidence.trim().is_empty()
        && response.answer != OBSERVED_MALFORMED
        && response.answer != UNPARSEABLE_OBSERVED
        && !(response.answer == OBSERVED_IDK && enforced_scope != full_scope())
    {
        return ParsedAnswer {
            answer: EMPTY_EVIDENCE_OBSERVED.to_string(),
            evidence: "evaluator response evidence was empty".to_string(),
            scope: response.scope,
        };
    }
    response
}

fn write_review_events(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    record: &CheckRecord,
) -> Result<(), EvaluatorError> {
    write_parsed_answer_review_events(
        diagnostic_log,
        expectation_id,
        enforced_scope,
        &record.observed,
        &record.evidence,
    )
}

pub(crate) fn write_parsed_answer_review_events(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    enforced_scope: &[String],
    observed: &str,
    evidence: &str,
) -> Result<(), EvaluatorError> {
    match ObservedAnswerState::from_observed(observed) {
        ObservedAnswerState::Malformed => {
            write_review_required(diagnostic_log, expectation_id, MALFORMED_REVIEW_WARNING)?;
        }
        ObservedAnswerState::Unparseable => {
            write_review_required(
                diagnostic_log,
                expectation_id,
                "unparseable evaluator response",
            )?;
        }
        ObservedAnswerState::EmptyEvidence => {
            write_review_required(diagnostic_log, expectation_id, "empty evaluator evidence")?;
        }
        ObservedAnswerState::Idk if enforced_scope == full_scope() => {
            write_review_required(diagnostic_log, expectation_id, "full-scope idk")?;
        }
        ObservedAnswerState::Unknown => {
            write_review_required(
                diagnostic_log,
                expectation_id,
                "unknown observed answer state",
            )?;
        }
        ObservedAnswerState::Idk | ObservedAnswerState::Answer => {}
    }
    if evidence.trim().is_empty() {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event("warn", "evidence.empty", &[("id", json!(expectation_id))])?;
        }
    }
    Ok(())
}

fn write_review_required(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    reason: &str,
) -> Result<(), EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "warn",
            "review.required",
            &[("id", json!(expectation_id)), ("reason", json!(reason))],
        )?;
    }
    Ok(())
}
