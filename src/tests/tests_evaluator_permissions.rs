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
    let disabled = app_server_args(false, &config.agent);
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
    assert!(filesystem_arg.contains(r#""glob_scan_max_depth"=32"#));
    assert!(!filesystem_arg.contains(r#""."="read""#));

    let enabled = app_server_args(true, &config.agent);
    assert_eq!(enabled.first().map(String::as_str), Some("app-server"));
    assert!(!enabled.iter().any(|arg| arg == "--disable"));
    assert_eq!(&enabled[enabled.len() - 2..], ["--listen", "stdio://"]);
}
