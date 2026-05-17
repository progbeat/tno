use crate::check_output::compact_json_string_array;
use crate::config_types::AgentConfig;

const DEVELOPER_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("instructions/evaluator_developer_instructions.txt");
// Prompt fragments live as tracked text files so policy wording can be reviewed
// without editing Rust string literals; include_str! wires them into the binary.
const EVALUATOR_ANSWER_POLICY: &str = include_str!("instructions/evaluator_answer_policy.txt");
const EVALUATOR_RESPONSE_FORMAT: &str = include_str!("instructions/evaluator_response_format.txt");
pub(crate) const EVALUATOR_BASE_INSTRUCTIONS: &str =
    "You are a read-only canon evaluator. Answer the current turn using only this thread's developer instructions, current turn input, and permitted project files.";

pub(crate) fn developer_instructions(agent: &AgentConfig, scope: &[String]) -> String {
    let scope = compact_json_string_array(scope);
    render_instruction_template(
        DEVELOPER_INSTRUCTIONS_TEMPLATE.trim_end(),
        &[
            ("{{response_format}}", response_format_block()),
            ("{{scope}}", &scope),
            ("{{answer_policy}}", answer_policy()),
            ("{{agent_instructions}}", agent.instructions.trim()),
        ],
    )
}

pub(crate) fn response_format_block() -> &'static str {
    EVALUATOR_RESPONSE_FORMAT
}

fn answer_policy() -> &'static str {
    EVALUATOR_ANSWER_POLICY.trim_end()
}

fn render_instruction_template(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(template.to_owned(), |rendered, (placeholder, value)| {
            rendered.replace(placeholder, value)
        })
}
