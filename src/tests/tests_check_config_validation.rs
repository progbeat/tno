use super::*;

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
fn check_config_accepts_agent_ignore_wildcards() {
    let config = parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore:
    - "logs/*"
  plugins: []
expectations:
  - q: x
    a: y
"#,
    )
    .unwrap();
    assert_eq!(config.agent.ignore, vec!["logs/*"]);
}

#[test]
fn check_config_rejects_cooldown_with_surrounding_whitespace() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
    cooldown: " 1d "
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: malformed
"#
    )
    .is_err());
}

#[test]
fn cooldown_parser_rejects_non_ascii_unit_without_panicking() {
    assert!(parse_cooldown("1д").is_err());
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
fn check_config_rejects_blank_agent_instructions() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "   "
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\u200B"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\uFE0F"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\uFFF9"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\U00013430"
  ignore: []
  plugins: []
expectations:
  - q: x
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: "\u2800"
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
fn check_config_rejects_visually_blank_expectation_questions() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "\u200B"
    a: y
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: "\u180B"
    a: y
"#
    )
    .is_err());
}

#[test]
fn check_config_rejects_impossible_expected_answers() {
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: maybe
"#
    )
    .is_err());
    assert!(parse_check_config(
        r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - q: x
    a: idk
"#
    )
    .is_err());
}

#[test]
fn check_config_rejects_extra_expectation_fields() {
    let yaml = r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins: []
expectations:
  - id: extra
    q: "Question?"
    a: "yes"
"#;
    let err = parse_check_config(yaml).unwrap_err();
    assert!(err.contains("unknown field"));
    assert!(err.contains("id"));
}

#[test]
fn plugin_keys_must_have_exactly_one_nonempty_separator() {
    assert!(validate_plugin_config_key("plugin@marketplace").is_ok());
    assert!(validate_plugin_config_key("@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@").is_err());
    assert!(validate_plugin_config_key("plugin@market@extra").is_err());
    assert!(validate_plugin_config_key(" plugin@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@marketplace ").is_err());
    assert!(validate_plugin_config_key("plugin @marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@ marketplace").is_err());
    assert!(validate_plugin_config_key("foo/bar@marketplace").is_err());
    assert!(validate_plugin_config_key("MyPlugin@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin@openai_curated").is_err());
    assert!(validate_plugin_config_key("foo--bar@marketplace").is_err());
    assert!(validate_plugin_config_key("plugin-1@openai-curated").is_ok());
}

#[test]
fn model_ids_must_not_have_surrounding_whitespace() {
    assert!(validate_optional_model(Some("gpt-5.4-mini"), "agent.model.primary").is_ok());
    assert!(validate_optional_model(Some(" gpt-5.4-mini"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini "), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt 5.4-mini"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini\u{7}"), "agent.model.primary").is_err());
    assert!(validate_optional_model(Some("gpt-5.4-mini\u{200b}"), "agent.model.primary").is_err());
}

#[test]
fn selected_expectation_selectors_are_validated() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let second_id = test_selector(&config, "2");
    let second_prefix = expectation_identities(&config).unwrap()[1]
        .display_id
        .clone();
    assert_eq!(select_expectations(&config, &[]).unwrap().len(), 2);
    assert_eq!(
        select_expectations(&config, &[second_id.into()]).unwrap()[0].number,
        2
    );
    assert_eq!(
        select_expectations(&config, &[second_prefix.into()]).unwrap()[0].number,
        2
    );
    assert!(select_expectations(&config, &["".into()]).is_err());
    assert!(select_expectations(&config, &["not-a-prefix".into()]).is_err());
    let duplicate = test_selector(&config, "1");
    assert!(select_expectations(&config, &[duplicate.clone().into(), duplicate.into()]).is_err());
}

#[test]
fn check_options_accept_all_with_selected_selectors() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let default_options =
        parse_check_options(&config, &[test_selector(&config, "2").into()]).unwrap();
    assert!(!default_options.check_all);
    assert_eq!(default_options.selected.len(), 1);
    assert_eq!(default_options.selected[0].number, 2);
    assert_eq!(default_options.skipped, 1);

    let all_options = parse_check_options(
        &config,
        &["--all".into(), test_selector(&config, "2").into()],
    )
    .unwrap();
    assert!(all_options.check_all);
    assert_eq!(all_options.selected.len(), 1);
    assert_eq!(all_options.selected[0].number, 2);
    assert_eq!(all_options.skipped, 1);
    assert!(parse_check_options(&config, &["--all".into(), "--all".into()]).is_err());
}
