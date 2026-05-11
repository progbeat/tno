use super::*;

#[test]
fn init_creates_template_and_fails_when_existing() {
    let root = temp_home("init");
    run_init(&root).unwrap();
    let check_path = root.join(CHECK_PATH);
    let agents_path = root.join(AGENTS_PATH);
    assert_eq!(
        fs::read_to_string(&check_path).unwrap(),
        DEFAULT_CHECK_TEMPLATE
    );
    assert_eq!(
        fs::read_to_string(&agents_path).unwrap(),
        DEFAULT_AGENTS_TEMPLATE
    );
    assert!(!root.join(".gitignore").exists());
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    assert!(run_init(&root).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn init_preserves_existing_agents_file() {
    let root = temp_home("init-existing-agents");
    let agents_path = root.join(AGENTS_PATH);
    fs::write(&agents_path, "repo-specific instructions\n").unwrap();

    run_init(&root).unwrap();

    assert_eq!(
        fs::read_to_string(root.join(CHECK_PATH)).unwrap(),
        DEFAULT_CHECK_TEMPLATE
    );
    assert_eq!(
        fs::read_to_string(&agents_path).unwrap(),
        "repo-specific instructions\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn init_does_not_require_thread_id() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CODEX_THREAD_ID"]);
    env_snapshot.remove("CODEX_THREAD_ID");
    let root = temp_home("init-no-thread");
    run_init(&root).unwrap();
    assert!(root.join(CHECK_PATH).exists());
    let _ = fs::remove_dir_all(root);
}
