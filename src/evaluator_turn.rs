use crate::evaluator_response_cache::{response_excerpt, EvaluatorResponseParseCache};
use crate::hash::full_scope;
use crate::history_cache_key::history_cache_key;
use crate::history_reuse::is_reusable_history_record;
use crate::logging::DiagnosticLogWriter;
use crate::time::{format_log_record_timestamp, unix_timestamp};
use crate::types::{
    AgentConfig, CheckRecord, CheckResult, EvaluatorError, EvaluatorRunner, ObservedAnswerState,
    ParsedAnswer, SelectedExpectation, TokenUsage,
};
use crate::{EMPTY_EVIDENCE_OBSERVED, OBSERVED_IDK, OBSERVED_MALFORMED, UNPARSEABLE_OBSERVED};
use serde::Serialize;
use serde_json::{json, Value};

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

pub(crate) fn is_model_technical_failure(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::is_model_technical)
}

pub(crate) fn is_context_window_failure(err: &EvaluatorError) -> bool {
    err.kind() == Some(EvaluatorFailureKind::ContextWindow)
}

pub(crate) fn session_failure_invalidates_thread(err: &EvaluatorError) -> bool {
    err.kind()
        .is_some_and(EvaluatorFailureKind::invalidates_thread)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EvaluatorFailureKind {
    UsageLimit,
    RateLimit,
    ModelUnavailable,
    TurnTimeout,
    ContextWindow,
    UnknownAppServer,
}

impl EvaluatorFailureKind {
    pub(crate) fn is_model_technical(self) -> bool {
        matches!(
            self,
            EvaluatorFailureKind::UsageLimit
                | EvaluatorFailureKind::RateLimit
                | EvaluatorFailureKind::ModelUnavailable
                | EvaluatorFailureKind::TurnTimeout
                | EvaluatorFailureKind::ContextWindow
                | EvaluatorFailureKind::UnknownAppServer
        )
    }

    pub(crate) fn invalidates_thread(self) -> bool {
        self.is_model_technical()
    }
}

pub(crate) fn record_from_response(
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    let requires_human_review =
        ObservedAnswerState::from_observed(&response.answer).requires_human_review();
    let result = if !requires_human_review && response.answer == expectation.a {
        CheckResult::Pass
    } else {
        CheckResult::Fail
    };
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
        id: expectation.id.clone(),
        display_id: expectation.display_id.clone(),
        number: expectation.number,
        result,
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: response.answer,
        evidence: response.evidence,
        scope: enforced_scope,
        scope_hash,
        cache_key: Some(history_cache_key(agent, expectation)),
    })
}

pub(crate) fn is_verified_record(record: &CheckRecord) -> bool {
    is_reusable_history_record(record)
}

// This module owns one evaluator turn: model labels, response parsing, and
// record finalization. It is intentionally not sufficient by itself to judge
// the full interrogation policy; with only this file visible, full-policy
// questions require `idk` rather than a pass/fail conclusion. Cross-turn policy
// such as restricted `idk` full-scope retries, narrowing verification, and
// cache orchestration is in `check.rs`, `check_interrogation_records.rs`, and
// `check_cache.rs`.
pub(crate) struct EvaluatorTurnContext<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
}

pub(crate) fn ask_once<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
) -> Result<ParsedAnswer, EvaluatorError> {
    let response = ask_and_log(
        runner,
        turn,
        prompt,
        diagnostic_log,
        expectation_id,
        1,
        "initial",
    )?;
    let mut parsed = match parser_cache.parse(&response, agent) {
        Ok(answer) => answer,
        Err(err) => ParsedAnswer {
            answer: UNPARSEABLE_OBSERVED.to_string(),
            evidence: format!(
                "evaluator response could not be parsed: {}\nresponse: {}",
                err,
                response_excerpt(&response)
            ),
            scope: full_scope(),
        },
    };

    if parsed.evidence.trim().is_empty()
        && parsed.answer != OBSERVED_IDK
        && parsed.answer != OBSERVED_MALFORMED
        && parsed.answer != UNPARSEABLE_OBSERVED
    {
        parsed = ParsedAnswer {
            answer: EMPTY_EVIDENCE_OBSERVED.to_string(),
            evidence: "evaluator response evidence was empty".to_string(),
            scope: parsed.scope,
        };
    }

    Ok(parsed)
}

pub(crate) fn ask_and_log<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_id: Option<&str>,
    attempt: usize,
    reason: &str,
) -> Result<String, EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_request = serde_json::to_value(EvaluatorTurnLogRequest {
            session_id: turn.session_id,
            prompt,
            model: turn.model,
            thinking: turn.thinking,
        })
        .map_err(|err| format!("failed to encode evaluator turn request log: {}", err))?;
        writer.write_event(
            "info",
            "agent.request",
            &[
                ("id", json!(expectation_id)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("raw", json!(prompt)),
                ("rawRequest", raw_request.clone()),
                ("request", raw_request),
            ],
        )?;
    }
    let response = runner.ask(turn.session_id, prompt, turn.model, turn.thinking)?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        let raw_response = json!({
            "sessionId": turn.session_id,
            "text": response.clone(),
        });
        writer.write_event(
            "info",
            "agent.response",
            &[
                ("id", json!(expectation_id)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("raw", json!(response.clone())),
                ("rawResponse", json!(response.clone())),
                ("response", raw_response),
            ],
        )?;
    }
    Ok(response)
}

#[derive(Serialize)]
struct EvaluatorTurnLogRequest<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    prompt: &'a str,
    model: Option<&'a str>,
    thinking: &'a str,
}
