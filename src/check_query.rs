use crate::check_interrogation::{ask_with_reused_thread, ThreadTurnRequest};
use crate::check_interrogation_records::finalize_query_response;
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_model_fallback::run_with_model_fallbacks;
use crate::check_types::QueryInterrogationResult;
use crate::evaluator_types::{EvaluatorError, EvaluatorRunner};
use crate::hash::full_scope;
use crate::logging::DiagnosticLogWriter;

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
        None,
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
    // Query mode has no expectation ID and no history-derived scope seed. Its
    // enforced scope is therefore always the full staged snapshot; when normal
    // expectation mode is also run with full scope, both paths send the same
    // developer instructions and the same question text into the first turn.
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
            expectation_id: None,
            prompt: &prompt,
        },
    )?;
    finalize_query_response(runtime, question, diagnostic_log, state, response.answer)
}
