use crate::evaluator_json::validate_evaluator_response_key_order;
use crate::evaluator_scope::parse_scope_strings;
use crate::types::{AgentConfig, EvaluatorResponseJson, ParsedAnswer};
use crate::{OBSERVED_IDK, OBSERVED_MALFORMED};

pub(crate) fn parse_evaluator_response(
    text: &str,
    agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = parse_evaluator_response_json(text)?;
    if response.answer.contains('\n') || response.answer.contains('\r') {
        return Err("answer must be a single-line string".to_string());
    }
    if !evaluator_answer_is_allowed(&response.answer) {
        return Err(
            "answer must be exactly yes, no, idk, malformed, or one lowercase ASCII option letter"
                .to_string(),
        );
    }
    Ok(ParsedAnswer {
        answer: response.answer,
        evidence: response.evidence,
        scope: parse_scope_strings(&response.scope, agent)?,
    })
}

pub(crate) fn evaluator_answer_is_allowed(answer: &str) -> bool {
    matches!(answer, "yes" | "no" | OBSERVED_IDK | OBSERVED_MALFORMED)
        || matches!(answer.as_bytes(), [letter] if letter.is_ascii_lowercase())
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
