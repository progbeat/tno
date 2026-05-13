use crate::*;

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

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct RawExpectationItem {
    #[serde(default)]
    pub(crate) q: Option<String>,
    #[serde(default)]
    pub(crate) q_template: Option<String>,
    #[serde(default)]
    pub(crate) a: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) include: Option<String>,
    #[serde(default)]
    pub(crate) cooldown: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SelectedExpectation {
    pub(crate) number: usize,
    pub(crate) id: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    pub(crate) number: usize,
    pub(crate) result: CheckResult,
    pub(crate) prompt: String,
    pub(crate) expected: String,
    pub(crate) observed: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    #[serde(rename = "scopeHash")]
    pub(crate) scope_hash: String,
    #[serde(default, rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    pub(crate) cache_key: Option<String>,
}

pub(crate) struct CheckOptions {
    // CLI-expanded expectation candidates. This is not the final selected set:
    // cooldown and reusable passing cache hits can remove candidates before the
    // check report records its selected/skipped counts.
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) skipped: usize,
    pub(crate) fail_fast: bool,
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
    // Final selected count after every deselection rule has run. This excludes
    // command-number misses, cooldown matches, and reusable passing cache hits.
    pub(crate) selected: usize,
    // Final non-selected count. It is the complement to `selected` across the
    // active check configuration, not merely the original CLI number filter.
    pub(crate) skipped: usize,
    // Non-selected expectations that intentionally produce no per-expectation
    // stdout, currently cooldown matches and reusable passing cache hits.
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
            selected,
            skipped,
            silent,
            narrowing,
        },
    }
}

impl std::ops::Deref for CheckRunReport {
    type Target = [CheckRecord];

    fn deref(&self) -> &Self::Target {
        &self.records
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
