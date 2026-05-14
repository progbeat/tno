use crate::app_server::LazyAppServerRunner;
use crate::check::run_check_with_runner;
use crate::check_command_args::parse_check_command_args;
use crate::check_lazy_reset::apply_lazy_full_scope_reset_or_warn;
use crate::check_output::write_summary_line;
use crate::check_preflight::install_sigint_handler;
use crate::check_query_command::run_check_query_command;
use crate::check_reporting::{
    collect_and_print_check_token_usage, write_check_finish_event, write_check_finish_report_event,
    CheckFinishStats,
};
use crate::check_selection::parse_check_options;
use crate::check_validation::check_config_loads_plugins;
use crate::cli::CommandError;
use crate::git::resolve_git_path;
use crate::history_cleanup::maybe_cleanup_stale_cache_dirs;
use crate::logging::DiagnosticLogWriter;
use crate::repo_inspection::RepoInspectionCache;
use crate::staged_worktree::StagedWorktreeView;
use crate::types::{CheckConfig, CheckRecord};
use crate::{CHECK_INTERRUPTED, GIT_CANON_CACHE_DIR};
use serde_json::json;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    install_sigint_handler().map_err(CommandError::from)?;
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
    let command = parse_check_command_args(args)?;
    let mut repo_cache = RepoInspectionCache::new();
    // Runtime logs are canon-owned state under `.git/canon/logs`, not project
    // working-tree content. They are created before snapshot evaluation and are
    // denied to evaluator sessions by the mandatory ignore policy.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let config = match repo_cache.load_check_config(root, &command.config_path) {
        Ok(config) => config,
        Err(err) => {
            diagnostic_log.write_event(
                "info",
                "check.start",
                &[
                    ("query", json!(command.query.is_some())),
                    ("selected", json!(Vec::<String>::new())),
                ],
            )?;
            write_check_finish_event(
                &mut diagnostic_log,
                command.query.is_some(),
                CheckFinishStats {
                    errors: 1,
                    ..CheckFinishStats::default()
                },
                Some(&err),
            )?;
            return Err(err.into());
        }
    };
    if let Some(question) = command.query.as_deref() {
        return run_check_query_command(root, &config, question, diagnostic_log)
            .map_err(CommandError::from);
    }
    // Check-specific options are parsed with the active config so selectors can
    // be resolved against expectation IDs.
    let options = match parse_check_options(&config, &command.option_args) {
        Ok(options) => options,
        Err(err) => {
            diagnostic_log.write_event(
                "info",
                "check.start",
                &[("selected", json!(Vec::<String>::new()))],
            )?;
            write_check_finish_event(
                &mut diagnostic_log,
                false,
                CheckFinishStats::default(),
                Some(&err),
            )?;
            return Err(err.into());
        }
    };
    diagnostic_log.write_event(
        "info",
        "check.start",
        &[(
            "selected",
            json!(options
                .selected
                .iter()
                .map(|expectation| expectation.id.clone())
                .collect::<Vec<_>>()),
        )],
    )?;
    let mut execution = prepare_check_execution(root, &config, &mut diagnostic_log, false, 0)
        .map_err(CommandError::from)?;
    let cache_dir = resolve_git_path(root, GIT_CANON_CACHE_DIR).map_err(CommandError::from)?;
    let cleanup = match maybe_cleanup_stale_cache_dirs(&cache_dir, &config) {
        Ok(cleanup) => cleanup,
        Err(err) => {
            write_check_finish_event(
                &mut diagnostic_log,
                false,
                CheckFinishStats {
                    errors: 1,
                    ..CheckFinishStats::default()
                },
                Some(&err),
            )?;
            return Err(err.into());
        }
    };
    if cleanup.sampled {
        diagnostic_log.write_event(
            "info",
            "cache.cleanup",
            &[
                ("removed", json!(cleanup.removed)),
                ("kept", json!(cleanup.kept)),
            ],
        )?;
    }
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut result_output: &mut dyn Write = &mut stdout;
    // `run_check_with_runner` calls `write_and_flush_result_output` after each
    // selected expectation; that helper renders the public human-readable
    // check-output record (`P. OK`, `P. FAILED`, or `P. ERROR`) and flushes it
    // before the next expectation starts.
    let records_result = run_check_with_runner(
        root,
        execution.staged_view.snapshot_root(),
        &config,
        &options,
        &mut execution.runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
    );
    if let Err(err) = result_output.flush() {
        let err = format!("failed to flush check result to stdout: {}", err);
        write_check_finish_event(
            &mut diagnostic_log,
            false,
            CheckFinishStats {
                errors: 1,
                ..CheckFinishStats::default()
            },
            Some(&err),
        )?;
        return Err(err.into());
    }
    let usage =
        match collect_and_print_check_token_usage(&mut execution.runner, &mut diagnostic_log) {
            Ok(usage) => usage,
            Err(err) => {
                write_check_finish_event(
                    &mut diagnostic_log,
                    false,
                    CheckFinishStats {
                        errors: 1,
                        ..CheckFinishStats::default()
                    },
                    Some(&err),
                )?;
                return Err(err.into());
            }
        };
    let report = match records_result {
        Ok(report) => report,
        Err(err) => {
            let report = err.report;
            apply_lazy_full_scope_reset_or_warn(
                root,
                &config,
                usage,
                &report.non_selected,
                &mut diagnostic_log,
            );
            write_check_finish_report_event(&mut diagnostic_log, false, &report, Some(&err.error))?;
            // The summary line is computed from the final report and elapsed
            // time here. Per-expectation records have already been flushed as
            // they were produced inside `run_check_with_runner`.
            write_summary_line(&mut result_output, &report, started.elapsed())?;
            return Err(CommandError::CheckFailed);
        }
    };
    apply_lazy_full_scope_reset_or_warn(
        root,
        &config,
        usage,
        &report.non_selected,
        &mut diagnostic_log,
    );
    write_check_finish_report_event(&mut diagnostic_log, false, &report, None)?;
    // The summary line does not exist until the final report exists. This call
    // renders that newly computed stdout piece and flushes it immediately.
    write_summary_line(&mut result_output, &report, started.elapsed())?;
    if report.records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}

pub(crate) struct PreparedCheckExecution {
    pub(crate) staged_view: StagedWorktreeView,
    pub(crate) runner: LazyAppServerRunner,
}

pub(crate) fn prepare_check_execution(
    root: &Path,
    config: &CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
) -> Result<PreparedCheckExecution, String> {
    // Materialize the staged Git snapshot outside the real working tree so
    // evaluator sessions cannot observe unstaged or untracked project content.
    let staged_view = match StagedWorktreeView::apply(root) {
        Ok(staged_view) => staged_view,
        Err(err) => {
            write_prepare_check_failure(diagnostic_log, query, errors_on_failure, &err)?;
            return Err(err);
        }
    };
    let runner = LazyAppServerRunner::new(check_config_loads_plugins(config), &config.agent);
    Ok(PreparedCheckExecution {
        staged_view,
        runner,
    })
}

fn write_prepare_check_failure(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
    err: &str,
) -> Result<(), String> {
    write_check_finish_event(
        diagnostic_log,
        query,
        CheckFinishStats {
            errors: errors_on_failure,
            ..CheckFinishStats::default()
        },
        Some(err),
    )
}
