use super::*;

#[test]
fn log_timestamp_uses_utc_rfc3339_format() {
    assert_eq!(format_log_record_timestamp(0), "1970-01-01T00:00:00Z");
}

#[test]
fn diagnostic_log_is_written_to_numeric_active_file_and_flushed() {
    let root = git_project("check-log");
    let records = vec![sample_record(1, "pass")];
    let path = write_diagnostic_log(&root, &records).unwrap();
    assert_eq!(path, root.join(".git/canon/logs/0.jsonl"));
    let content = fs::read_to_string(&path).unwrap();
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let json: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(json["result"], "pass");
    assert_eq!(json["id"], expectation_id("Question?", "yes"));
    assert!(json.get("number").is_none());
    assert_eq!(json["prompt"], "Question?");
    assert_eq!(json["expected"], "yes");
    assert_eq!(json["observed"], "yes");
    assert_eq!(json["evidence"], "README.md has evidence");
    assert_eq!(json["scope"], json!(["."]));
    assert_eq!(json["scopeHash"], "AAAAAAAAAAAAAAAAAAAA");
    let expected_order = [
        "\"timestamp\"",
        "\"id\"",
        "\"result\"",
        "\"observed\"",
        "\"evidence\"",
        "\"scope\"",
        "\"scopeHash\"",
        "\"prompt\"",
        "\"expected\"",
    ];
    let mut previous = 0;
    for key in expected_order {
        let index = lines[0].find(key).unwrap();
        assert!(index >= previous);
        previous = index;
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_log_rotates_at_start_when_active_file_is_large() {
    let root = git_project("check-log-rotate");
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config();
    assert_eq!(config.max_bytes, 1024 * 1024);
    assert_eq!(config.files.len(), 8);
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((config.max_bytes + 1) as usize),
    )
    .unwrap();
    for (index, file_name) in config.files.iter().enumerate().skip(1) {
        fs::write(log_dir.join(file_name), format!("old-{index}")).unwrap();
    }

    let writer = DiagnosticLogWriter::create(&root).unwrap();
    assert_eq!(writer.path, log_dir.join("0.jsonl"));
    assert!(!log_dir.join("0.jsonl").exists());
    assert_eq!(
        fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
        (config.max_bytes + 1) as usize
    );
    for (index, file_name) in config.files.iter().enumerate().skip(2) {
        assert_eq!(
            fs::read_to_string(log_dir.join(file_name)).unwrap(),
            format!("old-{}", index - 1)
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_runtime_log_event_rotates_before_writing() {
    let root = git_project("runtime-log-append-rotate");
    let log_dir = root.join(".git/canon/logs");
    fs::create_dir_all(&log_dir).unwrap();
    let config = diagnostic_log_config();
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((config.max_bytes + 1) as usize),
    )
    .unwrap();

    append_runtime_log_event(&root, "error", "worktree.restore.failed", &[]).unwrap();

    let active = fs::read_to_string(log_dir.join("0.jsonl")).unwrap();
    assert!(active.contains(r#""event":"worktree.restore.failed""#));
    assert_eq!(
        fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
        (config.max_bytes + 1) as usize
    );
    let _ = fs::remove_dir_all(root);
}
