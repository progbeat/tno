use super::*;

#[test]
fn reusable_history_record_uses_current_expectation_metadata() {
    let root = git_project("history-current-number");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: 99,
            result: CheckResult::Pass,
            prompt: "old prompt text".to_string(),
            expected: "old expected".to_string(),
            observed: "yes".to_string(),
            evidence: "cached answer".to_string(),
            scope: full_scope(),
            scope_hash: staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
            cache_key: Some(history_cache_key(&config.agent, &expectation)),
        },
    )
    .unwrap();

    let mut moved = expectation.clone();
    moved.number = 7;
    let record = reusable_history_record(&root, &config.agent, &moved)
        .unwrap()
        .unwrap();
    assert_eq!(record.number, 7);
    assert_eq!(record.prompt, expectation.q);
    assert_eq!(record.expected, expectation.a);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_allows_missing_cache_key() {
    let root = git_project("history-cache-key-metadata");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.cache_key = None;
    append_history_record(&root, &expectation, &record).unwrap();

    assert!(reusable_history_record(&root, &config.agent, &expectation)
        .unwrap()
        .unwrap()
        .passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_uses_scope_hash_not_cache_key_for_changed_evaluator_config() {
    let root = git_project("history-cache-key-not-reuse-gate");
    let old_config = parse_check_config(check_config_yaml()).unwrap();
    let new_config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  thinking: medium
  instructions: |
    Different evaluator instructions.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let old_expectation = check_options(&old_config, &["1"], false, true).selected[0].clone();
    let new_expectation = check_options(&new_config, &["1"], false, true).selected[0].clone();
    append_history_record(
        &root,
        &old_expectation,
        &expectation_record(
            &old_config.agent,
            &old_expectation,
            "pass",
            "yes",
            staged_scope_hash(&root, &new_config.agent, &full_scope()).unwrap(),
        ),
    )
    .unwrap();

    assert!(
        reusable_history_record(&root, &new_config.agent, &new_expectation)
            .unwrap()
            .unwrap()
            .passed()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reusable_history_record_ignores_current_agent_ignore_patterns() {
    let root = git_project("history-reuse-ignore-independent");
    let base_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let ignored_readme_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "README.md"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let old_expectation = check_options(&base_config, &["1"], false, true).selected[0].clone();
    let new_expectation =
        check_options(&ignored_readme_config, &["1"], false, true).selected[0].clone();
    append_history_record(
        &root,
        &old_expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: old_expectation.number,
            result: CheckResult::Pass,
            prompt: old_expectation.q.clone(),
            expected: old_expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "README.md answers it".to_string(),
            scope: vec!["README.md".to_string()],
            scope_hash: staged_scope_hash(&root, &base_config.agent, &["README.md".to_string()])
                .unwrap(),
            cache_key: Some(history_cache_key(&base_config.agent, &old_expectation)),
        },
    )
    .unwrap();

    let record = reusable_history_record(&root, &ignored_readme_config.agent, &new_expectation)
        .unwrap()
        .unwrap();

    assert!(record.passed());
    assert_eq!(record.scope, vec!["README.md"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_does_not_depend_on_agent_ignore_patterns() {
    let root = git_project("history-scope-hash-ignore-independent");
    let base_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();
    let ignored_readme_config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "README.md"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
"#,
    )
    .unwrap();

    assert_eq!(
        staged_scope_hash(&root, &base_config.agent, &full_scope()).unwrap(),
        staged_scope_hash(&root, &ignored_readme_config.agent, &full_scope()).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_hash_includes_tracked_canon_paths() {
    let root = git_project("history-scope-hash-includes-canon");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let before = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();

    let after = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();

    assert_ne!(before, after);
    let _ = fs::remove_dir_all(root);
}
