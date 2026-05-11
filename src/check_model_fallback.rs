use crate::*;

pub(crate) fn interrogate_expectation_with_response_repairs<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    expectation: &SelectedExpectation,
    runner: &mut R,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    let mut failures = Vec::new();
    let models = state.available_models(&runtime.config.agent);
    for (model_index, model) in models.iter().enumerate() {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match interrogate_expectation_with_model(
            runtime,
            expectation,
            runner,
            diagnostic_log,
            state,
            enforced_scope,
            model.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                let next_model = models.get(model_index + 1);
                if next_model.is_some() {
                    // The primary/fallback order is still honored for the
                    // first technical failure. Once a fallback succeeds during
                    // this invocation, skip the failed model for later
                    // interrogations because usage limits refresh on a much
                    // longer timescale than a single `canon check` run.
                    state.mark_model_unavailable(model.as_deref());
                }
                write_model_fallback_events(
                    diagnostic_log,
                    expectation.number,
                    model.as_deref(),
                    next_model.and_then(Option::as_deref),
                    err.message_str(),
                )?;
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

fn write_model_fallback_events(
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    number: usize,
    model: Option<&str>,
    next_model: Option<&str>,
    error: &str,
) -> Result<(), String> {
    let Some(writer) = diagnostic_log.as_deref_mut() else {
        return Ok(());
    };
    writer.write_event(
        "warn",
        "model.failure",
        &[
            ("number", json!(number)),
            ("model", json!(model_label(model))),
            ("error", json!(error)),
        ],
    )?;
    if let Some(next_model) = next_model {
        writer.write_event(
            "warn",
            "model.fallback",
            &[
                ("number", json!(number)),
                ("from", json!(model_label(model))),
                ("to", json!(model_label(Some(next_model)))),
                ("reason", json!(error)),
            ],
        )?;
    }
    Ok(())
}
