use super::*;

#[test]
fn lazy_full_scope_reset_invalidates_only_sampled_non_selected_history() {
    let root = git_project("check-lazy-reset-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectations = check_options(&config, &[], false, true).selected;
    let first_hash = staged_scope_hash(&root, &config.agent, &full_scope()).unwrap();
    let second_hash = first_hash.clone();
    append_history_record(
        &root,
        &expectations[0],
        &expectation_record(&config.agent, &expectations[0], "pass", "yes", first_hash),
    )
    .unwrap();
    append_history_record(
        &root,
        &expectations[1],
        &expectation_record(&config.agent, &expectations[1], "pass", "no", second_hash),
    )
    .unwrap();

    assert!(reset_non_selected_expectation_histories(&root, &[expectations[1].clone()]).is_empty());

    assert_eq!(
        read_history_records(&root, &expectations[0]).unwrap().len(),
        1
    );
    let reset_records = read_history_records(&root, &expectations[1]).unwrap();
    assert!(reset_records.is_empty());
    assert!(
        reusable_history_record(&root, &config.agent, &expectations[1])
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_count_uses_token_ratio_and_candidate_cap() {
    assert_eq!(lazy_full_scope_reset_count(0, 10, 1, 5), 0);
    assert_eq!(lazy_full_scope_reset_count(1_000, 10, 1, 3), 3);
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
fn lazy_full_scope_reset_does_not_create_missing_history_files() {
    let root = git_project("check-lazy-reset-missing-history");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &[], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();

    assert!(reset_non_selected_expectation_histories(&root, &[expectation]).is_empty());

    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lazy_full_scope_reset_removes_cooldown_pass() {
    let root = git_project("check-lazy-reset-cooldown");
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

    assert!(
        reset_non_selected_expectation_histories(&root, std::slice::from_ref(&expectation))
            .is_empty()
    );

    assert!(read_history_records(&root, &expectation)
        .unwrap()
        .is_empty());
    let mut history_cache = HistoryCache::new();
    assert!(cooldown_history_record(
        &root,
        &config.agent,
        &expectation,
        &mut history_cache,
        unix_timestamp().unwrap(),
    )
    .unwrap()
    .is_none());
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
    let _ = fs::remove_dir_all(root);
}
