use crate::*;

pub(crate) fn developer_instructions(agent: &AgentConfig, scope: &[String]) -> String {
    format!(
        "{}\n\nEnforced scope: {}\n\nAnswer-selection policy:\n{}\n\nWhen the answer-selection policy says to answer exactly `yes`, `no`, `idk`, `malformed`, or an option letter, put that exact string in the JSON `answer` field. Never output the raw answer as the whole response.",
        response_format_block(),
        compact_json_string_array(scope),
        agent.instructions.trim(),
    )
}

pub(crate) fn response_format_block() -> &'static str {
    concat!(
        "Response format:\n",
        "Return exactly one valid JSON object and no markdown, code fences, or surrounding prose.\n",
        "Schema: {\"answer\":\"<single-line answer>\",\"evidence\":\"<free-form evidence citing supporting files or code>\",\"scope\":[\"<normalized repository-relative path>\"]}\n",
        "The `answer` field is where the exact yes/no/idk/malformed/option-letter answer goes; do not write that answer outside the JSON object. ",
        "`scope` is the smallest allowed project context sufficient to determine the correct answer among all valid answers; it is not the list of evidence citations. ",
        "Use [\".\"] when the answer depends on project-wide absence, consistency, duplication, garbage, overall quality, or denied/inaccessible paths. ",
        "If the enforced task `scope` is narrower than [\".\"] and the question requires repository-wide or cross-module evidence, answer `idk` instead of drawing a positive or negative conclusion from incomplete context. ",
        "If visible code says the relevant behavior lives outside the enforced scope, answer `idk` rather than failing the project from that incomplete slice. ",
        "Never include denied or inaccessible paths in `scope`. Denied paths are intentionally outside the allowed evidence boundary; do not answer `idk` solely because a denied path is unreadable. ",
        "The current project state is the staged Git snapshot exposed at the working directory; do not treat files that exist only in `HEAD`, cache history, or diagnostic logs as current project files. ",
        "Evaluator agents are read-only; do not answer `idk` merely because a build or test command cannot run in the sandbox. Use visible staged source and test files as evidence unless the question specifically asks whether a command executed successfully. ",
        "The user-provided `agent.instructions` above are active project policy loaded from `.canon/check.yml`, not hardcoded implementation text and not necessarily the embedded default template shown in README; do not cite those instructions as `src/check.rs` contents. ",
        "A reusable cache hit is not an evaluator interrogation; questions about every evaluator interrogation concern only turns where `canon check` actually asks the evaluator model. ",
        "For `.canon/check.yml` schema/configuration questions, do not try to answer by opening `.canon/check.yml`; that path is denied by design. Use the fact that `canon check` has already loaded and validated the active config before starting the evaluator, plus the visible README, parser, validation, and template code; do not answer `idk` solely because `.canon/check.yml` itself is denied. This does not grant file access: if a question asks whether you can read or open `.canon/check.yml`, answer `no`. ",
        "`CANON_STATE_DIR` is the repository Git state directory at `git rev-parse --git-path canon`, normally `.git/canon`; evaluator permissions deny `.git/canon` and `.git/canon/**`, so if a question asks whether you can read files under `CANON_STATE_DIR`, answer `no`. ",
        "For absence and quality questions, answer `yes` only when there is a concrete removable file, code path, hack, or idiom violation with evidence; answer `no` when repository-wide inspection finds no concrete candidate, because absolute proof of absence is not required. ",
        "Treat behavior required by the active check contract, such as staged-snapshot isolation, process-tree cleanup, and configured log rolling, as not avoidable by itself.\n",
    )
}
