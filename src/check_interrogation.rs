use crate::*;

#[derive(Clone, Copy)]
pub(crate) struct ThreadTurnRequest<'a> {
    pub(crate) enforced_scope: &'a [String],
    pub(crate) model: Option<&'a str>,
    pub(crate) thinking: &'a str,
    pub(crate) number: usize,
    pub(crate) prompt: &'a str,
}

pub(crate) fn ask_with_reused_thread<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedAnswer, EvaluatorError> {
    let config = runtime.config;
    let session_key = evaluator_session_key(request.enforced_scope);
    let existing_session = state.sessions_by_scope.get(&session_key).cloned();
    let had_existing_session = existing_session.is_some();
    let mut session_id = match existing_session {
        Some(existing) => {
            write_thread_reuse_event(diagnostic_log, state, &config.agent, &existing, request)?;
            existing
        }
        None => start_thread_session(
            runtime,
            runner,
            diagnostic_log,
            state,
            &session_key,
            request,
        )?,
    };
    let response = match ask_in_thread(
        runner,
        &session_id,
        &config.agent,
        &mut state.parse_cache,
        diagnostic_log,
        request,
    ) {
        Ok(response) => response,
        Err(err) if had_existing_session && is_context_window_failure(&err) => {
            remove_thread_session(state, &session_key);
            write_model_fallback_events(
                diagnostic_log,
                request.number,
                request.model,
                None,
                err.message_str(),
            )?;
            write_thread_restart_event(diagnostic_log, &session_id, request, err.message_str())?;
            session_id = start_thread_session(
                runtime,
                runner,
                diagnostic_log,
                state,
                &session_key,
                request,
            )?;
            match ask_in_thread(
                runner,
                &session_id,
                &config.agent,
                &mut state.parse_cache,
                diagnostic_log,
                request,
            ) {
                Ok(response) => response,
                Err(err) => {
                    if session_failure_invalidates_thread(&err) {
                        remove_thread_session(state, &session_key);
                    }
                    return Err(err);
                }
            }
        }
        Err(err) => {
            if session_failure_invalidates_thread(&err) {
                remove_thread_session(state, &session_key);
            }
            return Err(err);
        }
    };
    state.sessions_by_scope.insert(session_key, session_id);
    Ok(response)
}

fn ask_in_thread<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    agent: &AgentConfig,
    parse_cache: &mut EvaluatorResponseParseCache,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    request: ThreadTurnRequest<'_>,
) -> Result<ParsedAnswer, EvaluatorError> {
    let turn = EvaluatorTurnContext {
        session_id,
        model: request.model,
        thinking: request.thinking,
    };
    ask_with_repairs(
        runner,
        &turn,
        request.prompt,
        agent,
        parse_cache,
        diagnostic_log,
        request.number,
    )
}

fn start_thread_session<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    session_key: &str,
    request: ThreadTurnRequest<'_>,
) -> Result<String, EvaluatorError> {
    let config = runtime.config;
    let developer_instructions = developer_instructions(&config.agent, request.enforced_scope);
    let created = runner.start_session(
        runtime.snapshot_root,
        &developer_instructions,
        &config.agent,
        request.model,
        request.thinking,
        request.enforced_scope,
    )?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "thread.start",
            &[
                ("threadId", json!(created.clone())),
                ("scope", json!(request.enforced_scope)),
                ("model", json!(model_label(request.model))),
                ("thinking", json!(request.thinking)),
                ("developerInstructions", json!(developer_instructions)),
            ],
        )?;
    }
    state
        .session_instructions
        .insert(created.clone(), developer_instructions);
    state
        .sessions_by_scope
        .insert(session_key.to_string(), created.clone());
    Ok(created)
}

fn write_thread_reuse_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &InterrogationState,
    agent: &AgentConfig,
    session_id: &str,
    request: ThreadTurnRequest<'_>,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    let developer_instructions = state
        .session_instructions
        .get(session_id)
        .cloned()
        .unwrap_or_else(|| developer_instructions(agent, request.enforced_scope));
    writer.write_event(
        "info",
        "thread.reuse",
        &[
            ("threadId", json!(session_id)),
            ("scope", json!(request.enforced_scope)),
            ("model", json!(model_label(request.model))),
            ("thinking", json!(request.thinking)),
            ("developerInstructions", json!(developer_instructions)),
        ],
    )
}

fn write_thread_restart_event(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    session_id: &str,
    request: ThreadTurnRequest<'_>,
    reason: &str,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer.write_event(
        "warn",
        "thread.restart",
        &[
            ("threadId", json!(session_id)),
            ("number", json!(request.number)),
            ("scope", json!(request.enforced_scope)),
            ("model", json!(model_label(request.model))),
            ("reason", json!(reason)),
        ],
    )
}

fn remove_thread_session(state: &mut InterrogationState, session_key: &str) {
    if let Some(removed) = state.sessions_by_scope.remove(session_key) {
        state.session_instructions.remove(&removed);
    }
}

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
    let prompt = expectation.q.clone();
    let thinking = effective_thinking(&config.agent, expectation);
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            enforced_scope: &enforced_scope,
            model,
            thinking,
            number: expectation.number,
            prompt: &prompt,
        },
    )?;
    finalize_interrogation_response(
        runtime,
        expectation,
        diagnostic_log,
        state,
        &enforced_scope,
        response,
    )
}
