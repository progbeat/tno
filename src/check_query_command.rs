use crate::check_command::prepare_check_execution;
use crate::check_interrogation_state::{CheckRuntime, InterrogationState};
use crate::check_output::write_query_output;
use crate::check_query::run_query_with_runner;
use crate::check_reporting::{
    collect_check_token_usage, print_token_usage_summary, write_check_finish_event,
    write_check_token_usage_event, CheckFinishStats,
};
use crate::config_types::CheckConfig;
use crate::logging::DiagnosticLogWriter;
use crate::scope_hash::ScopeHashCache;
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
    diagnostic_log
        .write_event(
            "info",
            "check.start",
            &[
                ("query", json!(true)),
                ("selected", json!(Vec::<usize>::new())),
            ],
        )
        .map_err(|err| err.to_string())?;
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut execution = prepare_check_execution(
        root,
        config,
        &mut diagnostic_log,
        true,
        1,
        &mut scope_hash_cache,
    )?;
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
            let usage = collect_check_token_usage(&mut execution.runner)?;
            print_token_usage_summary(Some(usage))?;
            write_check_token_usage_event(&mut diagnostic_log, usage)?;
            write_query_error_finish(&mut diagnostic_log, &err)?;
            return Err(err);
        }
    };
    // `run_query_with_runner` returns only when the query answer is known. This
    // is the first public stdout piece query mode can compute, so write and
    // flush it before token-usage collection or finish logging can do later
    // work.
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if let Err(err) = write_query_output(&mut stdout, &result.answer) {
        write_query_error_finish(&mut diagnostic_log, &err)?;
        return Err(err);
    }
    let usage = match collect_check_token_usage(&mut execution.runner) {
        Ok(usage) => usage,
        Err(err) => {
            write_query_error_finish(&mut diagnostic_log, &err)?;
            return Err(err);
        }
    };
    // Query token usage is not known until pending app-server usage updates are
    // drained above. Once known, `print_token_usage_summary` writes and flushes
    // the stderr line before the internal finish log event is recorded.
    print_token_usage_summary(Some(usage))?;
    write_check_token_usage_event(&mut diagnostic_log, usage)?;
    // Query mode is ad-hoc and has no selected/non-selected expectation set.
    // Do not run lazy full-scope reset here; that reset invalidates expectation
    // caches and belongs only to normal expectation-check invocations.
    write_check_finish_event(&mut diagnostic_log, true, CheckFinishStats::default(), None)
}

fn write_query_error_finish(
    diagnostic_log: &mut DiagnosticLogWriter,
    err: &str,
) -> Result<(), String> {
    write_check_finish_event(
        diagnostic_log,
        true,
        CheckFinishStats {
            errors: 1,
            ..CheckFinishStats::default()
        },
        Some(err),
    )
}
