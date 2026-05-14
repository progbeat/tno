use crate::check_command::prepare_check_execution;
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_lazy_reset::apply_lazy_full_scope_reset_or_warn;
use crate::check_output::write_query_output;
use crate::check_query::run_query_with_runner;
use crate::check_reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
    CheckFinishStats,
};
use crate::check_selection::initial_non_selected_expectations;
use crate::logging::DiagnosticLogWriter;
use crate::types::{CheckConfig, TokenUsage};
use serde_json::json;
use std::io;
use std::path::Path;

pub(crate) fn run_check_query_command(
    root: &Path,
    config: &CheckConfig,
    question: &str,
    mut diagnostic_log: DiagnosticLogWriter,
) -> Result<(), String> {
    // `canon check -q` is an ad-hoc interrogation mode. It loads the active
    // evaluator config, but it does not select or run expectations and is not a
    // per-expectation check run governed by the normal check-output summary.
    diagnostic_log.write_event(
        "info",
        "check.start",
        &[
            ("query", json!(true)),
            ("selected", json!(Vec::<usize>::new())),
        ],
    )?;
    let mut execution = prepare_check_execution(root, config, &mut diagnostic_log, true, 1)?;
    let runtime = CheckRuntime {
        root,
        snapshot_root: execution.staged_view.snapshot_root(),
        config,
    };
    let mut interrogation_state = InterrogationState::new();
    let result = run_query_with_runner(
        &runtime,
        question,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    );
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            let usage = collect_check_token_usage(&mut execution.runner, &mut diagnostic_log)?;
            print_token_usage_summary(Some(usage))?;
            apply_query_lazy_full_scope_reset(root, config, usage, &mut diagnostic_log)?;
            write_check_finish_event(
                &mut diagnostic_log,
                true,
                CheckFinishStats {
                    errors: 1,
                    ..CheckFinishStats::default()
                },
                Some(&err),
            )?;
            return Err(err);
        }
    };
    // The query answer is the first public stdout piece produced by query mode.
    // Write and flush it immediately at the command boundary, before token-usage
    // collection, lazy reset, or finish logging can do later work.
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if let Err(err) = write_query_output(&mut stdout, &result.answer) {
        write_check_finish_event(
            &mut diagnostic_log,
            true,
            CheckFinishStats {
                errors: 1,
                ..CheckFinishStats::default()
            },
            Some(&err),
        )?;
        return Err(err);
    }
    let usage = match collect_check_token_usage(&mut execution.runner, &mut diagnostic_log) {
        Ok(usage) => usage,
        Err(err) => {
            write_check_finish_event(
                &mut diagnostic_log,
                true,
                CheckFinishStats {
                    errors: 1,
                    ..CheckFinishStats::default()
                },
                Some(&err),
            )?;
            return Err(err);
        }
    };
    print_token_usage_summary(Some(usage))?;
    apply_query_lazy_full_scope_reset(root, config, usage, &mut diagnostic_log)?;
    write_check_finish_event(&mut diagnostic_log, true, CheckFinishStats::default(), None)
}

fn apply_query_lazy_full_scope_reset(
    root: &Path,
    config: &CheckConfig,
    usage: TokenUsage,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    let non_selected = initial_non_selected_expectations(config, &[])?;
    apply_lazy_full_scope_reset_or_warn(root, config, usage, &non_selected, diagnostic_log);
    Ok(())
}
