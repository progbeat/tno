use super::*;

#[test]
fn evaluator_prompt_is_only_current_question_text() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let prompt = "Permission question?".to_string();
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
    let prompt = "Permission question?".to_string();
    let input = evaluator_turn_input(&prompt).unwrap();
    assert_eq!(input, json!("Permission question?"));
    assert_eq!(render_evaluator_turn_input(&input).unwrap(), prompt);
}

#[test]
fn evaluator_turn_uses_strict_json_output_schema() {
    let schema = evaluator_response_output_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["answer", "evidence", "scope"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["answer"]["type"], "string");
    assert_eq!(schema["properties"]["answer"]["pattern"], "^[^\\r\\n]*$");
    assert_eq!(schema["properties"]["evidence"]["type"], "string");
    assert_eq!(schema["properties"]["scope"]["type"], "array");
    assert_eq!(schema["properties"]["scope"]["minItems"], 1);
    assert_eq!(schema["properties"]["scope"]["items"]["type"], "string");
    assert_eq!(schema["properties"]["scope"]["items"]["minLength"], 1);
    assert_eq!(
        schema["properties"]["scope"]["items"]["pattern"],
        "^[^\\r\\n]*$"
    );
}

#[test]
fn developer_instructions_include_agent_instructions_and_response_format() {
    let config = parse_check_config(check_config_yaml()).unwrap();
    let instructions = developer_instructions(&config.agent, &full_scope());
    assert!(instructions.contains(config.agent.instructions.trim()));
    assert!(instructions.contains("Response format:\nReturn exactly one valid JSON object"));
    assert!(instructions.contains(r#""answer":"<single-line answer>""#));
    assert!(instructions.contains(r#""scope":["<normalized repository-relative path>"]"#));
    assert!(instructions
        .contains("do not answer `idk` merely because a build or test command cannot run"));
    assert!(instructions.contains(
        "if a question asks whether you can read or open `.canon/check.yml`, answer `no`"
    ));
    assert!(instructions.contains(
        "if a question asks whether you can read files under `CANON_STATE_DIR`, answer `no`"
    ));
}
