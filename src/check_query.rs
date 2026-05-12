use crate::*;

pub(crate) fn run_query_with_runner<R: EvaluatorRunner>(
    runtime: &CheckRuntime<'_>,
    question: &str,
    runner: &mut R,
    diagnostic_log: Option<&mut DiagnosticLogWriter>,
    state: &mut InterrogationState,
) -> Result<QueryInterrogationResult, String> {
    let mut diagnostic_log = diagnostic_log;
    run_with_model_fallbacks(
        &runtime.config.agent,
        state,
        &mut diagnostic_log,
        0,
        |state, diagnostic_log, model| {
            interrogate_query_with_model(runtime, question, runner, diagnostic_log, state, model)
        },
    )
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
    let prompt = question.to_string();
    let response = ask_with_reused_thread(
        runtime,
        runner,
        diagnostic_log,
        state,
        ThreadTurnRequest {
            enforced_scope: &enforced_scope,
            model,
            thinking: &config.agent.thinking,
            number: 0,
            prompt: &prompt,
        },
    )?;
    finalize_query_response(runtime, question, diagnostic_log, state, response)
}
