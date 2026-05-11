use crate::*;

pub(crate) fn run_check_query_command(
    root: &Path,
    config: &CheckConfig,
    question: &str,
    repo_cache: &mut RepoInspectionCache,
) -> Result<(), String> {
    // `canon check -q` is an ad-hoc interrogation mode. It loads the active
    // evaluator config, but it does not select or run expectations and is not a
    // per-expectation check run governed by the normal check-output summary.
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, repo_cache)?;
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
        snapshot_root: root,
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
    let usage = collect_check_token_usage(&mut execution.runner, &mut diagnostic_log)?;
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            print_token_usage_summary(Some(usage));
            write_check_finish_event(
                &mut diagnostic_log,
                true,
                0,
                0,
                1,
                0,
                NarrowingStats::default(),
                Some(&err),
            )?;
            return Err(err);
        }
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_query_output(&mut stdout, &result.answer)?;
    print_token_usage_summary(Some(usage));
    write_check_finish_event(
        &mut diagnostic_log,
        true,
        0,
        0,
        0,
        0,
        NarrowingStats::default(),
        None,
    )
}
