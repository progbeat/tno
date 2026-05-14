use crate::app_server::{AppServerRunner, LazyAppServerRunner};
use crate::check_validation::codex_reasoning_effort;
use crate::evaluator::{
    evaluator_response_output_schema, evaluator_turn_input, render_evaluator_turn_input,
};
use crate::evaluator_config::evaluator_thread_config;
use crate::evaluator_turn::is_model_technical_failure;
use crate::types::{AgentConfig, EvaluatorError, EvaluatorRunner, TokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

impl LazyAppServerRunner {
    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        let mut total = self.retired_token_usage;
        if let Some(usage) = self.inner.as_ref().and_then(AppServerRunner::token_usage) {
            total = total.add(usage);
        }
        if total.total_tokens == 0 {
            None
        } else {
            Some(total)
        }
    }

    pub(crate) fn drain_token_usage_updates(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates();
        }
    }

    fn retire_inner_after_model_failure(&mut self, err: &EvaluatorError) {
        if !is_model_technical_failure(err) {
            return;
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates();
            if let Some(usage) = inner.token_usage() {
                self.retired_token_usage = self.retired_token_usage.add(usage);
            }
        }
        self.sessions.clear();
        self.inner = None;
    }
}

impl EvaluatorRunner for LazyAppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError> {
        let result = self
            .inner()?
            .start_session(root, instructions, agent, model, thinking, scope);
        match result {
            Ok(session_id) => {
                self.sessions.insert(session_id.clone());
                Ok(session_id)
            }
            Err(err) => {
                self.retire_inner_after_model_failure(&err);
                Err(err)
            }
        }
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError> {
        if !self.sessions.contains(session_id) {
            return Err("app-server runner does not own session".into());
        }
        let result = self
            .inner
            .as_mut()
            .ok_or_else(|| EvaluatorError::message("app-server runner is not initialized"))?
            .ask(session_id, prompt, model, thinking);
        if let Err(err) = &result {
            self.retire_inner_after_model_failure(err);
        }
        result
    }
}

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError> {
        let params = ThreadStartParams {
            cwd: root.display().to_string(),
            developer_instructions: instructions,
            approval_policy: "never",
            config: evaluator_thread_config(agent, scope, model, thinking),
            ephemeral: true,
            session_start_source: "startup",
        };
        let params = serde_json::to_value(params)
            .map_err(|err| format!("failed to encode thread/start params: {}", err))?;
        let result = self.send_request("thread/start", params)?;
        let response: ThreadStartResponse = serde_json::from_value(result)
            .map_err(|err| format!("thread/start response missing thread.id: {}", err))?;
        Ok(response.thread.id)
    }

    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError> {
        let input = evaluator_turn_input(prompt)?;
        let input_text = render_evaluator_turn_input(&input)?;
        let mut request = json!({
            "threadId": session_id,
            "input": [
                {
                    "type": "text",
                    "text": input_text
                }
            ]
        });
        if let Some(model) = model {
            request["model"] = Value::String(model.to_string());
        }
        if let Some(effort) = codex_reasoning_effort(thinking) {
            request["effort"] = Value::String(effort.to_string());
        }
        request["outputSchema"] = evaluator_response_output_schema();
        self.send_turn_request("turn/start", request)
    }
}

#[derive(Serialize)]
struct ThreadStartParams<'a> {
    cwd: String,
    #[serde(rename = "developerInstructions")]
    developer_instructions: &'a str,
    #[serde(rename = "approvalPolicy")]
    approval_policy: &'a str,
    config: Value,
    ephemeral: bool,
    #[serde(rename = "sessionStartSource")]
    session_start_source: &'a str,
}

#[derive(Deserialize)]
struct ThreadStartResponse {
    thread: ThreadStartThread,
}

#[derive(Deserialize)]
struct ThreadStartThread {
    id: String,
}
