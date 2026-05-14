use super::*;

#[test]
fn check_runner_flushes_each_result_output_record() {
    let root = git_project("check-output");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let mut output = FlushCountingWriter::new();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();
    assert_eq!(records.records.len(), 2);
    assert_eq!(output.flushes, 2);
    let lines = String::from_utf8(output.bytes).unwrap();
    assert_eq!(lines.lines().count(), 2);
    assert!(lines.contains(&format!("{}. OK", options.selected[0].display_id)));
    assert!(lines.contains(&format!("{}. OK", options.selected[1].display_id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_output_escapes_non_ascii_control_characters() {
    assert_eq!(
        escape_check_output_text("line\nx\u{1f}y\u{7f}z\u{85}"),
        "line\\nx\\u001fy\\u007fz\\u0085"
    );
}

#[test]
fn check_output_failed_and_error_records_use_specified_line_counts() {
    let mut record = CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        id: "AAAAAAAAAAAAAAAAAAAA".to_string(),
        display_id: "A".to_string(),
        number: 7,
        result: CheckResult::Fail,
        prompt: "Question?".to_string(),
        expected: "yes".to_string(),
        observed: "no".to_string(),
        evidence: "evidence".to_string(),
        scope: vec!["src".to_string()],
        scope_hash: "hash".to_string(),
        cache_key: None,
    };

    assert_eq!(render_check_output_record(&record).lines().count(), 6);

    record.observed = OBSERVED_IDK.to_string();
    assert_eq!(render_check_output_record(&record).lines().count(), 5);
}

#[test]
fn query_mode_uses_agent_and_does_not_write_history() {
    let root = git_project("query-mode");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut runner = FakeRunner::new(&[&answer("no", "src/main.rs says no", &["src"])]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let runtime = CheckRuntime {
        root: &root,
        snapshot_root: &root,
        config: &config,
    };
    let mut interrogation_state = InterrogationState::new();

    let result = run_query_with_runner(
        &runtime,
        "Ad-hoc question?",
        &mut runner,
        Some(&mut diagnostic_log),
        &mut interrogation_state,
    )
    .unwrap();

    assert_eq!(result.answer.answer, "no");
    assert_eq!(result.answer.scope, vec!["src"]);
    assert_eq!(runner.prompts, vec!["Ad-hoc question?".to_string()]);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert_eq!(runner.start_models, vec![Some("gpt-5.4-mini".to_string())]);
    assert_eq!(runner.start_thinking, vec!["medium".to_string()]);
    let cache_dir = root.join(".git/canon/cache");
    assert!(!cache_dir.exists() || fs::read_dir(&cache_dir).unwrap().next().is_none());
    let output = render_query_output(&result.answer);
    assert_eq!(
        output,
        "Observed: no\nEvidence: src/main.rs says no\nScope: [\"src\"]\n"
    );
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert!(log.contains(r#""event":"query.result""#));
    let log_events: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let request = log_events
        .iter()
        .find(|event| event["event"] == "agent.request")
        .unwrap();
    assert_eq!(
        request["rawRequest"]["sessionId"].as_str(),
        Some("session-1")
    );
    assert_eq!(
        request["rawRequest"]["prompt"].as_str(),
        Some("Ad-hoc question?")
    );
    let response = log_events
        .iter()
        .find(|event| event["event"] == "agent.response")
        .unwrap();
    assert!(response["rawResponse"]
        .as_str()
        .unwrap()
        .starts_with(r#"{"answer":"no""#));
    assert!(!log.contains(r#""event":"expectation.result""#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn successful_narrowing_logs_stats_and_one_final_result() {
    let root = git_project("narrowing-success");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full answer", &["src"]),
        &answer("yes", "narrow answer", &["src"]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert_eq!(report.narrowing.attempted, 1);
    assert_eq!(report.narrowing.accepted, 1);
    assert_eq!(report.narrowing.rejected, 0);
    assert_eq!(report.records[0].scope, vec!["src"]);
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert_eq!(log.matches(r#""event":"expectation.result""#).count(), 1);
    assert_eq!(log.matches(r#""event":"interrogation.result""#).count(), 2);
    assert!(log.contains(r#""event":"scope.narrowing""#));
    assert!(log.contains(r#""accepted":true"#));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_narrowing_logs_stats_and_keeps_wider_final_result() {
    let root = git_project("narrowing-fail");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    let mut runner = FakeRunner::new(&[
        &answer("no", "full answer", &["src"]),
        &answer("yes", "narrow answer", &["src"]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();

    let report = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();

    assert_eq!(report.narrowing.attempted, 1);
    assert_eq!(report.narrowing.accepted, 0);
    assert_eq!(report.narrowing.rejected, 1);
    assert_eq!(report.records[0].observed, "no");
    assert_eq!(report.records[0].scope, vec!["."]);
    let history = read_history_records(&root, &expectation).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].observed, "no");
    assert_eq!(history[0].scope, vec!["."]);
    let log = fs::read_to_string(diagnostic_log.path).unwrap();
    assert_eq!(log.matches(r#""event":"expectation.result""#).count(), 1);
    assert_eq!(log.matches(r#""event":"interrogation.result""#).count(), 2);
    assert!(log.contains(r#""event":"scope.narrowing""#));
    assert!(log.contains(r#""accepted":false"#));
    let _ = fs::remove_dir_all(root);
}
