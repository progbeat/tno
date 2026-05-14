use super::*;

#[test]
fn check_runner_replaces_restricted_idk_with_full_scope_answer() {
    let root = git_project("check-restricted-idk");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "idk".to_string(),
            evidence: "src/main.rs was not enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "src/main.rs was not enough", &["src/main.rs"]),
        &answer("yes", "README.md and src/main.rs answer it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(
        runner.start_scopes,
        vec![vec!["src/main.rs".to_string()], vec![".".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_retries_full_scope_for_restricted_idk_with_empty_evidence() {
    let root = git_project("check-restricted-idk-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "", &["src/main.rs"]),
        &answer("yes", "full project answers it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(
        runner.start_scopes,
        vec![vec!["src/main.rs".to_string()], vec![".".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_rejects_widened_idk_scope_then_retries_full_scope() {
    let root = git_project("check-restricted-idk-widened-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "src/main.rs was not enough", &["."]),
        &answer("yes", "full project answers it", &["."]),
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

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(
        runner.start_scopes,
        vec![vec!["src/main.rs".to_string()], vec![".".to_string()]]
    );
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert!(log.contains("widens enforced scope"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_starts_from_latest_answer_history_scope_even_when_failed() {
    let root = git_project("check-failed-history-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "no".to_string(),
            evidence: "restricted scope was misleading".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer(
        "yes",
        "restricted scope now answers it",
        &["src/main.rs"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_scope_seed_ignores_non_reusable_history_answer() {
    let root = git_project("check-history-scope-non-reusable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: UNPARSEABLE_OBSERVED.to_string(),
            evidence: "legacy review record kept a useful scope".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer("yes", "full project answers it", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_ignore_cache_uses_latest_history_scope() {
    let root = git_project("check-ignore-cache-history-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer(
        "yes",
        "src/main.rs still answers it",
        &["src/main.rs"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_verifies_narrowed_scope_after_restricted_idk_widens() {
    let root = git_project("check-restricted-idk-narrows");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "idk".to_string(),
            evidence: "src/main.rs was not enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "src/main.rs was not enough", &["src/main.rs"]),
        &answer("yes", "src is enough", &["src"]),
        &answer("yes", "src still answers it", &["src"]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records[0].passed());
    assert_eq!(records.records[0].observed, "yes");
    assert_eq!(records.records[0].scope, vec!["src".to_string()]);
    assert_eq!(
        runner.start_scopes,
        vec![
            vec!["src/main.rs".to_string()],
            vec![".".to_string()],
            vec!["src".to_string()]
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_widen_restricted_answer_mismatch() {
    let root = git_project("check-restricted-failure");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "src/main.rs was misleading", &["src/main.rs"]),
        &answer("yes", "full project context answers it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, "no");
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_rejects_restricted_scope_widening_without_full_scope_retry() {
    let root = git_project("check-restricted-widening");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("yes", "needs wider scope", &["."]),
        &answer("yes", "full project answers it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert!(records.records[0]
        .evidence
        .contains("widens enforced scope"));
    assert_eq!(records.records[0].scope, vec!["src/main.rs"]);
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_widen_restricted_unparseable_response() {
    let root = git_project("check-restricted-unparseable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, false);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            id: expectation.id.clone(),
            display_id: expectation.display_id.clone(),
            number: expectation.number,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "malformed".to_string(),
            evidence: "restricted response was empty".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&["", ""]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records.records[0].passed());
    assert_eq!(records.records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    assert_eq!(read_history_records(&root, &expectation).unwrap().len(), 2);
    let _ = fs::remove_dir_all(root);
}
