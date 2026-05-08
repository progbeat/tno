#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(name: &str) -> PathBuf {
        let mut path = PathBuf::from("/tmp");
        path.push(format!("canon-test-{}-{}", name, process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn with_env<F>(name: &str, f: F)
    where
        F: FnOnce(PathBuf),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = temp_home(name);
        env::set_var("CANON_HOME", &home);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(home.clone());
        env::remove_var("CANON_HOME");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(home);
    }

    fn with_tmpdir<F>(name: &str, f: F)
    where
        F: FnOnce(PathBuf),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = temp_home(name);
        env::remove_var("CANON_HOME");
        env::set_var("TMPDIR", &temp);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(temp.clone());
        env::remove_var("TMPDIR");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(temp);
    }

    fn check_config_yaml() -> &'static str {
        r#"
version: 1
agent:
  model: gpt-5.3-codex-spark
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
        let config: CheckConfig = serde_yaml::from_str(yaml).map_err(|err| err.to_string())?;
        validate_check_config(&config)?;
        Ok(config)
    }

    struct FakeRunner {
        answers: VecDeque<String>,
        prompts: Vec<String>,
        sessions: Vec<String>,
        start_roots: Vec<PathBuf>,
        start_ignores: Vec<Vec<String>>,
        start_models: Vec<Option<String>>,
        start_plugins: Vec<Vec<String>>,
        start_scopes: Vec<Vec<String>>,
        starts: usize,
    }

    impl FakeRunner {
        fn new(answers: &[&str]) -> FakeRunner {
            FakeRunner {
                answers: answers.iter().map(|answer| answer.to_string()).collect(),
                prompts: Vec::new(),
                sessions: Vec::new(),
                start_roots: Vec::new(),
                start_ignores: Vec::new(),
                start_models: Vec::new(),
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
            scope: &[String],
        ) -> Result<String, String> {
            self.starts += 1;
            self.start_roots.push(root.to_path_buf());
            self.start_ignores.push(effective_ignore_patterns(agent));
            self.start_models.push(agent.model.clone());
            self.start_plugins.push(agent.plugins.clone());
            self.start_scopes.push(scope.to_vec());
            Ok(format!("session-{}", self.starts))
        }

        fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
            self.sessions.push(session_id.to_string());
            self.prompts.push(prompt.to_string());
            self.answers
                .pop_front()
                .ok_or("fake runner has no answer".to_string())
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
        format!(
            "ANSWER: {}\nEVIDENCE:\n{}\nSCOPE: {}",
            answer,
            evidence,
            serde_json::to_string(scope).unwrap()
        )
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
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CODEX_THREAD_ID");
        env::set_var("CANON_HOME", temp_home("missing-thread"));
        let result = Config::from_env();
        assert!(result.is_err());
        env::remove_var("CANON_HOME");
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
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CANON_HOME");
        env::remove_var("TMPDIR");
        env::set_var("CODEX_THREAD_ID", "thread-test");
        let config = Config::from_env().unwrap();
        assert_eq!(config.root, PathBuf::from("/tmp/canon/codex/thread-test"));
        env::remove_var("CODEX_THREAD_ID");
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
            let note = note_for_key(&config, "src/main.rs");
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
    fn collision_metadata_mismatch_fails() {
        with_env("collision", |_| {
            let config = Config::from_env().unwrap();
            let note = note_for_key(&config, "expected");
            ensure_dir(&config.root).unwrap();
            fs::write(&note.path, header("actual", &note.hash)).unwrap();
            let result = ensure_note(&config, "expected");
            assert!(result.is_err());
        });
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
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CODEX_THREAD_ID");
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
        assert_eq!(config.agent.model.as_deref(), Some("gpt-5.3-codex-spark"));
        assert_eq!(config.agent.ignore, vec!["target/**"]);
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
            parse_check_command_args(&["-c".into(), "a.yml".into(), "--config=b.yml".into()])
                .is_err()
        );
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
        assert!(
            parse_check_options(&config, &["--fail-fast".into(), "--fail-fast".into()]).is_err()
        );
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
    fn parser_handles_answer_and_free_form_evidence() {
        let parsed = parse_evaluator_response(
            "ANSWER: yes\nEVIDENCE:\nline: one\n- two\nSCOPE: [\".\"]",
            &parse_check_config(check_config_yaml()).unwrap().agent,
        )
        .unwrap();
        assert_eq!(parsed.answer, "yes");
        assert_eq!(parsed.evidence, "line: one\n- two");
        assert_eq!(parsed.scope, vec!["."]);
        assert!(parse_evaluator_response(
            "yes",
            &parse_check_config(check_config_yaml()).unwrap().agent
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
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
        assert!(records.iter().all(CheckRecord::passed));
        assert_eq!(runner.starts, 1);
        assert_eq!(runner.start_roots, vec![root.clone()]);
        assert_eq!(
            runner.start_ignores,
            vec![vec![
                ".canon".to_string(),
                ".canon/**".to_string(),
                ".git".to_string(),
                ".git/**".to_string(),
                "target/**".to_string()
            ]]
        );
        assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
        assert_eq!(
            runner.start_models,
            vec![Some("gpt-5.3-codex-spark".to_string())]
        );
        assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
        assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
        assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
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
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
        assert!(!records[0].passed());
        assert!(!records[1].passed());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_fail_fast_stops_after_first_failure() {
        let root = git_project("check-fail-fast");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1", "2"], true, true);
        let mut runner = FakeRunner::new(&[&answer("no", "wrong", &["."])]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
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
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
        assert!(records[0].passed());
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("could not be parsed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_warns_when_evidence_stays_empty() {
        let root = git_project("check-empty-evidence");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1"], false, true);
        let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &answer("yes", "", &["."])]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
        assert!(records[0].passed());
        assert!(records[0].evidence.is_empty());
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("no evidence"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_requires_human_review_for_malformed_answer() {
        let root = git_project("check-malformed-answer");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1"], false, true);
        let malformed = answer("malformed", "question is malformed", &["."]);
        let mut runner = FakeRunner::new(&[&malformed, &malformed]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None)
                .unwrap();
        assert!(!records[0].passed());
        assert_eq!(records[0].observed, "malformed");
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
        assert!(parse_scope_json(r#"["target/output.txt"]"#, &config.agent).is_err());
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
        let mut output = Vec::new();
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
        let lines = String::from_utf8(output).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert!(lines.contains("\"number\":1"));
        assert!(lines.contains("\"number\":2"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn evaluator_permissions_always_deny_canon_and_agent_ignores() {
        let agent = AgentConfig {
            model: None,
            instructions: "Answer from files only.".to_string(),
            ignore: vec!["target/**".to_string()],
            plugins: Vec::new(),
        };
        let config = evaluator_thread_config(&agent, &full_scope());
        let root_permissions = config["permissions"]["canon_check"]["filesystem"][":project_roots"]
            .as_object()
            .unwrap();
        assert_eq!(root_permissions["."], "read");
        assert_eq!(root_permissions[".canon"], "none");
        assert_eq!(root_permissions[".canon/**"], "none");
        assert_eq!(root_permissions[".git"], "none");
        assert_eq!(root_permissions[".git/**"], "none");
        assert_eq!(root_permissions["target/**"], "none");
        assert_eq!(
            config["permissions"]["canon_check"]["filesystem"]["/"],
            "read"
        );
        assert_eq!(
            config["permissions"]["canon_check"]["filesystem"]["~/.codex/tmp/**"],
            "read"
        );
        assert_eq!(config["history"]["persistence"], "none");
        assert!(config.get("plugins").is_none());
    }

    #[test]
    fn evaluator_model_is_configured_when_present() {
        let config = parse_check_config(check_config_yaml()).unwrap();
        let thread_config = evaluator_thread_config(&config.agent, &full_scope());
        assert_eq!(thread_config["model"], "gpt-5.3-codex-spark");
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
        let thread_config = evaluator_thread_config(&config.agent, &full_scope());
        assert_eq!(
            thread_config["plugins"]["canon@codex-plugins"]["enabled"],
            json!(true)
        );
    }

    #[test]
    fn app_server_starts_with_plugins_disabled_by_default() {
        assert_eq!(
            app_server_args(false),
            vec!["app-server", "--disable", "plugins", "--listen", "stdio://"]
        );
        assert_eq!(
            app_server_args(true),
            vec!["app-server", "--listen", "stdio://"]
        );
    }
}
