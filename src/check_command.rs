use crate::*;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    let started = Instant::now();
    install_sigint_handler();
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
    let command = parse_check_command_args(args)?;
    let mut repo_cache = RepoInspectionCache::new();
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let config = match repo_cache.load_check_config(root, &command.config_path) {
        Ok(config) => config,
        Err(err) => {
            diagnostic_log.write_event(
                "info",
                "check.start",
                &[
                    ("query", json!(command.query.is_some())),
                    ("selected", json!(Vec::<usize>::new())),
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
    // `canon check` accepts `--fail-fast` through `parse_check_options`; the
    // `canon gate` rejection below is gate-specific and does not apply here.
    let options = match parse_check_options(&config, &command.option_args) {
        Ok(options) => options,
        Err(err) => {
            diagnostic_log.write_event(
                "info",
                "check.start",
                &[("selected", json!(Vec::<usize>::new()))],
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
                .map(|expectation| expectation.number)
                .collect::<Vec<_>>()),
        )],
    )?;
    let mut execution = prepare_check_execution(root, &config, &mut diagnostic_log, false, 0)
        .map_err(CommandError::from)?;
    let cleanup = match maybe_cleanup_stale_cache_dirs(root, &config) {
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
    // check-output record (`N. OK`, `N. FAILED`, or `N. ERROR`) and flushes it
    // before the next expectation starts.
    let records_result = run_check_with_runner(
        root,
        root,
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
    if let Err(err) =
        collect_and_print_check_token_usage(&mut execution.runner, &mut diagnostic_log)
    {
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
    let report = match records_result {
        Ok(report) => report,
        Err(err) => {
            let report = err.report;
            write_check_finish_report_event(&mut diagnostic_log, false, &report, Some(&err.error))?;
            write_summary_line(&mut result_output, &report, started.elapsed())?;
            return Err(CommandError::CheckFailed);
        }
    };
    write_check_finish_report_event(&mut diagnostic_log, false, &report, None)?;
    write_summary_line(&mut result_output, &report, started.elapsed())?;
    if report.records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err(CommandError::CheckFailed)
    }
}

pub(crate) struct PreparedCheckExecution {
    pub(crate) _staged_view: StagedWorktreeView,
    pub(crate) runner: LazyAppServerRunner,
}

pub(crate) fn prepare_check_execution(
    root: &Path,
    config: &CheckConfig,
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    errors_on_failure: usize,
) -> Result<PreparedCheckExecution, String> {
    if let Err(err) = staged_changed_paths(root).and_then(|paths| fail_on_mixed_canon_paths(&paths))
    {
        write_prepare_check_failure(diagnostic_log, query, errors_on_failure, &err)?;
        return Err(err);
    }
    // Apply the staged Git snapshot as an in-place index view: unstaged and
    // untracked worktree changes are preserved away, so the evaluator sees the
    // index contents at the real project root. This creates no copied
    // repository, copied tree, or copied snapshot directory. File visibility is
    // enforced by app-server permissions, not by copying the repository to a
    // filtered view.
    let staged_view = match StagedWorktreeView::apply(root) {
        Ok(staged_view) => staged_view,
        Err(err) => {
            write_prepare_check_failure(diagnostic_log, query, errors_on_failure, &err)?;
            return Err(err);
        }
    };
    let runner = LazyAppServerRunner::new(check_config_loads_plugins(config), &config.agent);
    Ok(PreparedCheckExecution {
        _staged_view: staged_view,
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
