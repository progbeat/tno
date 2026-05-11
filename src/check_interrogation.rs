use crate::*;

pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, EvaluatorError> {
    let config = runtime.config;
    let enforced_scope = sanitize_scope(enforced_scope, &config.agent)?;
    // Threads are reused for the same canonical enforced scope. The model is
    // supplied on each turn, so fallback attempts can continue the same thread.
    let session_key = evaluator_session_key(&enforced_scope);
    let existing_session = state.sessions_by_scope.get(&session_key).cloned();
    let had_existing_session = existing_session.is_some();
    let mut session_id = match existing_session {
        Some(existing) => {
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                let developer_instructions = state
                    .session_instructions
                    .get(&existing)
                    .cloned()
                    .unwrap_or_else(|| developer_instructions(&config.agent, &enforced_scope));
                writer.write_event(
                    "info",
                    "thread.reuse",
                    &[
                        ("threadId", json!(existing.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            existing
        }
        None => {
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            let created = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                effective_thinking(&config.agent, expectation),
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(created.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            state
                .session_instructions
                .insert(created.clone(), developer_instructions);
            state
                .sessions_by_scope
                .insert(session_key.clone(), created.clone());
            created
        }
    };
    let prompt = expectation.q.clone();
    let thinking = effective_thinking(&config.agent, expectation);
    let turn = EvaluatorTurnContext {
        session_id: &session_id,
        model,
        thinking,
    };
    let response = match ask_with_repairs(
        runner,
        &turn,
        &prompt,
        &config.agent,
        &mut state.parse_cache,
        diagnostic_log,
        expectation.number,
    ) {
        Ok(response) => response,
        Err(err) if had_existing_session && is_context_window_failure(&err) => {
            if let Some(removed) = state.sessions_by_scope.remove(&session_key) {
                state.session_instructions.remove(&removed);
            }
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "warn",
                    "model.failure",
                    &[
                        ("number", json!(expectation.number)),
                        ("model", json!(model_label(model))),
                        ("error", json!(err.message_str())),
                    ],
                )?;
                writer.write_event(
                    "warn",
                    "thread.restart",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("number", json!(expectation.number)),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        ("reason", json!(err.message_str())),
                    ],
                )?;
            }
            let developer_instructions = developer_instructions(&config.agent, &enforced_scope);
            session_id = runner.start_session(
                runtime.snapshot_root,
                &developer_instructions,
                &config.agent,
                model,
                effective_thinking(&config.agent, expectation),
                &enforced_scope,
            )?;
            if let Some(writer) = diagnostic_log.as_deref_mut() {
                writer.write_event(
                    "info",
                    "thread.start",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("scope", json!(enforced_scope)),
                        ("model", json!(model_label(model))),
                        (
                            "thinking",
                            json!(effective_thinking(&config.agent, expectation)),
                        ),
                        ("developerInstructions", json!(developer_instructions)),
                    ],
                )?;
            }
            state
                .session_instructions
                .insert(session_id.clone(), developer_instructions);
            let turn = EvaluatorTurnContext {
                session_id: &session_id,
                model,
                thinking,
            };
            match ask_with_repairs(
                runner,
                &turn,
                &prompt,
                &config.agent,
                &mut state.parse_cache,
                diagnostic_log,
                expectation.number,
            ) {
                Ok(response) => response,
                Err(err) => {
                    if session_failure_invalidates_thread(&err) {
                        if let Some(removed) = state.sessions_by_scope.remove(&session_key) {
                            state.session_instructions.remove(&removed);
                        }
                    }
                    return Err(err);
                }
            }
        }
        Err(err) => {
            if session_failure_invalidates_thread(&err) {
                if let Some(removed) = state.sessions_by_scope.remove(&session_key) {
                    state.session_instructions.remove(&removed);
                }
            }
            return Err(err);
        }
    };
    state
        .sessions_by_scope
        .insert(session_key, session_id.clone());
    finalize_interrogation_response(
        runtime,
        expectation,
        diagnostic_log,
        state,
        &enforced_scope,
        response,
    )
}
