use super::*;

#[test]
fn mixed_canon_and_non_canon_changes_fail() {
    assert!(fail_on_mixed_canon_paths(&[".canon/check.yml".to_string()]).is_ok());
    assert!(fail_on_mixed_canon_paths(&["src/main.rs".to_string()]).is_ok());
    assert!(fail_on_mixed_canon_paths(&[
        ".canon/check.yml".to_string(),
        "src/main.rs".to_string()
    ])
    .is_err());
}

#[test]
fn staged_changed_paths_include_rename_sources() {
    let root = git_project("rename-into-canon");
    commit_all(&root, "initial");
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::rename(root.join("src/main.rs"), root.join(".canon/main.rs")).unwrap();
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let paths = staged_changed_paths(&root).unwrap();

    assert!(paths.contains(&"src/main.rs".to_string()));
    assert!(paths.contains(&".canon/main.rs".to_string()));
    assert!(fail_on_mixed_canon_paths(&paths).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_changed_paths_tolerate_non_utf8_paths() {
    let paths = staged_changed_paths_from_name_status_z(b"A\0nonutf8-\xff\0").unwrap();

    assert_eq!(paths.len(), 1);
    assert!(fail_on_mixed_canon_paths(&paths).is_ok());
}

#[test]
fn check_command_logs_start_and_finish_for_mixed_canon_preflight_failure() {
    let root = git_project("check-mixed-preflight-log");
    write_check_config(&root);
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let output = Command::new("git")
        .args(["add", ".canon/check.yml", "src/main.rs"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_check_command(&root, &[]).unwrap_err();

    assert!(err
        .to_string()
        .contains(".canon/** changes must not be mixed"));
    let log = fs::read_to_string(root.join(".git/canon/logs/0.jsonl")).unwrap();
    assert!(log.contains(r#""event":"check.start""#));
    assert!(log.contains(r#""event":"check.finish""#));
    assert!(log.contains(r#""error":"canon check failed: .canon/** changes must not be mixed"#));
    let _ = fs::remove_dir_all(root);
}
