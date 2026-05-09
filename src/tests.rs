use crate::*;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> TestDir {
        let mut path = PathBuf::from("/tmp");
        path.push(format!("canon-test-{}-{}", name, process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create test directory");
        TestDir { path }
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn temp_home(name: &str) -> PathBuf {
    let mut path = PathBuf::from("/tmp");
    path.push(format!("canon-test-{}-{}", name, process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create test directory");
    path
}

struct EnvSnapshot {
    values: Vec<(&'static str, Option<OsString>)>,
}

impl EnvSnapshot {
    fn capture(keys: &[&'static str]) -> EnvSnapshot {
        EnvSnapshot {
            values: keys.iter().map(|key| (*key, env::var_os(key))).collect(),
        }
    }

    fn set(&self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
        env::set_var(key, value);
    }

    fn remove(&self, key: &str) {
        env::remove_var(key);
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (key, value) in &self.values {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }
}

fn with_env<F>(name: &str, f: F)
where
    F: FnOnce(PathBuf),
{
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "CODEX_THREAD_ID"]);
    let home = TestDir::new(name);
    env_snapshot.set("CANON_HOME", home.path());
    env_snapshot.set("CODEX_THREAD_ID", "thread-test");
    f(home.path());
}

fn with_tmpdir<F>(name: &str, f: F)
where
    F: FnOnce(PathBuf),
{
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "TMPDIR", "CODEX_THREAD_ID"]);
    let temp = TestDir::new(name);
    env_snapshot.remove("CANON_HOME");
    env_snapshot.set("TMPDIR", temp.path());
    env_snapshot.set("CODEX_THREAD_ID", "thread-test");
    f(temp.path());
}

fn check_config_yaml() -> &'static str {
    r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
    fallbacks:
      - gpt-5.3-codex-spark
  thinking: medium
  instructions: |
    Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "no"
"#
}

fn parse_check_config(yaml: &str) -> Result<CheckConfig, String> {
    parse_check_config_content(Path::new(".canon/check.yml"), yaml)
}

struct FakeRunner {
    answers: VecDeque<Result<String, String>>,
    prompts: Vec<String>,
    sessions: Vec<String>,
    start_roots: Vec<PathBuf>,
    start_ignores: Vec<Vec<String>>,
    start_models: Vec<Option<String>>,
    start_thinking: Vec<String>,
    start_plugins: Vec<Vec<String>>,
    start_scopes: Vec<Vec<String>>,
    starts: usize,
}

impl FakeRunner {
    fn new(answers: &[&str]) -> FakeRunner {
        FakeRunner {
            answers: answers
                .iter()
                .map(|answer| Ok((*answer).to_string()))
                .collect(),
            prompts: Vec::new(),
            sessions: Vec::new(),
            start_roots: Vec::new(),
            start_ignores: Vec::new(),
            start_models: Vec::new(),
            start_thinking: Vec::new(),
            start_plugins: Vec::new(),
            start_scopes: Vec::new(),
            starts: 0,
        }
    }

    fn new_results(answers: Vec<Result<&str, &str>>) -> FakeRunner {
        FakeRunner {
            answers: answers
                .into_iter()
                .map(|answer| answer.map(str::to_string).map_err(str::to_string))
                .collect(),
            prompts: Vec::new(),
            sessions: Vec::new(),
            start_roots: Vec::new(),
            start_ignores: Vec::new(),
            start_models: Vec::new(),
            start_thinking: Vec::new(),
            start_plugins: Vec::new(),
            start_scopes: Vec::new(),
            starts: 0,
        }
    }
}

impl EvaluatorRunner for FakeRunner {
    fn start_session(
        &mut self,
        root: &Path,
        _instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, String> {
        self.starts += 1;
        self.start_roots.push(root.to_path_buf());
        self.start_ignores.push(effective_ignore_patterns(agent));
        self.start_models
            .push(model.or(agent.model.primary.as_deref()).map(str::to_string));
        self.start_thinking.push(thinking.to_string());
        self.start_plugins.push(agent.plugins.clone());
        self.start_scopes.push(scope.to_vec());
        Ok(format!("session-{}", self.starts))
    }

    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
        self.sessions.push(session_id.to_string());
        self.prompts.push(prompt.to_string());
        self.answers
            .pop_front()
            .unwrap_or_else(|| Err("fake runner has no answer".to_string()))
    }
}

struct FlushCountingWriter {
    bytes: Vec<u8>,
    flushes: usize,
}

impl FlushCountingWriter {
    fn new() -> FlushCountingWriter {
        FlushCountingWriter {
            bytes: Vec::new(),
            flushes: 0,
        }
    }
}

impl std::io::Write for FlushCountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

fn git_project(name: &str) -> PathBuf {
    let root = temp_home(name);
    Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "hello").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap();
    root
}

fn commit_all(root: &Path, message: &str) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.test",
            "commit",
            "-m",
            message,
        ])
        .current_dir(root)
        .output()
        .expect("run git commit");
    assert!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_check_config(root: &Path) {
    fs::create_dir_all(root.join(".canon")).unwrap();
    fs::write(root.join(CHECK_PATH), check_config_yaml()).unwrap();
}

