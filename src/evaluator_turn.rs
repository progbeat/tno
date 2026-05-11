use crate::*;

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

pub(crate) fn is_turn_timeout_failure(err: &EvaluatorError) -> bool {
    err.kind() == Some(EvaluatorFailureKind::TurnTimeout)
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
        )
    }

    pub(crate) fn invalidates_thread(self) -> bool {
        matches!(
            self,
            EvaluatorFailureKind::TurnTimeout | EvaluatorFailureKind::ContextWindow
        )
    }
}

pub(crate) fn record_from_response(
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    let requires_human_review = response.answer == OBSERVED_MALFORMED
        || response.answer == UNPARSEABLE_OBSERVED
        || response.answer == OBSERVED_IDK;
    let result = if !requires_human_review && response.answer == expectation.a {
        CheckResult::Pass
    } else {
        CheckResult::Fail
    };
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
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

// This module owns one evaluator turn: model labels, response repair, and
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

pub(crate) fn ask_with_repairs<R: EvaluatorRunner>(
    runner: &mut R,
    turn: &EvaluatorTurnContext<'_>,
    prompt: &str,
    agent: &AgentConfig,
    parser_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    expectation_number: usize,
) -> Result<ParsedAnswer, EvaluatorError> {
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

    if parsed.evidence.trim().is_empty()
        && parsed.answer != OBSERVED_MALFORMED
        && parsed.answer != UNPARSEABLE_OBSERVED
    {
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
) -> Result<String, EvaluatorError> {
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "agent.request",
            &[
                ("number", json!(expectation_number)),
                ("attempt", json!(attempt)),
                ("reason", json!(reason)),
                ("raw", json!(prompt)),
                (
                    "request",
                    json!({
                        "sessionId": turn.session_id,
                        "prompt": prompt,
                        "model": model_label(turn.model),
                        "thinking": turn.thinking,
                    }),
                ),
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
                (
                    "response",
                    json!({
                        "sessionId": turn.session_id,
                        "raw": response.clone(),
                    }),
                ),
            ],
        )?;
    }
    Ok(response)
}
