use super::*;

#[test]
fn lazy_full_scope_reset_sets_only_sampled_narrowed_history_to_full_scope() {
    let root = git_project("check-lazy-reset-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let first_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let second_scope = vec!["README.md".to_string()];
    let second_hash = staged_scope_hash(&root, &config.agent, &second_scope).unwrap();
    append_history_record(
        &root,
        &expectations[0],
        &expectation_record(&config.agent, &expectations[0], "pass", "yes", first_hash),
    )
    .unwrap();
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        second_hash.clone(),
    );
    narrowed_record.scope = second_scope;
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();
    let reset_history_path = history_path(&root, &expectations[1]).unwrap();

    reset_non_selected_expectation_histories(&root, &[expectations[1].clone()]).unwrap();

    assert_eq!(
        read_history_records(&root, &expectations[0]).unwrap().len(),
        1
    );
    assert!(reset_history_path.exists());
    let reset_records = read_history_records(&root, &expectations[1]).unwrap();
    assert!(reset_records.is_empty());
    assert!(
        reusable_history_record(&root, &config.agent, &expectations[1])
            .unwrap()
            .is_none()
    );
    let mut history_cache = HistoryCache::new();
    assert_eq!(
        latest_history_scope_with_cache(
            &root,
            &config.agent,
            &expectations[1],
            &mut history_cache,
        )
        .unwrap(),
        None
    );
    assert_eq!(
        latest_history_scope_with_cache(
            &root,
            &config.agent,
            &expectations[1],
            &mut HistoryCache::new(),
        )
        .unwrap()
        .unwrap_or_else(full_scope),
        full_scope()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_count_uses_token_ratio_and_candidate_cap() {
    assert_eq!(lazy_full_scope_reset_count(0, 10, 1, 5), 0);
    assert_eq!(lazy_full_scope_reset_count(400, 10, 1, 5), 2);
    assert_eq!(lazy_full_scope_reset_count(1_000, 10, 1, 3), 3);
}

#[test]
fn lazy_full_scope_reset_plan_samples_only_narrowed_history() {
    let root = git_project("check-lazy-reset-candidates");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    append_history_record(
        &root,
        &expectations[0],
        &expectation_record(
            &config.agent,
            &expectations[0],
            "pass",
            "yes",
            staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
        ),
    )
    .unwrap();
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectations[1],
        "pass",
        "no",
        staged_scope_hash(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope;
    append_history_record(&root, &expectations[1], &narrowed_record).unwrap();

    let plan =
        plan_lazy_full_scope_reset(&root, &config.agent, u64::MAX, &expectations, 0).unwrap();

    assert_eq!(plan.candidate_count, 1);
    assert_eq!(plan.expectations.len(), 1);
    assert_eq!(plan.expectations[0].id, expectations[1].id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_preserves_existing_full_scope_pass_when_resetting_narrowed_scope() {
    let root = git_project("check-lazy-reset-preserve-full-pass");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &expectation_record(
            &config.agent,
            &expectation,
            "pass",
            "yes",
            staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
        ),
    )
    .unwrap();
    let narrowed_scope = vec!["README.md".to_string()];
    let mut narrowed_record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &narrowed_scope).unwrap(),
    );
    narrowed_record.scope = narrowed_scope;
    append_history_record(&root, &expectation, &narrowed_record).unwrap();

    reset_non_selected_expectation_histories(&root, std::slice::from_ref(&expectation)).unwrap();

    let records = read_history_records(&root, &expectation).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].scope, full_scope());
    assert_eq!(
        reusable_history_record(&root, &config.agent, &expectation)
            .unwrap()
            .map(|record| record.scope),
        Some(full_scope())
    );
    let mut history_cache = HistoryCache::new();
    assert_eq!(
        latest_history_scope_with_cache(&root, &config.agent, &expectation, &mut history_cache,)
            .unwrap(),
        Some(full_scope())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_size_estimate_counts_staged_text_and_skips_binary() {
    let root = git_project("check-lazy-reset-text-size");
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
"#,
    )
    .unwrap();
    let baseline = estimate_staged_project_size_tokens(&root, &config.agent).unwrap();
    fs::write(root.join("text.txt"), "12345678").unwrap();
    fs::write(root.join("binary.bin"), [0xff, 0xfe, 0xfd]).unwrap();
    let add = Command::new("git")
        .args(["add", "text.txt", "binary.bin"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(add.status.success());

    assert_eq!(
        estimate_staged_project_size_tokens(&root, &config.agent).unwrap(),
        baseline + 2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn project_size_estimate_reads_staged_colon_prefixed_paths() {
    let root = git_project("check-lazy-reset-colon-path");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let baseline = estimate_staged_project_size_tokens(&root, &config.agent).unwrap();
    let path = ":notes.txt";
    fs::write(root.join(path), "12345678").unwrap();
    let add = Command::new("git")
        .args(["--literal-pathspecs", "add", "--", path])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );

    assert_eq!(
        estimate_staged_project_size_tokens(&root, &config.agent).unwrap(),
        baseline + 2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_does_not_create_missing_history_files() {
    let root = git_project("check-lazy-reset-missing-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();

    reset_non_selected_expectation_histories(&root, &[expectation]).unwrap();

    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_preserves_full_scope_cooldown_pass() {
    let root = git_project("check-lazy-reset-full-cooldown");
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
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    let mut record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    record.timestamp = format_record_timestamp(unix_timestamp().unwrap());
    append_history_record(&root, &expectation, &record).unwrap();
    let reset_history_path = history_path(&root, &expectation).unwrap();

    reset_non_selected_expectation_histories(&root, std::slice::from_ref(&expectation)).unwrap();

    assert!(reset_history_path.exists());
    assert_eq!(read_history_records(&root, &expectation).unwrap().len(), 1);
    let mut history_cache = HistoryCache::new();
    assert!(cooldown_history_record(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        unix_timestamp().unwrap(),
    )
    .unwrap()
    .is_some());
    let _ = fs::remove_dir_all(root);
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn project_size_estimate_tolerates_non_utf8_paths() {
    let root = git_project("check-lazy-reset-non-utf8");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let path = git_path_from_raw_bytes(b"nonutf8-\xff.txt").unwrap();
    fs::write(root.join(&path), "raw path").unwrap();
    let output = Command::new("git")
        .arg("add")
        .arg(&path)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(estimate_staged_project_size_tokens(&root, &config.agent).is_ok());
    assert!(staged_scope_hash(&root, &config.agent, &full_scope()).is_ok());
    let _ = fs::remove_dir_all(root);
}
