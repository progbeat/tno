use super::*;

#[test]
fn history_path_uses_expectation_id_directory() {
    let root = git_project("history-path");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1"], false, true);
    let expectation = options.selected.remove(0);
    assert_eq!(
        history_path(&root, &expectation).unwrap(),
        root.join(".git")
            .join(GIT_CANON_CACHE_DIR)
            .join(&expectation.id)
            .join(history_file_name())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_cache_cleanup_removes_inactive_expectation_entries() {
    let root = git_project("history-cleanup-stale-cache");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let active_ids = active_expectation_ids(&config);
    let active_id = active_ids.iter().next().unwrap().clone();
    let cache_dir = root.join(".git/canon/cache");
    fs::create_dir_all(cache_dir.join(&active_id)).unwrap();
    fs::write(cache_dir.join(&active_id).join(history_file_name()), "").unwrap();
    fs::create_dir_all(cache_dir.join("stale-id")).unwrap();
    fs::write(cache_dir.join("stale-id").join(history_file_name()), "").unwrap();
    fs::write(cache_dir.join("stale-file"), "old").unwrap();

    let stats = cleanup_stale_cache_dirs(&root, &active_ids).unwrap();

    assert_eq!(stats.removed, 2);
    assert_eq!(stats.kept, 1);
    assert!(cache_dir.join(&active_id).exists());
    assert!(!cache_dir.join("stale-id").exists());
    assert!(!cache_dir.join("stale-file").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn malformed_history_json_lines_fail_explicitly() {
    let root = git_project("history-malformed-json");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, "{not json}\n").unwrap();

    let error = read_history_records(&root, &expectation).unwrap_err();

    assert!(error.contains("invalid history JSON"));
    assert!(error.contains("line 1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_history_record_updates_in_memory_cache() {
    let root = git_project("history-cache-coherent");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1"], false, true);
    let expectation = options.selected.remove(0);
    let mut history_cache = HistoryCache::new();
    assert!(history_cache
        .read_records(&root, &expectation)
        .unwrap()
        .is_empty());

    let record = expectation_record(
        &config.agent,
        &expectation,
        "pass",
        "yes",
        staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
    );
    append_history_record_with_cache(&root, &expectation, &record, &mut history_cache).unwrap();

    let cached = history_cache.read_records(&root, &expectation).unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].observed, "yes");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_replaces_file_after_writing_latest_lines() {
    let root = git_project("history-compact");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let records = (1..=7)
        .map(|number| {
            let mut record = sample_record(number, "pass");
            record.evidence = format!("record {number}");
            serde_json::to_string(&record).unwrap()
        })
        .collect::<Vec<_>>();
    fs::write(&path, format!("{}\n", records.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 3", "record 4", "record 5", "record 6", "record 7"]
    );
    assert!(!compact_history_temp_path(&path).unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_malformed_lines_and_keeps_latest_valid_records() {
    let root = git_project("history-compact-malformed");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    let mut lines = vec!["not json".to_string()];
    lines.extend((1..=7).map(|number| {
        let mut record = sample_record(number, "pass");
        record.evidence = format!("record {number}");
        serde_json::to_string(&record).unwrap()
    }));
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 5);
    assert_eq!(
        compacted
            .iter()
            .map(|record| record.evidence.clone())
            .collect::<Vec<_>>(),
        vec!["record 3", "record 4", "record 5", "record 6", "record 7"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_history_drops_non_record_json_objects() {
    let root = git_project("history-compact-non-record");
    let path = root.join(".git/canon/cache/example/history.jsonl");
    ensure_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{{\"n\":1}}\n{}\n",
            serde_json::to_string(&sample_record(1, "pass")).unwrap()
        ),
    )
    .unwrap();

    compact_history(&path).unwrap();

    let compacted = read_history_records_from_path(&path).unwrap();
    assert_eq!(compacted.len(), 1);
    assert_eq!(compacted[0].id, expectation_id("Question?", "yes"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn git_path_from_raw_bytes_preserves_non_utf8_unix_paths() {
    use std::os::unix::ffi::OsStrExt;

    let path = git_path_from_raw_bytes(b"not-utf8-\xff.md").unwrap();
    assert_eq!(path.as_os_str().as_bytes(), b"not-utf8-\xff.md");
}
