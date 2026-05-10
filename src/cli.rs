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
#[serde(deny_unknown_fields)]
pub(crate) struct RawExpectationItem {
    #[serde(default)]
    pub(crate) q: Option<String>,
    #[serde(default)]
    pub(crate) q_template: Option<String>,
    pub(crate) a: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CheckRecord {
    pub(crate) timestamp: String,
    pub(crate) number: usize,
    pub(crate) result: String,
    pub(crate) prompt: String,
    pub(crate) expected: String,
    pub(crate) observed: String,
    pub(crate) evidence: String,
    pub(crate) scope: Vec<String>,
    #[serde(rename = "scopeHash")]
    pub(crate) scope_hash: String,
}

pub(crate) struct CheckOptions {
    pub(crate) selected: Vec<SelectedExpectation>,
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

pub(crate) struct CheckRunReport {
    pub(crate) records: Vec<CheckRecord>,
    pub(crate) skipped: usize,
    pub(crate) narrowing: NarrowingStats,
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
    ) -> Result<String, String>;
    fn ask(
        &mut self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
    ) -> Result<String, String>;
}

pub(crate) fn main() {
    if let Err(err) = run(env::args_os().skip(1).collect()) {
        if err == CHECK_FAILED_EXIT || err == GATE_FAILED_EXIT {
            process::exit(1);
        }
        eprintln!("canon: {}", err);
        process::exit(1);
    }
}

pub(crate) fn run(args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty() {
        let config = Config::from_env()?;
        print_root(&config)?;
        return Ok(());
    }

    let first = arg_to_string(&args[0])?;
    match first.as_str() {
        "init" => {
            if args.len() != 1 {
                return Err("init does not accept arguments".to_string());
            }
            let root = project_root_or_current(Path::new("."))?;
            return run_init(&root);
        }
        "hook" => {
            let root = git_project_root(Path::new("."))?;
            return run_hook_command(&root, &args[1..]);
        }
        "check" => {
            let root = git_project_root(Path::new("."))?;
            return run_check_command(&root, &args[1..]);
        }
        "gate" => {
            let root = git_project_root(Path::new("."))?;
            return run_gate_command(&root, &args[1..]);
        }
        "-h" | "--help" | "help" => {
            print_help();
            return Ok(());
        }
        _ => {}
    }

    let config = Config::from_env()?;
    match first.as_str() {
        "pwd" => {
            if args.len() != 1 {
                return Err("pwd does not accept arguments".to_string());
            }
            print_root(&config)?;
        }
        "p" | "path" => {
            let key = require_key(&args, 1)?;
            let note = ensure_note(&config, key)?;
            println!("{}", note.path.display());
        }
        "r" | "read" => {
            let key = require_key(&args, 1)?;
            read_note(&config, key)?;
        }
        "w" | "write" => {
            let key = require_key(&args, 1)?;
            let text = collect_text_or_stdin(&args, 2)?;
            write_note(&config, key, &text)?;
        }
        "a" | "append" => {
            let key = require_key(&args, 1)?;
            let text = collect_text_or_stdin(&args, 2)?;
            append_note(&config, key, &text)?;
        }
        "d" | "del" | "delete" | "rm" => {
            let key = require_key(&args, 1)?;
            delete_note(&config, key)?;
        }
        "rg" | "g" => {
            run_rg(&config, &args[1..])?;
        }
        _ => {
            if first.starts_with('-') {
                return Err(format!("unknown option: {}", first));
            }
            return Err(format!(
                "unknown command: {} (use `canon p <key>` to print a note path)",
                first
            ));
        }
    }

    Ok(())
}

pub(crate) fn print_root(config: &Config) -> Result<(), String> {
    ensure_dir(&config.root)?;
    println!("{}", config.root.display());
    Ok(())
}

impl Config {
    pub(crate) fn from_env() -> Result<Config, String> {
        let thread_id = env::var("CODEX_THREAD_ID")
            .map_err(|_| "CODEX_THREAD_ID is required in v1".to_string())?;
        if thread_id.trim().is_empty() {
            return Err("CODEX_THREAD_ID is empty".to_string());
        }
        if thread_id == "."
            || thread_id == ".."
            || thread_id.contains('/')
            || thread_id.contains('\\')
        {
            return Err("CODEX_THREAD_ID must be a single path segment".to_string());
        }

        if let Some(value) = env::var_os("CANON_HOME") {
            if !value.is_empty() {
                return Ok(Config {
                    root: PathBuf::from(value).join("codex").join(thread_id),
                });
            }
        }

        let temp_root = env::var_os("TMPDIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);

        Ok(Config {
            root: temp_root.join("canon").join("codex").join(thread_id),
        })
    }
}

pub(crate) fn project_root_or_current(start: &Path) -> Result<PathBuf, String> {
    match git_project_root(start) {
        Ok(root) => Ok(root),
        Err(_) => env::current_dir().map_err(|err| format!("failed to read current dir: {}", err)),
    }
}

pub(crate) fn command_output_utf8<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    std::str::from_utf8(bytes)
        .map_err(|err| format!("{} must be valid UTF-8: {}", description, err))
}

pub(crate) fn command_output_trimmed<'a>(
    bytes: &'a [u8],
    description: &str,
) -> Result<&'a str, String> {
    Ok(command_output_utf8(bytes, description)?.trim())
}

pub(crate) fn git_project_root(start: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to find git project root: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    Ok(path_from_git_stdout(output.stdout))
}

pub(crate) fn path_from_git_stdout(mut bytes: Vec<u8>) -> PathBuf {
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&bytes).to_string())
    }
}
