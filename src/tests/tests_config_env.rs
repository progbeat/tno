use super::*;

#[test]
fn missing_thread_id_fails() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new("missing-thread");
    env_snapshot.remove("CODEX_THREAD_ID");
    env_snapshot.set("CANON_HOME", home.path());
    let result = Config::from_env();
    assert!(result.is_err());
}

#[test]
fn unsafe_thread_id_segments_fail() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new("unsafe-thread");
    env_snapshot.set("CANON_HOME", home.path());

    env_snapshot.set("CODEX_THREAD_ID", "..");
    assert!(Config::from_env().is_err());

    env_snapshot.set("CODEX_THREAD_ID", ".");
    assert!(Config::from_env().is_err());
}

#[test]
fn canon_home_overrides_default_root() {
    with_env("home-override", |home| {
        let config = Config::from_env().unwrap();
        assert_eq!(config.root, home.join("codex").join("thread-test"));
    });
}

#[test]
fn default_root_uses_tmpdir() {
    with_tmpdir("tmpdir-root", |temp| {
        let config = Config::from_env().unwrap();
        assert_eq!(
            config.root,
            temp.join("canon").join("codex").join("thread-test")
        );
    });
}

#[test]
fn default_root_uses_system_temp_without_tmpdir() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "TMPDIR", "CODEX_THREAD_ID"]);
    env_snapshot.remove("CANON_HOME");
    env_snapshot.remove("TMPDIR");
    env_snapshot.set("CODEX_THREAD_ID", "thread-test");
    let config = Config::from_env().unwrap();
    assert_eq!(
        config.root,
        env::temp_dir()
            .join("canon")
            .join("codex")
            .join("thread-test")
    );
}
