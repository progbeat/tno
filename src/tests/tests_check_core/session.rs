use crate::check::run_check_with_runner;
use crate::check_types::CheckRecord;
use crate::evaluator_prompt::response_format_block;
use crate::logging::DiagnosticLogWriter;
use crate::tests::{
    answer, check_config_yaml, check_options, git_project, parse_check_config, FakeRunner,
};
use std::fs;

#[test]
fn check_runner_hides_expected_answers_and_reuses_session() {
    let root = git_project("check-runner");
    let config = parse_check_config(check_config_yaml()).unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("no", "README.md says enough", &["."]),
    ]);
    let mut diagnostic_log = DiagnosticLogWriter::create(&root).unwrap();
    let records = run_check_with_runner(
        &root,
        &root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        None,
    )
    .unwrap();
    assert!(records.records.iter().all(CheckRecord::passed));
    let log = fs::read_to_string(diagnostic_log.path()).unwrap();
    assert!(log
        .lines()
        .any(|line| line.contains(r#""event":"thread.reuse""#)
            && line.contains(r#""developerInstructions":"#)));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_roots, vec![root.clone()]);
    assert_eq!(
        runner.start_ignores,
        vec![vec![
            ".canon".to_string(),
            ".canon/**".to_string(),
            ".git/canon".to_string(),
            ".git/canon/**".to_string(),
            ".git/canon/logs".to_string(),
            ".git/canon/logs/**".to_string(),
            "target/**".to_string()
        ]]
    );
    assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
    assert_eq!(runner.start_models, vec![Some("gpt-5.4-mini".to_string())]);
    assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
    assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
    assert_eq!(
        runner.ask_models,
        vec![
            Some("gpt-5.4-mini".to_string()),
            Some("gpt-5.4-mini".to_string())
        ]
    );
    assert_eq!(
        runner.ask_thinking,
        vec!["medium".to_string(), "medium".to_string()]
    );
    assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
    let response_format_heading = response_format_block().lines().next().unwrap();
    assert!(runner
        .prompts
        .iter()
        .all(|prompt| !prompt.contains(response_format_heading)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn check_runner_applies_thinking_per_turn_when_reusing_scope_thread() {
    let root = git_project("check-thinking-turn");
    let config = parse_check_config(
        r#"
version: 1
agent:
  model:
    primary: gpt-5.4-mini
  thinking: low
  instructions: Answer from files only.
  ignore: []
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "yes"
    thinking: high
"#,
    )
    .unwrap();
    let options = check_options(&config, &["1", "2"], false, true);
    let mut runner = FakeRunner::new(&[
        &answer("yes", "README.md says enough", &["."]),
        &answer("yes", "README.md says enough", &["."]),
    ]);

    let records =
        run_check_with_runner(&root, &root, &config, &options, &mut runner, None, None).unwrap();

    assert!(records.records.iter().all(CheckRecord::passed));
    assert_eq!(runner.starts, 1);
    assert_eq!(runner.start_thinking, vec!["low".to_string()]);
    assert_eq!(
        runner.ask_thinking,
        vec!["low".to_string(), "high".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}
