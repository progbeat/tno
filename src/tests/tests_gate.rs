use super::*;

#[test]
fn gate_passes_with_current_cached_pass() {
    let root = git_project("gate-pass");
    write_check_config(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let scope_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "pass", "yes", scope_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_fails_when_cache_is_missing() {
    let root = git_project("gate-missing");
    write_check_config(&root);

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_failed_error_display_is_descriptive() {
    assert_eq!(CommandError::GateFailed.to_string(), "canon gate failed");
    assert!(!command_error_needs_main_print(&CommandError::GateFailed));
    assert!(command_error_needs_main_print(&CommandError::CheckFailed));
}

#[test]
fn gate_skips_canon_only_change_when_visible_content_is_unchanged() {
    let root = git_project("gate-canon-only");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_does_not_skip_non_canon_change_with_missing_cache() {
    let root = git_project("gate-non-canon-missing");
    commit_all(&root, "initial");
    write_check_config(&root);
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert_eq!(result.unwrap_err(), CommandError::GateFailed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_accepts_fresh_cooldown_pass_when_scope_hash_changed() {
    let root = git_project("gate-cooldown-pass");
    commit_all(&root, "initial");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    let config = parse_check_config(yaml).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let old_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let mut record =
        expectation_record(&config.agent, &expectation, "pass", "yes", old_hash.clone());
    record.timestamp = format_log_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    assert_ne!(current_hash, old_hash);

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_prefers_fresh_cooldown_pass_over_older_exact_fail() {
    let root = git_project("gate-cooldown-over-exact-fail");
    commit_all(&root, "initial");
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "Question?"
    a: "yes"
    cooldown: 1d
"#;
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), yaml).unwrap();
    Command::new("git")
        .arg("add")
        .arg(CHECK_PATH)
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "add check config");
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(yaml).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();
    let mut pass = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        "old".to_string(),
    );
    pass.timestamp = format_log_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &pass).unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_fails_for_new_current_failure_without_head_failure() {
    let root = git_project("gate-new-fail");
    commit_all(&root, "initial");
    write_check_config(&root);
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gate_accepts_failure_already_present_on_head() {
    let root = git_project("gate-head-fail");
    commit_all(&root, "initial");
    write_check_config(&root);
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let head_hash =
        scope_hash_for_source(&root, &config.agent, &full_scope(), ScopeHashSource::Head)
            .unwrap()
            .unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", head_hash),
    )
    .unwrap();
    fs::write(root.join("README.md"), "changed\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    let current_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(&config.agent, &expectation, "fail", "no", current_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}
