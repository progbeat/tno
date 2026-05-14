use super::*;

#[test]
fn check_runner_hides_expected_answers_and_reuses_session() {
    let root = git_project("check-runner");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
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
    assert!(log
        .lines()
        .any(|line| line.contains(r#""event":"thread.reuse""#)
            && line.contains(r#""developerInstructions":"#)));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_roots, vec![root.clone()]);
    assert_eq!(
        runner.start_ignores,
        vec![vec![
            ".canon".to_string(),
            ".canon/**".to_string(),
            ".git/canon".to_string(),
            ".git/canon/**".to_string(),
            ".git/canon/logs".to_string(),
            ".git/canon/logs/**".to_string(),
            "target/**".to_string()
        ]]
    );
    assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
    assert_eq!(runner.start_models, vec![Some("gpt-5.4-mini".to_string())]);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
    assert_eq!(
        runner.ask_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.4-mini".to_string())
        ]
    );
    assert_eq!(
        runner.ask_thinking,
        vec!["medium".to_string(), "medium".to_string()]
    );
    assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
    assert!(runner
        .prompts
        .iter()
        .all(|prompt| !prompt.contains("Response format:")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_applies_thinking_per_turn_when_reusing_scope_thread() {
    let root = git_project("check-thinking-turn");
    let config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
  thinking: low
  instructions: Answer from files only.
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "yes"
    thinking: high
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("yes", "README.md says enough", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_thinking, vec!["low".to_string()]);
    assert_eq!(
        runner.ask_thinking,
        vec!["low".to_string(), "high".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_verifies_narrowed_scope_before_history_reuse() {
    let root = git_project("check-narrowing-accepted");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("yes", "src/main.rs still supports it", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records.records[0].passed());
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-accepted-incorrect");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("no", "src/main.rs still fails it", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "no");
    assert_eq!(records.records[0].scope, vec!["src/main.rs".to_string()]);
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-rejected");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("no", "full scope fails it", &["src/main.rs"]),
        &answer(
            "yes",
            "src/main.rs changes to a passing answer",
            &["src/main.rs"],
        ),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "no");
    assert_eq!(records.records[0].scope, vec!["."]);
    let history = read_history_records(&root, &options.selected[0]).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observed, "no");
    assert_eq!(history[0].scope, vec!["."]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_fails_mismatch_and_treats_idk_as_exact_string() {
    let root = git_project("check-fails");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("idk", "not enough", &["."]),
        &answer("yes", "wrong", &["."]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records.records[0].passed());
    assert!(!records.records[1].passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_treats_full_scope_idk_as_human_review() {
    let root = git_project("check-full-scope-idk");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "Are there any unused files?"
    a: "no"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("idk", "not enough evidence", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert!(record_requires_human_review(&records.records[0]));
    assert_eq!(records.records[0].observed, "idk");
    assert!(
        latest_recorded_non_pass_timestamp(&root, &options.selected[0])
            .unwrap()
            .is_some()
    );
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_stops_after_first_failure_by_default() {
    let root = git_project("check-default-stop");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = parse_check_options(
        &config,
        &[
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer("no", "wrong", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 1);
    assert!(!records.records[0].passed());
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_all_checks_full_selected_set_after_failure() {
    let root = git_project("check-all-after-failure");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = parse_check_options(
        &config,
        &[
            "--all".into(),
            test_selector(&config, "1").into(),
            test_selector(&config, "2").into(),
        ],
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "wrong", &["."]),
        &answer("no", "second answer", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(records.records.len(), 2);
    assert_eq!(runner.prompts.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_expectations_run_latest_non_pass_first() {
    let root = git_project("check-order-latest-non-pass");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        "no",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    append_history_record(&root, &second, &record).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first, second.clone()],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, second.id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_expectations_use_recorded_errors_for_order() {
    let root = git_project("check-order-runtime-errors");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let mut record = expectation_record(
        &config.agent,
        &second,
        "fail",
        UNPARSEABLE_OBSERVED,
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = "2026-01-01T00:00:00Z".to_string();
    write_latest_non_pass_record(&root, &second, &record).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first, second.clone()],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, second.id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_expectations_ignore_runtime_log_errors_for_order() {
    let root = git_project("check-order-ignores-runtime-errors");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let first = options.selected[0].clone();
    let second = options.selected[1].clone();
    let log_dir = root.join(".git/canon/logs");
    ensure_dir(&log_dir).unwrap();
    let line = render_runtime_log_event(
        "info",
        "expectation.result",
        &[
            ("id", json!(second.id.clone())),
            ("result", json!(RESULT_FAIL)),
            ("observed", json!(UNPARSEABLE_OBSERVED)),
        ],
    )
    .unwrap();
    fs::write(log_dir.join("0.jsonl"), line).unwrap();
    let mut history_cache = HistoryCache::new();

    let ordered = order_expectations_by_latest_non_pass(
        &root,
        vec![first.clone(), second],
        &mut history_cache,
    )
    .unwrap();

    assert_eq!(ordered[0].id, first.id);
    let _ = fs::remove_dir_all(root);
}