#[test]
fn git_project_root_finds_top_level_from_subdirectory() {
    let root = git_project("git-root-subdir");
    let subdir = root.join(".canon");
    fs::create_dir_all(&subdir).unwrap();
    assert_eq!(
        fs::canonicalize(git_project_root(&subdir).unwrap()).unwrap(),
        fs::canonicalize(&root).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_preserves_and_restores_changes() {
    let root = git_project("hide-worktree-changes");
    Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.test",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "unstaged\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();

    {
        let _staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!root.join("untracked.txt").exists());
    }

    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "unstaged\n"
    );
    assert!(root.join("untracked.txt").exists());
    let diff = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&diff.stdout).trim(), "README.md");
    let _ = fs::remove_dir_all(root);
}

fn check_options(
    config: &CheckConfig,
    numbers: &[&str],
    fail_fast: bool,
    ignore_cache: bool,
) -> CheckOptions {
    CheckOptions {
        selected: select_expectations(
            config,
            &numbers.iter().map(OsString::from).collect::<Vec<_>>(),
        )
        .unwrap(),
        fail_fast,
        ignore_cache,
    }
}

fn answer(answer: &str, evidence: &str, scope: &[&str]) -> String {
    serde_json::to_string(&json!({
        "answer": answer,
        "evidence": evidence,
        "scope": scope,
    }))
    .unwrap()
}

fn sample_record(number: usize, result: &str) -> CheckRecord {
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        number,
        result: result.to_string(),
        prompt: "Question?".to_string(),
        expected: "yes".to_string(),
        observed: if result == "pass" { "yes" } else { "no" }.to_string(),
        evidence: "README.md has evidence".to_string(),
        scope: vec![".".to_string()],
        scope_hash: "AAAAAAAAAAAAAAAAAAAA".to_string(),
    }
}

fn expectation_record(
    expectation: &SelectedExpectation,
    result: &str,
    observed: &str,
    scope_hash: String,
) -> CheckRecord {
    CheckRecord {
        timestamp: "1970-01-01T00:00:00Z".to_string(),
        number: expectation.number,
        result: result.to_string(),
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: observed.to_string(),
        evidence: "cached answer".to_string(),
        scope: full_scope(),
        scope_hash,
    }
}

#[test]
fn hash_is_ten_base64url_chars() {
    let hash = hash_key("src/lib.rs");
    assert_eq!(hash.len(), 10);
    assert!(hash
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
}

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
fn default_root_uses_slash_tmp_without_tmpdir() {
    let _guard = ENV_LOCK.lock().expect("lock test environment");
    let env_snapshot = EnvSnapshot::capture(&["CANON_HOME", "TMPDIR", "CODEX_THREAD_ID"]);
    env_snapshot.remove("CANON_HOME");
    env_snapshot.remove("TMPDIR");
    env_snapshot.set("CODEX_THREAD_ID", "thread-test");
    let config = Config::from_env().unwrap();
    assert_eq!(config.root, PathBuf::from("/tmp/canon/codex/thread-test"));
}

#[test]
fn path_creation_is_deterministic() {
    with_env("deterministic", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "a/b.rs").unwrap();
        let second = ensure_note(&config, "a/b.rs").unwrap();
        assert_eq!(first.path, second.path);
        assert!(first.path.exists());
    });
}

#[test]
fn write_and_append_preserve_metadata() {
    with_env("write-append", |_| {
        let config = Config::from_env().unwrap();
        write_note(&config, "src/main.rs", "body").unwrap();
        append_note(&config, "src/main.rs", "decision").unwrap();
        let note = note_for_key(&config, "src/main.rs").unwrap();
        let content = fs::read_to_string(note.path).unwrap();
        assert!(content.starts_with("<!-- canon key=\"src/main.rs\" hash=\""));
        assert!(content.contains("\nbody\n"));
        assert!(content.contains("decision"));
    });
}

#[test]
fn delete_removes_only_target() {
    with_env("delete", |_| {
        let config = Config::from_env().unwrap();
        let first = ensure_note(&config, "one").unwrap();
        let second = ensure_note(&config, "two").unwrap();
        delete_note(&config, "one").unwrap();
        assert!(!first.path.exists());
        assert!(second.path.exists());
        let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert!(!index.contains("\tone\n"));
        assert!(index.contains("\ttwo\n"));
    });
}

#[test]
fn collect_text_rejects_invalid_start_index() {
    let args = vec![OsString::from("one")];
    let err = collect_text(&args, 2).unwrap_err();
    assert!(err.contains("exceeds argument count"));
}

#[test]
fn note_keys_reject_index_separators() {
    with_env("bad-note-key", |_| {
        let config = Config::from_env().unwrap();
        assert!(write_note(&config, "bad\tkey", "body").is_err());
        assert!(write_note(&config, "bad\nkey", "body").is_err());
    });
}

#[test]
fn index_updates_do_not_drop_hash_collisions() {
    with_env("index-collision", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        fs::write(
            config.root.join("index.tsv"),
            "samehash\tother-key\noldhash\ttarget-key\n",
        )
        .unwrap();

        upsert_index(&config, "samehash", "target-key").unwrap();
        let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();

        assert!(index.contains("samehash\tother-key\n"));
        assert!(!index.contains("oldhash\ttarget-key\n"));
        assert!(index.contains("samehash\ttarget-key\n"));

        remove_index(&config, "samehash", "target-key").unwrap();
        let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();
        assert!(index.contains("samehash\tother-key\n"));
        assert!(!index.contains("samehash\ttarget-key\n"));
    });
}

