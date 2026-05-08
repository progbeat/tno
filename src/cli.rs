#[derive(Debug)]
struct Config {
    root: PathBuf,
}

#[derive(Debug)]
struct Note {
    key: String,
    hash: String,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckConfig {
    version: u32,
    agent: AgentConfig,
    expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    #[serde(default)]
    model: ModelConfig,
    instructions: String,
    ignore: Vec<String>,
    plugins: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
struct ModelConfig {
    #[serde(default)]
    primary: Option<String>,
    #[serde(default)]
    fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TokenUsage {
    total_tokens: u64,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expectation {
    q: String,
    a: String,
}

#[derive(Debug, Clone)]
struct SelectedExpectation {
    number: usize,
    id: String,
    q: String,
    a: String,
}

#[derive(Debug)]
struct ParsedAnswer {
    answer: String,
    evidence: String,
    scope: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluatorResponseJson {
    answer: String,
    evidence: String,
    scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckRecord {
    timestamp: String,
    number: usize,
    result: String,
    prompt: String,
    expected: String,
    observed: String,
    evidence: String,
    scope: Vec<String>,
    #[serde(rename = "scopeHash")]
    scope_hash: String,
}

struct CheckOptions {
    selected: Vec<SelectedExpectation>,
    fail_fast: bool,
    ignore_cache: bool,
}

struct CheckCommandArgs {
    config_path: PathBuf,
    option_args: Vec<OsString>,
}

struct InterrogationResult {
    record: CheckRecord,
}

trait EvaluatorRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        scope: &[String],
    ) -> Result<String, String>;
    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String>;
}

fn main() {
    if let Err(err) = run(env::args_os().skip(1).collect()) {
        eprintln!("canon: {}", err);
        process::exit(1);
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
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

fn print_root(config: &Config) -> Result<(), String> {
    ensure_dir(&config.root)?;
    println!("{}", config.root.display());
    Ok(())
}

impl Config {
    fn from_env() -> Result<Config, String> {
        let thread_id = env::var("CODEX_THREAD_ID")
            .map_err(|_| "CODEX_THREAD_ID is required in v1".to_string())?;
        if thread_id.trim().is_empty() {
            return Err("CODEX_THREAD_ID is empty".to_string());
        }
        if thread_id.contains('/') || thread_id.contains('\\') {
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
            .unwrap_or_else(|| PathBuf::from("/tmp"));

        Ok(Config {
            root: temp_root.join("canon").join("codex").join(thread_id),
        })
    }
}

fn project_root_or_current(start: &Path) -> Result<PathBuf, String> {
    match git_project_root(start) {
        Ok(root) => Ok(root),
        Err(_) => env::current_dir().map_err(|err| format!("failed to read current dir: {}", err)),
    }
}

fn git_project_root(start: &Path) -> Result<PathBuf, String> {
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
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|_| "git project root must be valid UTF-8".to_string())?;
    Ok(PathBuf::from(root.trim()))
}
