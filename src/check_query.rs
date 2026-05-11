use crate::*;

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
) -> Result<QueryInterrogationResult, String> {
    let mut diagnostic_log = diagnostic_log;
    let mut failures = Vec::new();
    let models = state.available_models(&runtime.config.agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match interrogate_query_with_model(
            runtime,
            question,
            runner,
            &mut diagnostic_log,
            state,
            model.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                let next_model = models.get(model_index + 1);
                if next_model.is_some() {
                    // Query mode shares the same per-invocation fallback
                    // stickiness as normal expectation checks.
                    state.mark_model_unavailable(model.as_deref());
                }
                if let Some(writer) = diagnostic_log.as_deref_mut() {
                    writer.write_event(
                        "warn",
                        "model.failure",
                        &[
                            ("number", json!(0)),
                            ("model", json!(model_label(model.as_deref()))),
                            ("error", json!(err.message_str())),
                        ],
                    )?;
                    if let Some(next_model) = next_model {
                        writer.write_event(
                            "warn",
                            "model.fallback",
                            &[
                                ("number", json!(0)),
                                ("from", json!(model_label(model.as_deref()))),
                                ("to", json!(model_label(next_model.as_deref()))),
                                ("reason", json!(err.message_str())),
                            ],
                        )?;
                    }
                }
                failures.push(format!(
                    "{}: {}",
                    model_label(model.as_deref()),
                    err.message_str()
                ));
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Err(format!(
        "all evaluator models failed: {}",
        failures.join("; ")
    ))
}

pub(crate) fn interrogate_query_with_model<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    model: Option<&str>,
) -> Result<QueryInterrogationResult, EvaluatorError> {
    let config = runtime.config;
    let enforced_scope = full_scope();
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
                        ("thinking", json!(config.agent.thinking.clone())),
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
                &config.agent.thinking,
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
                        ("thinking", json!(config.agent.thinking.clone())),
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
    let prompt = question.to_string();
    let turn = EvaluatorTurnContext {
        session_id: &session_id,
        model,
        thinking: &config.agent.thinking,
    };
    let response = match ask_with_repairs(
        runner,
        &turn,
        &prompt,
        &config.agent,
        &mut state.parse_cache,
        diagnostic_log,
        0,
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
                        ("number", json!(0)),
                        ("model", json!(model_label(model))),
                        ("error", json!(err.message_str())),
                    ],
                )?;
                writer.write_event(
                    "warn",
                    "thread.restart",
                    &[
                        ("threadId", json!(session_id.clone())),
                        ("number", json!(0)),
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
                &config.agent.thinking,
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
                        ("thinking", json!(config.agent.thinking.clone())),
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
                thinking: &config.agent.thinking,
            };
            match ask_with_repairs(
                runner,
                &turn,
                &prompt,
                &config.agent,
                &mut state.parse_cache,
                diagnostic_log,
                0,
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
    let mut response = response;
    if response.answer == UNPARSEABLE_OBSERVED {
        response.scope = enforced_scope.to_vec();
    } else {
        if !scope_is_within(&response.scope, &enforced_scope) {
            response = ParsedAnswer {
                answer: UNPARSEABLE_OBSERVED.to_string(),
                evidence: format!(
                    "evaluator response scope {:?} widens enforced scope {:?}",
                    response.scope, enforced_scope
                ),
                scope: enforced_scope.to_vec(),
            };
        }
    }
    let scope_hash =
        state
            .scope_hash_cache
            .staged_scope_hash(runtime.root, &config.agent, &response.scope)?;
    if response.answer == OBSERVED_MALFORMED {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[
                    ("number", json!(0)),
                    ("reason", json!(MALFORMED_REVIEW_WARNING)),
                ],
            )?;
        }
    }
    if response.answer == UNPARSEABLE_OBSERVED {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[
                    ("number", json!(0)),
                    ("reason", json!("unparseable evaluator response")),
                ],
            )?;
        }
    }
    if response.answer == OBSERVED_IDK {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event(
                "warn",
                "review.required",
                &[("number", json!(0)), ("reason", json!("full-scope idk"))],
            )?;
        }
    }
    if response.evidence.trim().is_empty() {
        if let Some(writer) = diagnostic_log.as_deref_mut() {
            writer.write_event("warn", "evidence.empty", &[("number", json!(0))])?;
        }
    }
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_event(
            "info",
            "query.result",
            &[
                ("prompt", json!(question)),
                ("observed", json!(response.answer.clone())),
                ("evidence", json!(response.evidence.clone())),
                ("scope", json!(response.scope.clone())),
                ("scopeHash", json!(scope_hash.clone())),
            ],
        )?;
    }
    Ok(QueryInterrogationResult { answer: response })
}