#[test]
fn read_index_rejects_malformed_lines() {
    with_env("bad-index", |_| {
        let config = Config::from_env().unwrap();
        ensure_dir(&config.root).unwrap();
        let path = config.root.join("index.tsv");
        fs::write(&path, "missing-tab\n").unwrap();

        let err = read_index(&path).unwrap_err();

        assert!(err.contains("malformed index line 1"));
    });
}

#[test]
fn collision_metadata_mismatch_fails() {
    with_env("collision", |_| {
        let config = Config::from_env().unwrap();
        let note = note_for_key(&config, "expected").unwrap();
        ensure_dir(&config.root).unwrap();
        fs::write(&note.path, header("actual", &note.hash)).unwrap();
        let result = ensure_note(&config, "expected");
        assert!(result.is_err());
    });
}

#[test]
fn header_parser_rejects_unknown_escape_sequences() {
    assert_eq!(
        parse_key_from_header(r#"<!-- canon key="bad\xkey" hash="hash" -->"#),
        None
    );
}

#[cfg(unix)]
#[test]
fn git_stdout_path_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path = path_from_git_stdout(vec![b'/', b't', 0xff, b'\n']);

    assert_eq!(path.as_os_str().as_bytes(), &[b'/', b't', 0xff]);
}

#[test]
fn aliases_work() {
    with_env("aliases", |_| {
        run(vec![]).unwrap();
        run(vec!["pwd".into()]).unwrap();
        run(vec!["p".into(), "file.rs".into()]).unwrap();
        run(vec!["path".into(), "file.rs".into()]).unwrap();
        run(vec!["w".into(), "file.rs".into(), "body".into()]).unwrap();
        run(vec!["a".into(), "file.rs".into(), "more".into()]).unwrap();
        run(vec!["read".into(), "file.rs".into()]).unwrap();
        run(vec!["d".into(), "file.rs".into()]).unwrap();
        assert!(run(vec!["-r".into()]).is_err());
        assert!(run(vec!["file.rs".into()]).is_err());
    });
}

