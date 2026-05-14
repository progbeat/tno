use super::*;

#[test]
fn check_runner_uses_model_fallback_after_usage_limit() {
    let root = git_project("check-model-fallback");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let answer = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new_results(vec![
        Err(EvaluatorError::failure(
            EvaluatorFailureKind::UsageLimit,
            "app-server turn/start failed: usageLimitExceeded",
        )),
        Ok(&answer),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records.records[0].passed());
    assert_eq!(runner.starts, 2);
    assert_eq!(
        runner.start_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.3-codex-spark".to_string())
        ]
    );
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec![".".to_string()]]
    );
    assert_eq!(
        runner.ask_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.3-codex-spark".to_string())
        ]
    );
    assert_eq!(
        runner.sessions,
        vec!["session-1".to_string(), "session-2".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_keeps_using_fallback_after_model_failure() {
    let root = git_project("check-sticky-model-fallback");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new_results(vec![
        Err(EvaluatorError::failure(
            EvaluatorFailureKind::UsageLimit,
            "app-server turn/start failed: usageLimitExceeded",
        )),
        Ok(&answer("yes", "first answer", &["."])),
        Ok(&answer("no", "second answer", &["."])),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 2);
    assert_eq!(
        runner.start_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.3-codex-spark".to_string())
        ]
    );
    assert_eq!(
        runner.ask_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.3-codex-spark".to_string()),
            Some("gpt-5.3-codex-spark".to_string())
        ]
    );
    assert_eq!(
        runner.sessions,
        vec![
            "session-1".to_string(),
            "session-2".to_string(),
            "session-2".to_string()
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_restarts_reused_thread_after_context_window_error() {
    let root = git_project("check-context-restart");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new_results(vec![
        Ok(&answer("yes", "first answer", &["."])),
        Err(EvaluatorError::failure(
            EvaluatorFailureKind::ContextWindow,
            "app-server turn/start failed: Codex ran out of room in the model's context window",
        )),
        Ok(&answer("no", "second answer", &["."])),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert!(log.contains(r#""event":"model.failure""#));
    assert!(log.contains(r#""event":"thread.restart""#));
    assert_eq!(runner.starts, 2);
    assert_eq!(
        runner.sessions,
        vec![
            "session-1".to_string(),
            "session-1".to_string(),
            "session-2".to_string()
        ]
    );
    assert_eq!(
        runner.start_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.4-mini".to_string())
        ]
    );
    let _ = fs::remove_dir_all(root);
}
