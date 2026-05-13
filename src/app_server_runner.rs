use crate::*;

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
        let result = self.send_request(
            "thread/start",
            json!({
                "cwd": root.display().to_string(),
                "developerInstructions": instructions,
                "approvalPolicy": "never",
                "config": evaluator_thread_config(agent, scope, model, thinking),
                "ephemeral": true,
                "sessionStartSource": "startup"
            }),
        )?;
        result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| EvaluatorError::message("thread/start response missing thread.id"))
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
