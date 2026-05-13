use super::*;

#[test]
fn check_runner_requires_human_review_for_unparseable_response() {
    let root = git_project("check-unparseable-first-response");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&["not parseable"]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records[0].passed());
    assert_eq!(records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_marks_unparseable_after_response_parse_fails() {
    let root = git_project("check-unparseable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[""]);
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
    assert!(!records[0].passed());
    assert_eq!(records[0].observed, UNPARSEABLE_OBSERVED);
    assert!(records[0].evidence.contains("response: <empty>"));
    assert_eq!(runner.prompts.len(), 1);
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert!(log.contains(r#""event":"review.required""#));
    assert!(log.contains(r#""reason":"unparseable evaluator response""#));
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_retries_full_scope_for_restricted_idk_non_answer() {
    let root = git_project("check-narrow-idk");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("idk", "src/main.rs is insufficient", &["src"])]);

    let report =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert_eq!(report.records[0].observed, OBSERVED_IDK);
    assert_eq!(report.narrowing.attempted, 0);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_unparseable_response() {
    let root = git_project("check-unparseable-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new(&["not json", &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_when_evidence_stays_empty() {
    let root = git_project("check-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."])]);
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
    assert!(!records[0].passed());
    assert!(record_requires_human_review(&records[0]));
    assert_eq!(records[0].observed, EMPTY_EVIDENCE_OBSERVED);
    assert!(records[0].evidence.contains("evidence was empty"));
    assert_eq!(runner.prompts.len(), 1);
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert!(log.contains(r#""event":"review.required""#));
    assert!(log.contains(r#""reason":"empty evaluator evidence""#));
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_for_malformed_answer() {
    let root = git_project("check-malformed-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let malformed = answer("malformed", "question is malformed", &["."]);
    let mut runner = FakeRunner::new(&[&malformed]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "malformed");
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_malformed_answer() {
    let root = git_project("check-malformed-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let malformed = answer("malformed", "", &["."]);
    let mut runner = FakeRunner::new(&[&malformed, &malformed, &answer("yes", "late", &["."])]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "malformed");
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_retry_after_empty_evidence() {
    let root = git_project("check-empty-evidence-no-retry");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let later_answer = answer("yes", "README.md has evidence", &["."]);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &later_answer]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, EMPTY_EVIDENCE_OBSERVED);
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_keeps_semantic_malformed_as_human_review_failure() {
    let root = git_project("check-full-malformed");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer(
        "malformed",
        "full scope response stayed malformed",
        &["."],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "malformed");
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    let _ = fs::remove_dir_all(root);
}
