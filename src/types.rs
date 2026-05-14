use crate::evaluator_turn::EvaluatorFailureKind;
use crate::{
    EMPTY_EVIDENCE_OBSERVED, OBSERVED_IDK, OBSERVED_MALFORMED, RESULT_FAIL, RESULT_PASS,
    UNPARSEABLE_OBSERVED,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) root: PathBuf,
}

#[derive(Debug)]
pub(crate) struct Note {
    pub(crate) key: String,
    pub(crate) hash: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<RawExpectationItem>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentConfig {
    #[serde(default)]
    pub(crate) model: ModelConfig,
    #[serde(default = "default_thinking")]
    pub(crate) thinking: String,
    pub(crate) instructions: String,
    pub(crate) ignore: Vec<String>,
    pub(crate) plugins: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelConfig {
    #[serde(default)]
    pub(crate) primary: Option<String>,
    #[serde(default)]
    pub(crate) fallbacks: Vec<String>,
}

pub(crate) fn default_thinking() -> String {
    "low".to_string()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TokenUsage {
    pub(crate) total_tokens: u64,
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_output_tokens: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expectation {
    pub(crate) q: String,
    pub(crate) a: String,
    #[serde(default)]
    pub(crate) cooldown: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Explicit(RawExplicitExpectation),
    Generator(RawGeneratorExpectation),
    Include(RawIncludeExpectation),
}

#[derive(Debug, Clone)]
pub(crate) struct RawExplicitExpectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratorExpectation {
    pub(crate) q_template: String,
    pub(crate) path: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExpectationFields {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    q_template: Option<String>,
    #[serde(default)]
    a: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    cooldown: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

impl<'de> Deserialize<'de> for RawExpectationItem {
    fn deserialize<D>(deserializer: D) -> Result<RawExpectationItem, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = RawExpectationFields::deserialize(deserializer)?;
        RawExpectationItem::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

impl RawExpectationItem {
    fn from_fields(fields: RawExpectationFields) -> Result<RawExpectationItem, &'static str> {
        match (
            fields.q,
            fields.q_template,
            fields.path,
            fields.include,
            fields.a,
        ) {
            (Some(q), None, None, None, Some(a)) => {
                Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                    q,
                    a,
                    cooldown: fields.cooldown,
                    thinking: fields.thinking,
                }))
            }
            (None, Some(q_template), Some(path), None, Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q_template,
                    path,
                    a,
                    cooldown: fields.cooldown,
                    thinking: fields.thinking,
                }))
            }
            (None, None, None, Some(include), None) => {
                Ok(RawExpectationItem::Include(RawIncludeExpectation {
                    include,
                }))
            }
            (_, _, _, Some(_), Some(_)) => Err("include item must not contain a"),
            (Some(_), _, _, Some(_), _) => Err("include item must not contain q"),
            (_, Some(_), _, Some(_), _) => Err("include item must not contain q_template"),
            (_, _, Some(_), Some(_), _) => Err("include item must not contain path"),
            (Some(_), Some(_), _, _, _) => Err("must not contain both q and q_template"),
            (Some(_), None, Some(_), _, _) => {
                Err("must not contain path on an explicit expectation")
            }
            (Some(_), None, None, None, None) => Err("must contain a"),
            (None, Some(_), None, _, _) => Err("generator must contain path"),
            (None, Some(_), Some(_), None, None) => Err("must contain a"),
            (None, None, Some(_), _, _) => Err("generator must contain q_template"),
            (None, None, None, None, Some(_)) => Err("must contain q or q_template"),
            (None, None, None, None, None) => Err("must contain q, q_template, or include"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SelectedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
    pub(crate) display_id: String,
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<Cooldown>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) seconds: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedAnswer {
    pub(crate) answer: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedAnswerState {
    Answer,
    Idk,
    Malformed,
    Unparseable,
    EmptyEvidence,
    Unknown,
}

impl ObservedAnswerState {
    pub(crate) fn from_observed(observed: &str) -> ObservedAnswerState {
        match observed {
            OBSERVED_IDK => ObservedAnswerState::Idk,
            OBSERVED_MALFORMED => ObservedAnswerState::Malformed,
            UNPARSEABLE_OBSERVED => ObservedAnswerState::Unparseable,
            EMPTY_EVIDENCE_OBSERVED => ObservedAnswerState::EmptyEvidence,
            "yes" | "no" => ObservedAnswerState::Answer,
            _ if matches!(observed.as_bytes(), [letter] if letter.is_ascii_lowercase()) => {
                ObservedAnswerState::Answer
            }
            _ => ObservedAnswerState::Unknown,
        }
    }

    pub(crate) fn requires_human_review(self) -> bool {
        !matches!(self, ObservedAnswerState::Answer)
    }

    pub(crate) fn is_reusable_history(self) -> bool {
        matches!(self, ObservedAnswerState::Answer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluatorResponseJson {
    pub(crate) answer: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CheckResult {
    Pass,
    Fail,
}

impl CheckResult {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CheckResult::Pass => RESULT_PASS,
            CheckResult::Fail => RESULT_FAIL,
        }
    }
}

impl std::fmt::Display for CheckResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

// In-memory check record used by history, runtime logs, gate diagnostics, and
// check output. Do not infer persisted history field order from this struct:
// `render_check_log_record` is the authoritative history JSON writer and emits
// the cache-required fields first, followed by metadata such as the full ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    #[serde(default, skip_serializing)]
    pub(crate) number: usize,
    pub(crate) result: CheckResult,
    pub(crate) prompt: String,
    pub(crate) expected: String,
    pub(crate) observed: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    #[serde(rename = "scopeHash")]
    pub(crate) scope_hash: String,
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default, skip)]
    pub(crate) display_id: String,
    #[serde(default, rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    pub(crate) cache_key: Option<String>,
}

pub(crate) struct CheckOptions {
    // CLI-expanded expectation candidates. This is not the final selected set:
    // cooldown and silent exact-cache passes can remove candidates before the
    // check report records its selected/skipped counts. Exact cache lookup can
    // still reuse failed records; those stay selected and are reported.
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) skipped: usize,
    // `canon check` stops after the first final non-pass by default. `--all`
    // keeps running the full already-selected set.
    pub(crate) check_all: bool,
    pub(crate) ignore_cache: bool,
}

pub(crate) struct CheckCommandArgs {
    pub(crate) config_path: PathBuf,
    pub(crate) query: Option<String>,
    pub(crate) option_args: Vec<OsString>,
}

pub(crate) struct InterrogationResult {
    pub(crate) record: CheckRecord,
}

pub(crate) struct QueryInterrogationResult {
    pub(crate) answer: ParsedAnswer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NarrowingStats {
    pub(crate) attempted: usize,
    pub(crate) accepted: usize,
    pub(crate) rejected: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunReport {
    pub(crate) records: Vec<CheckRecord>,
    pub(crate) non_selected: Vec<SelectedExpectation>,
    // Final selected count after every selection rule has run. This excludes
    // command-selector misses, cooldown matches, and silent exact-cache passes.
    // Failed exact-cache hits stay selected because they produce FAILED output.
    pub(crate) selected: usize,
    // Final non-selected count. Silent exact-cache passes are non-selected for
    // reporting because they produce no per-expectation stdout.
    pub(crate) skipped: usize,
    // Non-selected expectations that intentionally produce no per-expectation
    // stdout, currently cooldown matches and silent exact-cache passes.
    pub(crate) silent: usize,
    pub(crate) narrowing: NarrowingStats,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckRunError {
    pub(crate) error: String,
    pub(crate) report: CheckRunReport,
}

pub(crate) fn check_run_error(
    records: &[CheckRecord],
    non_selected: &[SelectedExpectation],
    selected: usize,
    skipped: usize,
    silent: usize,
    narrowing: NarrowingStats,
    error: String,
) -> CheckRunError {
    CheckRunError {
        error,
        report: CheckRunReport {
            records: records.to_vec(),
            non_selected: non_selected.to_vec(),
            selected,
            skipped,
            silent,
            narrowing,
        },
    }
}

pub(crate) trait EvaluatorRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        thinking: &str,
        scope: &[String],
    ) -> Result<String, EvaluatorError>;
    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, EvaluatorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvaluatorError {
    kind: Option<EvaluatorFailureKind>,
    message: String,
}

impl EvaluatorError {
    pub(crate) fn message(message: impl Into<String>) -> EvaluatorError {
        EvaluatorError {
            kind: None,
            message: message.into(),
        }
    }

    pub(crate) fn failure(
        kind: EvaluatorFailureKind,
        message: impl Into<String>,
    ) -> EvaluatorError {
        EvaluatorError {
            kind: Some(kind),
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> Option<EvaluatorFailureKind> {
        self.kind
    }

    pub(crate) fn message_str(&self) -> &str {
        &self.message
    }
}

impl From<String> for EvaluatorError {
    fn from(message: String) -> EvaluatorError {
        EvaluatorError::message(message)
    }
}

impl From<&str> for EvaluatorError {
    fn from(message: &str) -> EvaluatorError {
        EvaluatorError::message(message)
    }
}

impl std::fmt::Display for EvaluatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
