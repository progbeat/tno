use super::*;

#[test]
fn cooldown_reuse_stops_at_latest_valid_failure() {
    let root = git_project("history-cooldown-latest-fail");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "old pass".to_string(),
            scope: full_scope(),
            scope_hash: scope_hash.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            number: 1,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "no".to_string(),
            evidence: "latest fail".to_string(),
            scope: full_scope(),
            scope_hash: scope_hash.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "not-a-timestamp".to_string(),
            number: 1,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "no".to_string(),
            evidence: "invalid timestamp fail".to_string(),
            scope: full_scope(),
            scope_hash: "newer".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    assert!(
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_stops_at_latest_valid_non_answer_record() {
    let root = git_project("history-cooldown-latest-idk");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "old pass".to_string(),
            scope: full_scope(),
            scope_hash: scope_hash.clone(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            number: 1,
            result: CheckResult::Fail,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: OBSERVED_IDK.to_string(),
            evidence: "latest human review".to_string(),
            scope: full_scope(),
            scope_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();

    let mut history_cache = HistoryCache::new();
    assert!(
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_uses_fresh_pass_without_cache_key_filter() {
    let root = git_project("history-cooldown-no-cache-key-gate");
    let old_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let new_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 2d
"#,
    )
    .unwrap();
    let old_expectation = check_options(&old_config, &["1"], false, false).selected[0].clone();
    let new_expectation = check_options(&new_config, &["1"], false, false).selected[0].clone();
    let mut record = expectation_record(
        &old_config.agent,
        &old_expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &old_config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = format_log_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &old_expectation, &record).unwrap();

    let mut history_cache = HistoryCache::new();
    assert!(cooldown_history_record(
        &root,
        &new_config.agent,
        &new_expectation,
        &mut history_cache,
        unix_timestamp().unwrap(),
    )
    .unwrap()
    .unwrap()
    .passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_skips_records_with_invalid_timestamps() {
    let root = git_project("history-cooldown-invalid-timestamp");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "old pass".to_string(),
            scope: full_scope(),
            scope_hash,
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "not-a-timestamp".to_string(),
            number: 1,
            result: CheckResult::Pass,
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "invalid timestamp pass".to_string(),
            scope: full_scope(),
            scope_hash: "new".to_string(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    let reused =
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .unwrap();
    assert_eq!(reused.evidence, "old pass");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cooldown_reuse_returns_history_record_without_updating_metadata() {
    let root = git_project("history-cooldown-preserves-record");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#,
    )
    .unwrap();
    let mut expectation = check_options(&config, &["1"], false, false).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let record = CheckRecord {
        timestamp: "1970-01-01T00:00:10Z".to_string(),
        number: 42,
        result: CheckResult::Pass,
        prompt: "Question?".to_string(),
        expected: "yes".to_string(),
        observed: "yes".to_string(),
        evidence: "old pass".to_string(),
        scope: full_scope(),
        scope_hash,
        cache_key: Some(history_cache_key(&config.agent, &expectation)),
    };
    append_history_record(&root, &expectation, &record).unwrap();
    expectation.number = 7;

    let mut history_cache = HistoryCache::new();
    let reused =
        cooldown_history_record(&root, &config.agent, &expectation, &mut history_cache, 30)
            .unwrap()
            .unwrap();

    assert_eq!(reused.number, 42);
    assert_eq!(reused.prompt, "Question?");
    assert_eq!(reused.expected, "yes");
    assert_eq!(reused.timestamp, "1970-01-01T00:00:10Z");
    let _ = fs::remove_dir_all(root);
}
