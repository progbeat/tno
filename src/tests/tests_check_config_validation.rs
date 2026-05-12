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
    assert_eq!(options.skipped, 1);
    assert!(parse_check_options(&config, &["--fail-fast".into(), "--fail-fast".into()]).is_err());
}