#[test]
fn init_creates_template_and_fails_when_existing() {
    let root = temp_home("init");
    run_init(&root).unwrap();
    let check_path = root.join(CHECK_PATH);
    assert_eq!(
        fs::read_to_string(&check_path).unwrap(),
        DEFAULT_CHECK_TEMPLATE
    );
    assert!(!root.join(".gitignore").exists());
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    assert!(run_init(&root).is_err());
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

#[test]
fn hook_install_creates_reusable_pre_commit_hook() {
    let root = temp_home("hook-install");
    run_hook_install(&root).unwrap();
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    assert!(!root.join(CHECK_PATH).exists());
    assert!(!root.join(".gitignore").exists());
    assert!(!DEFAULT_PRE_COMMIT_HOOK.contains("git status --porcelain -- .canon/"));
    assert_eq!(
        fs::read_to_string(&hook_path).unwrap(),
        DEFAULT_PRE_COMMIT_HOOK
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(&hook_path).unwrap().permissions().mode() & 0o111,
            0
        );
    }

    run_hook_install(&root).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_non_exact_existing_canon_pre_commit_hook() {
    let root = temp_home("hook-install-update");
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    let previous_hook = DEFAULT_PRE_COMMIT_HOOK.replace(
        "echo \"canon pre-commit: running canon gate\"",
        "if [ -n \"$(git status --porcelain -- .canon/)\" ]; then\n  echo \"canon pre-commit: .canon/ has uncommitted changes\" >&2\n  git status --porcelain -- .canon/ >&2\n  echo \"Clean .canon/ before committing.\" >&2\n  exit 1\nfi\n\necho \"canon pre-commit: running canon gate\"",
    );
    fs::write(&hook_path, previous_hook).unwrap();

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("already exists with different content"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_nonstandard_git_hooks_path() {
    let root = temp_home("hook-install-nonstandard");
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("config")
        .arg("--local")
        .arg("core.hooksPath")
        .arg(".githooks")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let err = run_hook_install(&root).unwrap_err();

    assert!(err.contains("core.hooksPath is already set to .githooks"));
    assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_install_refuses_different_existing_pre_commit_hook() {
    let root = temp_home("hook-install-existing");
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    fs::write(&hook_path, "custom hook").unwrap();

    let err = run_hook_install(&root).unwrap_err();
    assert!(err.contains("already exists with different content"));
    assert!(!root.join(CHECK_PATH).exists());
    assert!(!root.join(".gitignore").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_config_accepts_minimal_schema() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    assert_eq!(config.expectations.len(), 2);
    assert_eq!(config.agent.model.primary.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(config.agent.model.fallbacks, vec!["gpt-5.3-codex-spark"]);
    assert_eq!(config.agent.thinking, "medium");
    assert_eq!(config.agent.ignore, vec!["target/**"]);
}

#[test]
fn check_config_defaults_thinking_to_low() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#,
    )
    .unwrap();
    assert_eq!(config.agent.thinking, "low");
    assert!(parse_check_config(
        r#"
version: 1
agent:
  thinking: unsupported
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
}

#[test]
fn check_command_accepts_custom_config_option() {
    let parsed = parse_check_command_args(&[
        "--config".into(),
        "alt.yml".into(),
        "--fail-fast".into(),
        "2".into(),
    ])
    .unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("alt.yml"));
    assert_eq!(
        parsed.option_args,
        vec![OsString::from("--fail-fast"), OsString::from("2")]
    );

    let parsed = parse_check_command_args(&["-c".into(), "old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    let parsed = parse_check_command_args(&["--config=old.yml".into()]).unwrap();
    assert_eq!(parsed.config_path, PathBuf::from("old.yml"));

    assert!(parse_check_command_args(&["-c".into()]).is_err());
    assert!(
        parse_check_command_args(&["-c".into(), "a.yml".into(), "--config=b.yml".into()]).is_err()
    );
    assert!(parse_check_command_args(&["-c".into(), "../outside.yml".into()]).is_err());
    assert!(parse_check_command_args(&["-c".into(), "/tmp/outside.yml".into()]).is_err());
}

#[test]
fn check_config_rejects_missing_required_fields() {
    assert!(parse_check_config("version: 1\n").is_err());
    assert!(parse_check_config("version: 1\nagent: {}\nexpectations: []\n").is_err());
    assert!(parse_check_config(
        "version: 1\nagent:\n  instructions: x\n  ignore: []\nexpectations:\n  - q: x\n    a: y\n"
    )
    .is_err());
}

#[test]
fn check_config_rejects_unsupported_expectation_fields() {
    let yaml = r#"
	version: 1
	agent:
	  instructions: x
	  ignore: []
	  plugins: []
	expectations:
	  - id: bad
    q: "Question?"
    a: "yes"
"#;
    assert!(parse_check_config(yaml).is_err());
}

#[test]
fn selected_expectation_numbers_are_validated() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    assert_eq!(select_expectations(&config, &[]).unwrap().len(), 2);
    assert_eq!(
        select_expectations(&config, &["2".into()]).unwrap()[0].number,
        2
    );
    assert!(select_expectations(&config, &["0".into()]).is_err());
    assert!(select_expectations(&config, &["3".into()]).is_err());
    assert!(select_expectations(&config, &["1".into(), "1".into()]).is_err());
    assert!(select_expectations(&config, &["x".into()]).is_err());
}

#[test]
fn check_options_accept_fail_fast_with_selected_numbers() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = parse_check_options(&config, &["--fail-fast".into(), "2".into()]).unwrap();
    assert!(options.fail_fast);
    assert_eq!(options.selected.len(), 1);
    assert_eq!(options.selected[0].number, 2);
    assert!(parse_check_options(&config, &["--fail-fast".into(), "--fail-fast".into()]).is_err());
}

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
fn parser_handles_json_answer_and_free_form_evidence() {
    let parsed = parse_evaluator_response(
            r#"{"answer":"yes","evidence":"line: one\nSCOPE: this is evidence\nANSWER: also evidence","scope":["."]}"#,
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap();
    assert_eq!(parsed.answer, "yes");
    assert_eq!(
        parsed.evidence,
        "line: one\nSCOPE: this is evidence\nANSWER: also evidence"
    );
    assert_eq!(parsed.scope, vec!["."]);
    let canonicalized = parse_evaluator_response(
        r#"{"answer":"no","evidence":"code says no","scope":["src/check.rs","src"]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .unwrap();
    assert_eq!(canonicalized.answer, "no");
    assert_eq!(canonicalized.scope, vec!["src"]);
    assert!(parse_evaluator_response(
        r#"I checked the files first. {"answer":"yes","evidence":"README.md has evidence","scope":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "ANSWER: yes\nEVIDENCE:\nok\nSCOPE: [\".\"]",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        r#"{"answer":"yes\nno","evidence":"bad","scope":["."]}"#,
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
    assert!(parse_evaluator_response(
        "yes",
        &parse_check_config(check_config_yaml()).unwrap().agent,
    )
    .is_err());
}

#[test]
fn check_runner_hides_expected_answers_and_reuses_session() {
    let root = git_project("check-runner");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_roots, vec![root.clone()]);
    assert_eq!(
        runner.start_ignores,
        vec![vec![
            ".canon".to_string(),
            ".canon/**".to_string(),
            ".git/canon".to_string(),
            ".git/canon/**".to_string(),
            "target/**".to_string()
        ]]
    );
    assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
    assert_eq!(runner.start_models, vec![Some("gpt-5.4-mini".to_string())]);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
    assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
    assert!(runner
        .prompts
        .iter()
        .all(|prompt| !prompt.contains("Response format:")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_verifies_narrowed_scope_before_history_reuse() {
    let root = git_project("check-narrowing-accepted");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("yes", "src/main.rs still supports it", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records[0].passed());
    assert_eq!(records[0].scope, vec!["src/main.rs"]);
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec!["src/main.rs".to_string()]]
    );
    assert_eq!(
        read_history_records(&root, &options.selected[0])
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_dir_all(root);

    let root = git_project("check-narrowing-rejected");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "full scope supports it", &["src/main.rs"]),
        &answer("no", "src/main.rs changes the answer", &["src/main.rs"]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records[0].passed());
    assert_eq!(records[0].observed, "yes");
    assert_eq!(records[0].scope, vec!["src/main.rs"]);
    let history = read_history_records(&root, &options.selected[0]).unwrap();
    assert!(history.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_fails_mismatch_and_treats_idk_as_exact_string() {
    let root = git_project("check-fails");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("idk", "not enough", &["."]),
        &answer("yes", "wrong", &["."]),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records[0].passed());
    assert!(!records[1].passed());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_repairs_absence_question_idk_once() {
    let root = git_project("check-absence-idk");
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "Are there any unused files?"
    a: "no"
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("idk", "no concrete issue found", &["."]),
        &answer("no", "README.md and src/main.rs were inspected", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records[0].passed());
    assert_eq!(records[0].observed, "no");
    assert_eq!(runner.prompts[1], runner.prompts[0]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_fail_fast_stops_after_first_failure() {
    let root = git_project("check-fail-fast");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], true, true);
    let mut runner = FakeRunner::new(&[&answer("no", "wrong", &["."])]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert_eq!(records.len(), 1);
    assert!(!records[0].passed());
    assert_eq!(runner.prompts.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_repairs_malformed_response_once() {
    let root = git_project("check-repair");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let repaired = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new(&["not parseable", &repaired]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records[0].passed());
    assert_eq!(runner.prompts.len(), 2);
    assert_eq!(runner.prompts[1], runner.prompts[0]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_uses_model_fallback_after_usage_limit() {
    let root = git_project("check-model-fallback");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let answer = answer("yes", "README.md", &["."]);
    let mut runner = FakeRunner::new_results(vec![
        Err("app-server turn/start failed: usageLimitExceeded"),
        Ok(&answer),
    ]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records[0].passed());
    assert_eq!(
        runner.start_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.3-codex-spark".to_string())
        ]
    );
    assert_eq!(
        runner.start_scopes,
        vec![vec![".".to_string()], vec![".".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_marks_unparseable_after_response_repair_fails() {
    let root = git_project("check-unparseable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&["", ""]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records[0].passed());
    assert_eq!(records[0].observed, UNPARSEABLE_OBSERVED);
    assert!(records[0].evidence.contains("first response: <empty>"));
    assert!(read_history_records(&root, &options.selected[0])
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_warns_when_evidence_stays_empty() {
    let root = git_project("check-empty-evidence");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &answer("yes", "", &["."])]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(records[0].passed());
    assert!(records[0].evidence.is_empty());
    assert_eq!(runner.prompts.len(), 2);
    assert_eq!(runner.prompts[1], runner.prompts[0]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_requires_human_review_for_malformed_answer() {
    let root = git_project("check-malformed-answer");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let malformed = answer("malformed", "question is malformed", &["."]);
    let mut runner = FakeRunner::new(&[&malformed, &malformed, &malformed, &malformed]);
    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();
    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "malformed");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_replaces_restricted_idk_with_full_scope_answer() {
    let root = git_project("check-restricted-idk");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "pass".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "fail".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "idk".to_string(),
            evidence: "src/main.rs was not enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "src/main.rs was not enough", &["src/main.rs"]),
        &answer("yes", "README.md and src/main.rs answer it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records[0].passed());
    assert_eq!(records[0].observed, "yes");
    assert_eq!(
        runner.start_scopes,
        vec![vec!["src/main.rs".to_string()], vec![".".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_starts_from_latest_reusable_history_scope_even_when_failed() {
    let root = git_project("check-failed-history-scope");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "fail".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "no".to_string(),
            evidence: "restricted scope was misleading".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[&answer(
        "yes",
        "restricted scope now answers it",
        &["src/main.rs"],
    )]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records[0].passed());
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_verifies_narrowed_scope_after_restricted_idk_widens() {
    let root = git_project("check-restricted-idk-narrows");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "pass".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "fail".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "idk".to_string(),
            evidence: "src/main.rs was not enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("idk", "src/main.rs was not enough", &["src/main.rs"]),
        &answer("yes", "src is enough", &["src"]),
        &answer("yes", "src still answers it", &["src"]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records[0].passed());
    assert_eq!(records[0].observed, "yes");
    assert_eq!(records[0].scope, vec!["src".to_string()]);
    assert_eq!(
        runner.start_scopes,
        vec![
            vec!["src/main.rs".to_string()],
            vec![".".to_string()],
            vec!["src".to_string()]
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_widen_restricted_answer_mismatch() {
    let root = git_project("check-restricted-failure");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "pass".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&[
        &answer("no", "src/main.rs was misleading", &["src/main.rs"]),
        &answer("yes", "full project context answers it", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "no");
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_does_not_widen_restricted_unparseable_response() {
    let root = git_project("check-restricted-unparseable");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let expectation = options.selected[0].clone();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "pass".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "src/main.rs was previously enough".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number: expectation.number,
            result: "fail".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "malformed".to_string(),
            evidence: "restricted response was empty".to_string(),
            scope: vec!["src/main.rs".to_string()],
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    let mut runner = FakeRunner::new(&["", ""]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, UNPARSEABLE_OBSERVED);
    assert_eq!(runner.start_scopes, vec![vec!["src/main.rs".to_string()]]);
    assert_eq!(read_history_records(&root, &expectation).unwrap().len(), 2);
    let _ = fs::remove_dir_all(root);
}

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
            result: "pass".to_string(),
            prompt: "old prompt text".to_string(),
            expected: "old expected".to_string(),
            observed: "yes".to_string(),
            evidence: "cached answer".to_string(),
            scope: full_scope(),
            scope_hash: staged_scope_hash(&root, &config.agent, &full_scope()).unwrap(),
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
fn cooldown_reuse_stops_at_latest_reusable_failure() {
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
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:10Z".to_string(),
            number: 1,
            result: "pass".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "yes".to_string(),
            evidence: "old pass".to_string(),
            scope: full_scope(),
            scope_hash: "old".to_string(),
        },
    )
    .unwrap();
    append_history_record(
        &root,
        &expectation,
        &CheckRecord {
            timestamp: "1970-01-01T00:00:20Z".to_string(),
            number: 1,
            result: "fail".to_string(),
            prompt: expectation.q.clone(),
            expected: expectation.a.clone(),
            observed: "no".to_string(),
            evidence: "latest fail".to_string(),
            scope: full_scope(),
            scope_hash: "new".to_string(),
        },
    )
    .unwrap();
    let mut history_cache = HistoryCache::new();
    assert!(
        cooldown_history_record(&root, &expectation, &mut history_cache, 30)
            .unwrap()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_git_path_uses_expectation_id_directory() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let mut options = check_options(&config, &["1"], false, true);
    let expectation = options.selected.remove(0);
    assert_eq!(
        history_git_path(&expectation),
        format!("{}/{}/history.jsonl", GIT_CANON_CACHE_DIR, expectation.id)
    );
}

#[test]
fn malformed_history_json_lines_are_ignored() {
    let root = git_project("history-malformed-json");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let expectation = check_options(&config, &["1"], false, true).selected[0].clone();
    let path = history_path(&root, &expectation).unwrap();
    ensure_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, "{not json}\n").unwrap();

    let records = read_history_records(&root, &expectation).unwrap();

    assert!(records.is_empty());
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
    fs::write(
        &path,
        "{\"n\":1}\nnot json\n{\"n\":2}\n{\"n\":3}\n{\"n\":4}\n{\"n\":5}\n{\"n\":6}\n{\"n\":7}\n",
    )
    .unwrap();

    compact_history(&path).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\"n\":3}\n{\"n\":4}\n{\"n\":5}\n{\"n\":6}\n{\"n\":7}\n"
    );
    assert!(!compact_history_temp_path(&path).unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_untracked_stash_parent_is_not_restore_failure() {
    let root = git_project("stash-no-untracked-parent");
    commit_all(&root, "initial");

    assert!(!git_revision_exists(&root, "HEAD^3").unwrap());
    assert!(restore_untracked_from_stash(&root, "HEAD").is_ok());
    let _ = fs::remove_dir_all(root);
}

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
        &expectation_record(&expectation, "pass", "yes", scope_hash),
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

    assert!(result.is_err());
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
        &expectation_record(&expectation, "fail", "no", current_hash),
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
        &expectation_record(&expectation, "fail", "no", head_hash),
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
        &expectation_record(&expectation, "fail", "no", current_hash),
    )
    .unwrap();

    let result = run_gate_command(&root, &[OsString::from("1")]);

    assert!(result.is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_keeps_semantic_malformed_as_human_review_failure() {
    let root = git_project("check-full-malformed");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("malformed", "full scope response stayed malformed", &["."]),
        &answer("malformed", "question is malformed", &["."]),
        &answer("malformed", "question is malformed", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(!records[0].passed());
    assert_eq!(records[0].observed, "malformed");
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    let _ = fs::remove_dir_all(root);
}

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
    assert_eq!(json["number"], 1);
    assert_eq!(json["prompt"], "Question?");
    assert_eq!(json["expected"], "yes");
    assert_eq!(json["observed"], "yes");
    assert_eq!(json["evidence"], "README.md has evidence");
    assert_eq!(json["scope"], json!(["."]));
    assert_eq!(json["scopeHash"], "AAAAAAAAAAAAAAAAAAAA");
    let expected_order = [
        "\"timestamp\"",
        "\"number\"",
        "\"result\"",
        "\"prompt\"",
        "\"expected\"",
        "\"observed\"",
        "\"evidence\"",
        "\"scope\"",
        "\"scopeHash\"",
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
    fs::write(
        log_dir.join("0.jsonl"),
        "x".repeat((DIAGNOSTIC_LOG_MAX_BYTES + 1) as usize),
    )
    .unwrap();
    fs::write(log_dir.join("1.jsonl"), "one").unwrap();
    fs::write(log_dir.join("2.jsonl"), "two").unwrap();
    fs::write(log_dir.join("3.jsonl"), "three").unwrap();

    let writer = DiagnosticLogWriter::create(&root).unwrap();
    assert_eq!(writer.path, log_dir.join("0.jsonl"));
    assert!(!log_dir.join("0.jsonl").exists());
    assert_eq!(
        fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
        (DIAGNOSTIC_LOG_MAX_BYTES + 1) as usize
    );
    assert_eq!(fs::read_to_string(log_dir.join("2.jsonl")).unwrap(), "one");
    assert_eq!(fs::read_to_string(log_dir.join("3.jsonl")).unwrap(), "two");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn scope_is_canonicalized() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let scope = parse_scope_json(
        r#"["src/main.rs", "README.md", "src", "README.md"]"#,
        &config.agent,
    )
    .unwrap();
    assert_eq!(scope, vec!["README.md", "src"]);
    let many_paths = parse_scope_json(r#"["a", "b", "c", "d", "e"]"#, &config.agent).unwrap();
    assert_eq!(many_paths, vec!["a", "b", "c", "d", "e"]);
    assert!(parse_scope_json(r#"["target/output.txt"]"#, &config.agent).is_err());
}

#[test]
fn evaluator_response_scope_ignores_denied_paths() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let only_denied = parse_scope_strings(&[".canon/check.yml".to_string()], &config.agent)
        .expect("denied response scope should not make the answer unparseable");
    assert_eq!(only_denied, full_scope());
    let mixed = parse_scope_strings(
        &["src/main.rs".to_string(), "target/output.txt".to_string()],
        &config.agent,
    )
    .unwrap();
    assert_eq!(mixed, vec!["src/main.rs"]);
}

#[test]
fn check_runner_streams_result_output() {
    let root = git_project("check-output");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let mut output = FlushCountingWriter::new();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        None,
        Some(&mut output),
    )
    .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(output.flushes, 2);
    let lines = String::from_utf8(output.bytes).unwrap();
    assert_eq!(lines.lines().count(), 2);
    assert!(lines.contains("1. OK"));
    assert!(lines.contains("2. OK"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn question_prompt_includes_only_current_context() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let prompt = question_prompt("Permission question?", &full_scope()).unwrap();
    assert_eq!(prompt, "Permission question?");
    assert!(!prompt.contains("Response format:"));
    assert!(!prompt.contains("ANSWER: <single-line answer>"));
    assert!(!prompt.contains("Instructions:"));
    assert!(!prompt.contains(config.agent.instructions.trim()));
    assert!(!prompt.contains("Current context:"));
    assert!(!prompt.contains("\nQuestion:\n"));
    assert!(!prompt.contains("QUESTION:"));
    assert!(!prompt.contains("\nExpectation:\n"));
    assert!(!prompt.contains("Runtime canon metadata"));
    assert!(!prompt.contains("repository pre-commit hook"));
    assert!(!prompt.contains("core.hooksPath"));
    assert!(!prompt.contains("evaluator default permission profile"));
}

#[test]
fn evaluator_turn_input_is_plain_question_string() {
    let prompt = question_prompt("Permission question?", &["src".to_string()]).unwrap();
    let input = evaluator_turn_input(&prompt).unwrap();
    assert_eq!(input, json!("Permission question?"));
    assert_eq!(render_evaluator_turn_input(&input).unwrap(), prompt);
}

#[test]
fn absence_repair_detection_reads_json_question_prompt() {
    assert!(should_repair_absence_idk(
        &question_prompt("Are there any unused files?", &full_scope()).unwrap()
    ));
    assert!(!should_repair_absence_idk(
        &question_prompt("Does README exist?", &full_scope()).unwrap()
    ));
    assert!(should_repair_absence_idk("Are there any unused files?"));
}

#[test]
fn developer_instructions_include_agent_instructions_and_response_format() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let instructions = developer_instructions(&config.agent, &full_scope());
    assert!(instructions.contains(config.agent.instructions.trim()));
    assert!(instructions.contains("Response format:\nReturn exactly one valid JSON object"));
    assert!(instructions.contains(r#""answer":"<single-line answer>""#));
    assert!(instructions.contains(r#""scope":["<normalized repository-relative path>"]"#));
}

#[test]
fn evaluator_permissions_always_deny_canon_and_agent_ignores() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: "Answer from files only.".to_string(),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let config = evaluator_thread_config(&agent, &full_scope(), None, &agent.thinking);
    let root_permissions = config["permissions"]["canon_check"]["filesystem"][":project_roots"]
        .as_object()
        .unwrap();
    assert_eq!(root_permissions["."], "read");
    assert_eq!(root_permissions[".canon"], "none");
    assert_eq!(root_permissions[".canon/**"], "none");
    assert_eq!(root_permissions[".git/canon"], "none");
    assert_eq!(root_permissions[".git/canon/**"], "none");
    assert_eq!(root_permissions["target"], "none");
    assert_eq!(root_permissions["target/**"], "none");
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":root"],
        "read"
    );
    assert_eq!(config["model_reasoning_effort"], "low");
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/tmp/**"],
        "read"
    );
    assert_eq!(config["history"]["persistence"], "none");
    assert!(config.get("plugins").is_none());
}

#[test]
fn restricted_evaluator_scope_is_enforced_by_filesystem_permissions() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: "Answer from files only.".to_string(),
        ignore: vec!["target/**".to_string()],
        plugins: Vec::new(),
    };
    let root_permissions = evaluator_thread_root_permissions(&agent, &["src".to_string()]);

    assert_eq!(root_permissions["."], "none");
    assert_eq!(root_permissions["src"], "read");
    assert_eq!(root_permissions["src/**"], "read");
    assert_eq!(root_permissions[".canon"], "none");
    assert_eq!(root_permissions[".canon/**"], "none");
    assert_eq!(root_permissions["target"], "none");
    assert_eq!(root_permissions["target/**"], "none");
}

#[test]
fn completed_agent_message_text_is_turn_text_fallback() {
    let message = json!({
        "method": "item/completed",
        "params": {
            "item": {
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "ANSWER: yes\n" },
                    { "type": "output_text", "text": "EVIDENCE:\nok\nSCOPE: [\".\"]" }
                ]
            }
        }
    });
    let mut completed_text = String::new();
    append_completed_agent_text(&message, &mut completed_text);
    assert_eq!(
        turn_text(String::new(), completed_text),
        "ANSWER: yes\nEVIDENCE:\nok\nSCOPE: [\".\"]"
    );
    assert_eq!(
        turn_text("ANSWER: no".to_string(), "ANSWER: yes".to_string()),
        "ANSWER: no"
    );
}

#[test]
fn app_server_error_message_is_extracted() {
    let message = json!({
        "method": "error",
        "params": {
            "error": {
                "message": "You've hit your usage limit for GPT-5.3-Codex-Spark."
            }
        }
    });
    assert_eq!(
        app_server_error_message(&message).unwrap(),
        "You've hit your usage limit for GPT-5.3-Codex-Spark."
    );

    let turn_completed = json!({
        "method": "turn/completed",
        "params": {
            "turn": {
                "status": "failed",
                "error": {
                    "message": "model unavailable"
                }
            }
        }
    });
    assert_eq!(
        app_server_error_message(&turn_completed).unwrap(),
        "model unavailable"
    );
}

#[test]
fn token_usage_update_is_rendered_like_codex_summary() {
    let message = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "turnId": "turn-1",
            "tokenUsage": {
                "last": {
                    "totalTokens": 69748,
                    "inputTokens": 63574,
                    "cachedInputTokens": 361216,
                    "outputTokens": 6174,
                    "reasoningOutputTokens": 2911
                }
            }
        }
    });
    let (turn_id, usage) = token_usage_update(&message).unwrap();
    assert_eq!(turn_id, "turn-1");
    assert_eq!(
        render_token_usage_summary(usage),
        "Token usage: total=69,748 input=63,574 (+ 361,216 cached) output=6,174 (reasoning 2,911)"
    );
    assert_eq!(
        render_token_usage_summary(TokenUsage::default()),
        "Token usage: total=0 input=0 (+ 0 cached) output=0 (reasoning 0)"
    );
}

#[test]
fn evaluator_model_is_configured_when_present() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let thread_config =
        evaluator_thread_config(&config.agent, &full_scope(), None, &config.agent.thinking);
    assert_eq!(thread_config["model"], "gpt-5.4-mini");
    let fallback_config = evaluator_thread_config(
        &config.agent,
        &full_scope(),
        Some("gpt-5.3-codex-spark"),
        &config.agent.thinking,
    );
    assert_eq!(fallback_config["model"], "gpt-5.3-codex-spark");
}

#[test]
fn evaluator_plugin_list_is_explicitly_configured() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins:
    - "canon@codex-plugins"
expectations:
  - q: "Question?"
    a: "yes"
"#,
    )
    .unwrap();
    assert!(check_config_loads_plugins(&config));
    let thread_config =
        evaluator_thread_config(&config.agent, &full_scope(), None, &config.agent.thinking);
    assert_eq!(
        thread_config["plugins"]["canon@codex-plugins"]["enabled"],
        json!(true)
    );
}

#[test]
fn app_server_starts_with_plugins_disabled_by_default() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let disabled = app_server_args(false, &config.agent, Some("gpt-5.3-codex-spark"));
    assert_eq!(&disabled[..3], ["app-server", "--disable", "plugins"]);
    assert_eq!(&disabled[disabled.len() - 2..], ["--listen", "stdio://"]);
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "default_permissions=\"canon_check\""]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "model=\"gpt-5.3-codex-spark\""]));
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "model_reasoning_effort=\"medium\""]));
    let filesystem_arg = disabled
        .windows(2)
        .find_map(|pair| {
            (pair[0] == "-c" && pair[1].starts_with("permissions.canon_check.filesystem="))
                .then_some(pair[1].as_str())
        })
        .unwrap();
    assert!(filesystem_arg.contains(r#"":project_roots"={"."="none""#));
    assert!(filesystem_arg.contains(r#"".canon/**"="none""#));
    assert!(filesystem_arg.contains(r#""target"="none""#));
    assert!(filesystem_arg.contains(r#""target/**"="none""#));
    assert!(filesystem_arg.contains(r#"":root"="read""#));
    assert!(filesystem_arg.contains(r#""glob_scan_max_depth"=32"#));
    assert!(!filesystem_arg.contains(r#""."="read""#));

    let enabled = app_server_args(true, &config.agent, None);
    assert_eq!(enabled.first().map(String::as_str), Some("app-server"));
    assert!(!enabled.iter().any(|arg| arg == "--disable"));
    assert_eq!(&enabled[enabled.len() - 2..], ["--listen", "stdio://"]);
}
