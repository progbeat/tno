use super::*;

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
    assert_eq!(root_permissions[".git/canon/logs"], "none");
    assert_eq!(root_permissions[".git/canon/logs/**"], "none");
    assert_eq!(root_permissions["target"], "none");
    assert_eq!(root_permissions["target/**"], "none");
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"][":root"],
        "read"
    );
    assert_eq!(config["model_reasoning_effort"], "low");
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get("~/.codex/tmp/**")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get(":tmpdir")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get(":slash_tmp")
        .is_none());
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .get("/private/tmp/**")
        .is_none());
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/sessions/**"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories"],
        "none"
    );
    assert_eq!(
        config["permissions"]["canon_check"]["filesystem"]["~/.codex/memories/**"],
        "none"
    );
    assert!(config["permissions"]["canon_check"]["filesystem"]
        .as_object()
        .unwrap()
        .values()
        .all(|permission| permission != "write"));
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
    let root = git_project("app-server-args-default");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let disabled = app_server_args(&root, false, &config.agent).unwrap();
    assert_eq!(&disabled[..3], ["app-server", "--disable", "plugins"]);
    assert_eq!(&disabled[disabled.len() - 2..], ["--listen", "stdio://"]);
    assert!(disabled
        .windows(2)
        .any(|pair| pair == ["-c", "default_permissions=\"canon_check\""]));
    assert!(!disabled
        .windows(2)
        .any(|pair| pair[0] == "-c" && pair[1].starts_with("model=")));
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
    assert!(!filesystem_arg.contains(r#"":tmpdir""#));
    assert!(!filesystem_arg.contains(r#"":slash_tmp""#));
    assert!(!filesystem_arg.contains(r#""/private/tmp/**""#));
    assert!(!filesystem_arg.contains(r#""~/.codex/tmp/**""#));
    assert!(filesystem_arg.contains(r#""glob_scan_max_depth"=32"#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/sessions/**"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories"="none""#));
    assert!(filesystem_arg.contains(r#""~/.codex/memories/**"="none""#));
    assert!(!filesystem_arg.contains(r#""write""#));
    assert!(!filesystem_arg.contains(r#""."="read""#));
    assert!(disabled
        .windows(2)
        .any(|pair| { pair == ["-c", "thread_reuse.carryover_token_target=[10000,30000]",] }));

    let enabled = app_server_args(&root, true, &config.agent).unwrap();
    assert_eq!(enabled.first().map(String::as_str), Some("app-server"));
    assert!(!enabled.iter().any(|arg| arg == "--disable"));
    assert_eq!(&enabled[enabled.len() - 2..], ["--listen", "stdio://"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn app_server_startup_config_escapes_toml_control_characters() {
    let agent = AgentConfig {
        model: ModelConfig::default(),
        thinking: "low".to_string(),
        instructions: "Answer from files only.".to_string(),
        ignore: vec![
            "quoted\"path/**".to_string(),
            "control\u{0007}path/**".to_string(),
            "delete\u{007f}path/**".to_string(),
        ],
        plugins: Vec::new(),
    };

    let filesystem_arg = app_server_startup_filesystem_arg(&agent);

    assert!(filesystem_arg.contains(r#""quoted\"path"="none""#));
    assert!(filesystem_arg.contains(r#""control\u0007path"="none""#));
    assert!(filesystem_arg.contains(r#""delete\u007Fpath"="none""#));
    assert!(!filesystem_arg.contains('\u{0007}'));
    assert!(!filesystem_arg.contains('\u{007f}'));
}

#[test]
fn thread_reuse_config_reads_git_carryover_token_target() {
    let root = git_project("thread-reuse-config");
    let output = Command::new("git")
        .args([
            "config",
            "canon.threadReuse.carryoverTokenTarget",
            "12000,24000",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());

    let config = thread_reuse_config(&root).unwrap();

    assert_eq!(config.carryover_token_target.min, 12_000);
    assert_eq!(config.carryover_token_target.max, 24_000);
    assert_eq!(
        thread_reuse_carryover_token_target_arg(&config),
        "thread_reuse.carryover_token_target=[12000,24000]"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn thread_reuse_config_defaults_and_validates_carryover_token_target() {
    let root = git_project("thread-reuse-config-default");
    assert_eq!(
        thread_reuse_config(&root).unwrap(),
        DEFAULT_THREAD_REUSE_CONFIG
    );

    assert!(parse_carryover_token_target("30000,10000")
        .unwrap_err()
        .contains("MIN"));
    assert!(parse_carryover_token_target("10000")
        .unwrap_err()
        .contains("MIN,MAX"));
    assert!(parse_carryover_token_target("0,10000")
        .unwrap_err()
        .contains("greater than zero"));
    let _ = fs::remove_dir_all(root);
}
