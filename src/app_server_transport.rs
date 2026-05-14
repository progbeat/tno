use crate::app_server::AppServerRunner;
use crate::app_server_protocol::{
    app_server_error_value, app_server_failure_from_value, app_server_message,
    append_completed_agent_text, token_usage_update, turn_idle_timed_out, turn_started_id,
    turn_text,
};
use crate::check_preflight::check_interrupted;
use crate::evaluator_turn::EvaluatorFailureKind;
use crate::types::{EvaluatorError, TokenUsage};
use crate::APP_SERVER_TURN_TIMEOUT_SECS;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

impl AppServerRunner {
    pub(crate) fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, EvaluatorError> {
        let id = self.send_json_rpc_request(method, &params, "request")?;
        loop {
            if check_interrupted() {
                return Err("interrupted".into());
            }
            let message = self.read_message()?;
            self.record_token_usage(&message);
            let envelope = app_server_message(&message).map_err(app_server_protocol_error)?;
            if envelope.id == Some(id) {
                if let Some(error) = envelope.error.as_ref() {
                    return Err(app_server_failure_from_value(method, error));
                }
                return envelope.result.ok_or_else(|| {
                    EvaluatorError::message(format!(
                        "app-server {} response missing result",
                        method
                    ))
                });
            }
        }
    }

    pub(crate) fn send_turn_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<String, EvaluatorError> {
        let id = self.send_json_rpc_request(method, &params, "request")?;

        let mut saw_response = false;
        let mut saw_completed = false;
        let mut text = String::new();
        let mut completed_text = String::new();
        let thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut last_activity = Instant::now();
        let mut turn_id: Option<String> = None;
        let mut pending_error: Option<Value> = None;
        let mut interrupted = false;
        let mut interrupt_sent = false;
        loop {
            self.maybe_interrupt_turn(
                &mut interrupted,
                &mut interrupt_sent,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            let Some(message) = self.read_message_or_timeout()? else {
                if turn_idle_timed_out(last_activity, Instant::now()) {
                    return Err(EvaluatorError::failure(
                        EvaluatorFailureKind::TurnTimeout,
                        format!(
                            "app-server {} timed out after {} seconds without progress",
                            method, APP_SERVER_TURN_TIMEOUT_SECS
                        ),
                    ));
                }
                continue;
            };
            last_activity = Instant::now();
            self.record_token_usage(&message);
            let envelope = app_server_message(&message).map_err(app_server_protocol_error)?;
            if let Some(started_turn_id) = turn_started_id(&message) {
                turn_id = Some(started_turn_id);
                self.maybe_interrupt_turn(
                    &mut interrupted,
                    &mut interrupt_sent,
                    thread_id.as_deref(),
                    turn_id.as_deref(),
                )?;
            }
            if envelope.id == Some(id) {
                if let Some(error) = envelope.error.as_ref() {
                    return Err(app_server_failure_from_value(method, error));
                }
                saw_response = true;
                if saw_completed {
                    return Ok(turn_text(text, completed_text));
                }
                continue;
            }
            match envelope.method.as_deref() {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = message
                        .get("params")
                        .and_then(|params| params.get("delta"))
                        .and_then(Value::as_str)
                    {
                        text.push_str(delta);
                    }
                }
                Some("item/completed") | Some("item/agentMessage/completed") => {
                    append_completed_agent_text(&message, &mut completed_text);
                }
                Some("turn/completed") => {
                    if interrupted {
                        return Err("interrupted".into());
                    }
                    if let Some(error) =
                        app_server_error_value(&message).or_else(|| pending_error.take())
                    {
                        return Err(app_server_failure_from_value(method, &error));
                    }
                    saw_completed = true;
                    if saw_response {
                        return Ok(turn_text(text, completed_text));
                    }
                }
                Some("error") => {
                    if let Some(error) = app_server_error_value(&message) {
                        pending_error = Some(error);
                    }
                }
                Some(_) => {
                    if let Some(error) = app_server_error_value(&message) {
                        return Err(app_server_failure_from_value(method, &error));
                    }
                }
                _ => {}
            }
        }
    }

    fn maybe_interrupt_turn(
        &mut self,
        interrupted: &mut bool,
        interrupt_sent: &mut bool,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(), EvaluatorError> {
        if !check_interrupted() {
            return Ok(());
        }
        *interrupted = true;
        if *interrupt_sent {
            return Ok(());
        }
        let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
            return Ok(());
        };
        self.send_turn_interrupt(thread_id, turn_id)?;
        *interrupt_sent = true;
        Ok(())
    }

    fn send_turn_interrupt(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), EvaluatorError> {
        let params = json!({
            "threadId": thread_id,
            "turnId": turn_id
        });
        self.send_json_rpc_request("turn/interrupt", &params, "interrupt")?;
        Ok(())
    }

    fn send_json_rpc_request(
        &mut self,
        method: &str,
        params: &Value,
        operation: &str,
    ) -> Result<u64, EvaluatorError> {
        if check_interrupted() {
            return Err("interrupted".into());
        }
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server {}: {}", operation, err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server {}: {}", operation, err))?;
        Ok(id)
    }

    fn read_message(&mut self) -> Result<Value, EvaluatorError> {
        loop {
            match self.read_message_or_timeout()? {
                Some(message) => return Ok(message),
                None if check_interrupted() => return Err("interrupted".into()),
                None => {}
            }
        }
    }

    fn read_message_or_timeout(&mut self) -> Result<Option<Value>, EvaluatorError> {
        match self.messages.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => result.map(Some).map_err(EvaluatorError::message),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if check_interrupted() {
                    Err("interrupted".into())
                } else {
                    Err("app-server closed stdout".into())
                }
            }
        }
    }

    fn record_token_usage(&mut self, message: &Value) {
        let Some((turn_id, usage)) = token_usage_update(message) else {
            return;
        };
        self.token_usage_by_turn.insert(turn_id, usage);
    }

    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        let mut usage = TokenUsage::default();
        for turn_usage in self.token_usage_by_turn.values() {
            usage = usage.add(*turn_usage);
        }
        if usage.total_tokens == 0 {
            None
        } else {
            Some(usage)
        }
    }

    pub(crate) fn drain_token_usage_updates(&mut self) {
        loop {
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(message)) => self.record_token_usage(&message),
                Ok(Err(_)) => return,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn app_server_protocol_error(error: String) -> EvaluatorError {
    EvaluatorError::failure(EvaluatorFailureKind::UnknownAppServer, error)
}
