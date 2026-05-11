use crate::*;

pub(crate) fn turn_idle_timed_out(last_activity: Instant, now: Instant) -> bool {
    now.duration_since(last_activity) >= Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS)
}

pub(crate) fn app_server_failure_kind(error: &Value) -> Option<EvaluatorFailureKind> {
    let code = error
        .get("code")
        .or_else(|| error.get("type"))
        .and_then(Value::as_str);
    if let Some(kind) = code.and_then(app_server_failure_kind_from_code) {
        return Some(kind);
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())?;
    app_server_failure_kind_from_message(message)
}

pub(crate) fn app_server_failure_from_value(method: &str, error: &Value) -> EvaluatorError {
    let failure = format!("app-server {} failed: {}", method, error);
    match app_server_failure_kind(error) {
        Some(kind) => EvaluatorError::failure(kind, failure),
        None => EvaluatorError::message(failure),
    }
}

pub(crate) fn app_server_failure_from_message(method: &str, message: &str) -> EvaluatorError {
    let failure = format!("app-server {} failed: {}", method, message);
    match app_server_failure_kind_from_message(message) {
        Some(kind) => EvaluatorError::failure(kind, failure),
        None => EvaluatorError::message(failure),
    }
}

pub(crate) fn app_server_failure_kind_from_code(code: &str) -> Option<EvaluatorFailureKind> {
    match code {
        "usageLimitExceeded" | "usage_limit_exceeded" => Some(EvaluatorFailureKind::UsageLimit),
        "rateLimitExceeded" | "rate_limit_exceeded" => Some(EvaluatorFailureKind::RateLimit),
        "modelUnavailable" | "model_unavailable" => Some(EvaluatorFailureKind::ModelUnavailable),
        "contextWindowExceeded" | "context_window_exceeded" | "context_length_exceeded" => {
            Some(EvaluatorFailureKind::ContextWindow)
        }
        _ => None,
    }
}

pub(crate) fn app_server_failure_kind_from_message(message: &str) -> Option<EvaluatorFailureKind> {
    if message.contains("usageLimitExceeded") || message.contains("usage limit") {
        return Some(EvaluatorFailureKind::UsageLimit);
    }
    if message.contains("rate limit") {
        return Some(EvaluatorFailureKind::RateLimit);
    }
    if message.contains("model unavailable") || message.contains("model is unavailable") {
        return Some(EvaluatorFailureKind::ModelUnavailable);
    }
    if message.contains("context window") || message.contains("ran out of room") {
        return Some(EvaluatorFailureKind::ContextWindow);
    }
    None
}

impl TokenUsage {
    pub(crate) fn add(self, other: TokenUsage) -> TokenUsage {
        TokenUsage {
            total_tokens: self.total_tokens + other.total_tokens,
            input_tokens: self.input_tokens + other.input_tokens,
            cached_input_tokens: self.cached_input_tokens + other.cached_input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens + other.reasoning_output_tokens,
        }
    }
}

pub(crate) fn token_usage_update(message: &Value) -> Option<(String, TokenUsage)> {
    if message.get("method").and_then(Value::as_str) != Some("thread/tokenUsage/updated") {
        return None;
    }
    let params = message.get("params")?;
    let turn_id = params.get("turnId").and_then(Value::as_str)?.to_string();
    let usage = params.get("tokenUsage")?.get("last")?;
    Some((turn_id, parse_token_usage(usage)?))
}

pub(crate) fn turn_started_id(message: &Value) -> Option<String> {
    if message.get("method").and_then(Value::as_str) != Some("turn/started") {
        return None;
    }
    message
        .get("params")?
        .get("turn")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn parse_token_usage(value: &Value) -> Option<TokenUsage> {
    Some(TokenUsage {
        total_tokens: value.get("totalTokens").and_then(Value::as_u64)?,
        input_tokens: value.get("inputTokens").and_then(Value::as_u64)?,
        cached_input_tokens: value
            .get("cachedInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value.get("outputTokens").and_then(Value::as_u64)?,
        reasoning_output_tokens: value
            .get("reasoningOutputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "Token usage: total={} input={} (+ {} cached) output={} (reasoning {})",
        format_number(usage.total_tokens),
        format_number(usage.input_tokens),
        format_number(usage.cached_input_tokens),
        format_number(usage.output_tokens),
        format_number(usage.reasoning_output_tokens)
    )
}

pub(crate) fn format_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            output.push(',');
        }
        output.push(ch);
    }
    output.chars().rev().collect()
}

pub(crate) fn app_server_error_message(message: &Value) -> Option<String> {
    let method = message.get("method").and_then(Value::as_str)?;
    if method != "error" && method != "turn/failed" && method != "turn/error" {
        if method == "turn/completed"
            && string_at_path(message, &["params", "turn", "status"]) == Some("failed")
        {
            return string_at_path(message, &["params", "turn", "error", "message"])
                .or_else(|| string_at_path(message, &["params", "turn", "error"]))
                .map(str::to_string)
                .or_else(|| Some("turn failed".to_string()));
        }
        return None;
    }
    string_at_path(message, &["params", "error", "message"])
        .or_else(|| string_at_path(message, &["params", "message"]))
        .or_else(|| string_at_path(message, &["params", "error", "codexErrorInfo"]))
        .or_else(|| string_at_path(message, &["error", "message"]))
        .or_else(|| string_at_path(message, &["message"]))
        .or_else(|| string_at_path(message, &["params", "error"]))
        .map(str::to_string)
        .or_else(|| Some(method.to_string()))
}

pub(crate) fn string_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

pub(crate) fn turn_text(delta_text: String, completed_text: String) -> String {
    if delta_text.trim().is_empty() {
        completed_text
    } else {
        delta_text
    }
}

pub(crate) fn append_completed_agent_text(message: &Value, output: &mut String) {
    let Some(params) = message.get("params") else {
        return;
    };
    if let Some(item) = params.get("item") {
        if is_assistant_message_item(item) {
            append_text_fields(item, output);
        }
    } else if message.get("method").and_then(Value::as_str) == Some("item/agentMessage/completed") {
        append_text_fields(params, output);
    }
}

pub(crate) fn is_assistant_message_item(item: &Value) -> bool {
    item.get("role").and_then(Value::as_str) == Some("assistant")
        || item
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| kind.contains("agent") && kind.contains("message"))
            .unwrap_or(false)
}

pub(crate) fn append_text_fields(value: &Value, output: &mut String) {
    match value {
        Value::Array(items) => {
            for item in items {
                append_text_fields(item, output);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "text" {
                    if let Some(text) = value.as_str() {
                        output.push_str(text);
                    }
                } else {
                    append_text_fields(value, output);
                }
            }
        }
        _ => {}
    }
}
