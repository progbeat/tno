use super::*;

#[test]
fn completed_agent_message_text_is_turn_text_fallback() {
    let message = json!({
        "method": "item/completed",
        "params": {
            "item": {
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "ANSWER: yes\n" },
                    { "type": "output_text", "text": "EVIDENCE:\nok\nSCOPE: [\".\"]" }
                ]
            }
        }
    });
    let mut completed_text = String::new();
    append_completed_agent_text(&message, &mut completed_text);
    assert_eq!(
        turn_text(String::new(), completed_text),
        "ANSWER: yes\nEVIDENCE:\nok\nSCOPE: [\".\"]"
    );
    assert_eq!(
        turn_text("ANSWER: no".to_string(), "ANSWER: yes".to_string()),
        "ANSWER: no"
    );
}

#[test]
fn app_server_error_message_is_extracted() {
    let message = json!({
        "method": "error",
        "params": {
            "error": {
                "message": "You've hit your usage limit for GPT-5.3-Codex-Spark."
            }
        }
    });
    assert_eq!(
        app_server_error_message(&message).unwrap(),
        "You've hit your usage limit for GPT-5.3-Codex-Spark."
    );

    let turn_completed = json!({
        "method": "turn/completed",
        "params": {
            "turn": {
                "status": "failed",
                "error": {
                    "message": "model unavailable"
                }
            }
        }
    });
    assert_eq!(
        app_server_error_message(&turn_completed).unwrap(),
        "model unavailable"
    );
}

#[test]
fn evaluator_failure_classification_uses_typed_error() {
    let usage_limit = EvaluatorError::failure(
        EvaluatorFailureKind::UsageLimit,
        "app-server turn/start failed: usageLimitExceeded",
    );
    assert!(is_model_technical_failure(&usage_limit));
    assert!(session_failure_invalidates_thread(&usage_limit));
    assert!(!is_model_technical_failure(&EvaluatorError::message(
        "evaluator response scope [\"context window\"] widens enforced scope [\".\"]"
    )));

    let timeout = EvaluatorError::failure(
        EvaluatorFailureKind::TurnTimeout,
        "app-server turn/run timed out after 300 seconds without progress",
    );
    assert!(is_turn_timeout_failure(&timeout));
    assert!(session_failure_invalidates_thread(&timeout));
    assert_eq!(
        timeout.message_str(),
        "app-server turn/run timed out after 300 seconds without progress"
    );
    let app_server_usage_limit = app_server_failure_from_message(
        "turn/run",
        "You've hit your usage limit for GPT-5.3-Codex-Spark.",
    );
    assert_eq!(
        app_server_usage_limit.kind(),
        Some(EvaluatorFailureKind::UsageLimit)
    );
}

#[test]
fn turn_timeout_resets_after_app_server_progress() {
    let first_activity = Instant::now();
    let before_timeout = first_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS - 1);
    let at_timeout = first_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS);

    assert!(!turn_idle_timed_out(first_activity, before_timeout));
    assert!(turn_idle_timed_out(first_activity, at_timeout));

    let later_activity = at_timeout;
    let later_before_timeout =
        later_activity + Duration::from_secs(APP_SERVER_TURN_TIMEOUT_SECS - 1);
    assert!(!turn_idle_timed_out(later_activity, later_before_timeout));
}

#[test]
fn token_usage_update_is_rendered_like_codex_summary() {
    let message = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "turnId": "turn-1",
            "tokenUsage": {
                "last": {
                    "totalTokens": 69748,
                    "inputTokens": 63574,
                    "cachedInputTokens": 361216,
                    "outputTokens": 6174,
                    "reasoningOutputTokens": 2911
                }
            }
        }
    });
    let (turn_id, usage) = token_usage_update(&message).unwrap();
    assert_eq!(turn_id, "turn-1");
    assert_eq!(
        render_token_usage_summary(usage),
        "Token usage: total=69,748 input=63,574 (+ 361,216 cached) output=6,174 (reasoning 2,911)"
    );
    assert_eq!(
        render_token_usage_summary(TokenUsage::default()),
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)"
    );
}

#[test]
fn long_summary_padding_keeps_required_surrounding_spaces() {
    let inner = " 123456789 failed, 456789 errors, 789123 passed, 101112 skipped in 123456789.00s ";
    assert!(inner.len() >= 80);
    let line = pad_summary_line(inner);
    assert!(line.starts_with("= "));
    assert!(line.ends_with(" ="));
}
