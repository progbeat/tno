use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
const GIT_CANON_CACHE_DIR: &str = "canon/cache";
const GIT_CANON_LOG_DIR: &str = "canon/logs";
const DIAGNOSTIC_LOG_MAX_BYTES: u64 = 128 * 1024;
const DIAGNOSTIC_LOG_FILES: [&str; 4] = ["0.jsonl", "1.jsonl", "2.jsonl", "3.jsonl"];
const DEFAULT_CHECK_TEMPLATE: &str = include_str!("../templates/check.yml");
const GIT_HOOKS_PATH: &str = ".githooks";
const PRE_COMMIT_HOOK_PATH: &str = ".githooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../templates/pre-commit");
const MALFORMED_REVIEW_WARNING: &str =
    "human review required: evaluator marked the expectation question as malformed";

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
    instructions: String,
    ignore: Vec<String>,
    plugins: Vec<String>,
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

struct InterrogationResult {
    record: CheckRecord,
    proposed_scope: Vec<String>,
}

trait EvaluatorRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
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
            return run_init(Path::new("."));
        }
        "hook" => {
            return run_hook_command(Path::new("."), &args[1..]);
        }
        "check" => {
            return run_check_command(Path::new("."), &args[1..]);
        }
        "gate" => {
            return run_gate_command(Path::new("."), &args[1..]);
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

fn require_key<'a>(args: &'a [OsString], index: usize) -> Result<&'a str, String> {
    args.get(index)
        .ok_or("missing key".to_string())
        .and_then(|arg| arg.to_str().ok_or("key must be valid UTF-8".to_string()))
}

fn arg_to_string(arg: &OsString) -> Result<String, String> {
    arg.to_str()
        .map(|value| value.to_string())
        .ok_or("argument must be valid UTF-8".to_string())
}

fn collect_text(args: &[OsString], start: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    for arg in &args[start..] {
        parts.push(arg.to_str().ok_or("text must be valid UTF-8".to_string())?);
    }
    Ok(parts.join(" "))
}

fn collect_text_or_stdin(args: &[OsString], start: usize) -> Result<String, String> {
    if args.len() > start {
        return collect_text(args, start);
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| format!("failed to read stdin: {}", err))?;
    Ok(text)
}

fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create {}: {}", path.display(), err))
}

fn ensure_note(config: &Config, key: &str) -> Result<Note, String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key);
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
    } else {
        let content = initial_content(key, &note.hash);
        fs::write(&note.path, content)
            .map_err(|err| format!("failed to write {}: {}", note.path.display(), err))?;
    }
    upsert_index(config, &note.hash, key)?;
    Ok(note)
}

fn note_for_key(config: &Config, key: &str) -> Note {
    let hash = hash_key(key);
    let path = config.root.join(format!("{}.md", hash));
    Note {
        key: key.to_string(),
        hash,
        path,
    }
}

fn read_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key);
    if !note.path.exists() {
        return Err(format!("canon not found for key: {}", key));
    }
    verify_note_key(&note.path, key)?;
    let mut file = fs::File::open(&note.path)
        .map_err(|err| format!("failed to open {}: {}", note.path.display(), err))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
    print!("{}", content);
    Ok(())
}

fn write_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    let note = ensure_note(config, key)?;
    let content = format!(
        "{}{}\n",
        header(&note.key, &note.hash),
        normalize_body(text)
    );
    fs::write(&note.path, content)
        .map_err(|err| format!("failed to write {}: {}", note.path.display(), err))
}

fn append_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    let note = ensure_note(config, key)?;
    let timestamp = unix_timestamp()?;
    let section = format!("\n## {}\n\n{}\n", timestamp, normalize_body(text));
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&note.path)
        .map_err(|err| format!("failed to open {}: {}", note.path.display(), err))?;
    file.write_all(section.as_bytes())
        .map_err(|err| format!("failed to append {}: {}", note.path.display(), err))
}

fn delete_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key);
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
        fs::remove_file(&note.path)
            .map_err(|err| format!("failed to delete {}: {}", note.path.display(), err))?;
    }
    remove_index(config, &note.hash, key)
}

fn run_rg(config: &Config, rg_args: &[OsString]) -> Result<(), String> {
    if rg_args.is_empty() {
        return Err("missing rg pattern".to_string());
    }
    ensure_dir(&config.root)?;
    let mut command = Command::new("rg");
    command.args(rg_args);
    command.arg(&config.root);
    let status = command
        .status()
        .map_err(|err| format!("failed to run rg: {}", err))?;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(format!("rg exited with status {}", code)),
        None => Err("rg terminated by signal".to_string()),
    }
}

fn initial_content(key: &str, hash: &str) -> String {
    header(key, hash)
}

fn header(key: &str, hash: &str) -> String {
    format!(
        "<!-- canon key=\"{}\" hash=\"{}\" -->\n# {}\n",
        escape_attr(key),
        hash,
        key
    )
}

fn normalize_body(text: &str) -> String {
    let mut value = text.to_string();
    while value.ends_with('\n') {
        value.pop();
    }
    value
}

fn verify_note_key(path: &Path, expected_key: &str) -> Result<(), String> {
    let first = first_line(path)?;
    let actual_key = parse_key_from_header(&first)
        .ok_or_else(|| format!("missing canon metadata in {}", path.display()))?;
    if actual_key != expected_key {
        return Err(format!(
            "hash collision or stale file: {} belongs to key {:?}, not {:?}",
            path.display(),
            actual_key,
            expected_key
        ));
    }
    Ok(())
}

fn first_line(path: &Path) -> Result<String, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    Ok(content.lines().next().unwrap_or("").to_string())
}

fn parse_key_from_header(line: &str) -> Option<String> {
    let prefix = "<!-- canon key=\"";
    let rest = line.strip_prefix(prefix)?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => {
                let escaped = chars.next()?;
                match escaped {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

fn escape_attr(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn upsert_index(config: &Config, hash: &str, key: &str) -> Result<(), String> {
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(existing_hash, existing_key)| existing_hash != hash && existing_key != key);
    entries.push((hash.to_string(), key.to_string()));
    write_index(&path, &entries)
}

fn remove_index(config: &Config, hash: &str, key: &str) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(existing_hash, existing_key)| existing_hash != hash && existing_key != key);
    write_index(&path, &entries)
}

fn read_index(path: &Path) -> Result<Vec<(String, String)>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let mut entries = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let hash = parts.next().unwrap_or("").to_string();
        let key = parts.next().unwrap_or("").to_string();
        if !hash.is_empty() && !key.is_empty() {
            entries.push((hash, key));
        }
    }
    Ok(entries)
}

fn write_index(path: &Path, entries: &[(String, String)]) -> Result<(), String> {
    let mut content = String::new();
    for (hash, key) in entries {
        content.push_str(hash);
        content.push('\t');
        content.push_str(key);
        content.push('\n');
    }
    fs::write(path, content).map_err(|err| format!("failed to write {}: {}", path.display(), err))
}

fn unix_timestamp() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))
}

fn hash_key(key: &str) -> String {
    let mut hash = FNV_OFFSET;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    encode_60_bits(hash & ((1u64 << 60) - 1))
}

fn encode_60_bits(value: u64) -> String {
    let mut out = String::with_capacity(10);
    for shift in (0..60).step_by(6).rev() {
        let index = ((value >> shift) & 0x3f) as usize;
        out.push(B64_URL[index] as char);
    }
    out
}

fn run_init(root: &Path) -> Result<(), String> {
    let check_path = root.join(CHECK_PATH);
    if check_path.exists() {
        return Err(format!("{} already exists", CHECK_PATH));
    }

    if let Some(parent) = check_path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(&check_path, DEFAULT_CHECK_TEMPLATE)
        .map_err(|err| format!("failed to write {}: {}", check_path.display(), err))?;
    println!("Created {}", CHECK_PATH);
    Ok(())
}

fn run_hook_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: canon hook install".to_string());
    }
    let action = arg_to_string(&args[0])?;
    match action.as_str() {
        "install" => run_hook_install(root),
        _ => Err(format!("unknown hook command: {}", action)),
    }
}

fn run_hook_install(root: &Path) -> Result<(), String> {
    preflight_pre_commit_hook(root)?;
    preflight_git_hooks_path(root)?;
    install_pre_commit_hook(root)
}

fn preflight_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if !hook_path.exists() {
        return Ok(());
    }

    let existing = fs::read_to_string(&hook_path)
        .map_err(|err| format!("failed to read {}: {}", hook_path.display(), err))?;
    if existing != DEFAULT_PRE_COMMIT_HOOK {
        return Err(format!(
            "{} already exists with different content",
            PRE_COMMIT_HOOK_PATH
        ));
    }
    Ok(())
}

fn preflight_git_hooks_path(root: &Path) -> Result<(), String> {
    if let Some(existing) = current_git_hooks_path(root)? {
        if existing != GIT_HOOKS_PATH {
            return Err(format!(
                "git core.hooksPath is already set to {}; set it to {} manually if desired",
                existing, GIT_HOOKS_PATH
            ));
        }
    }
    Ok(())
}

fn install_pre_commit_hook(root: &Path) -> Result<(), String> {
    let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
    if let Some(parent) = hook_path.parent() {
        ensure_dir(parent)?;
    }
    if !hook_path.exists() {
        fs::write(&hook_path, DEFAULT_PRE_COMMIT_HOOK)
            .map_err(|err| format!("failed to write {}: {}", hook_path.display(), err))?;
        println!("Created {}", PRE_COMMIT_HOOK_PATH);
    }
    make_executable(&hook_path)?;
    configure_git_hooks_path(root)?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to chmod {}: {}", path.display(), err))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn configure_git_hooks_path(root: &Path) -> Result<(), String> {
    if !is_git_worktree(root)? {
        println!(
            "Git worktree not detected; {} was created but core.hooksPath was not set.",
            PRE_COMMIT_HOOK_PATH
        );
        return Ok(());
    }

    if current_git_hooks_path(root)?.as_deref() == Some(GIT_HOOKS_PATH) {
        println!("Git core.hooksPath already = {}", GIT_HOOKS_PATH);
        return Ok(());
    }

    set_git_hooks_path(root)?;
    println!("Configured git core.hooksPath = {}", GIT_HOOKS_PATH);
    Ok(())
}

fn current_git_hooks_path(root: &Path) -> Result<Option<String>, String> {
    if !is_git_worktree(root)? {
        return Ok(None);
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("--get")
        .arg("core.hooksPath")
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "failed to read git core.hooksPath: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn set_git_hooks_path(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--local")
        .arg("core.hooksPath")
        .arg(GIT_HOOKS_PATH)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "failed to set git core.hooksPath: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn is_git_worktree(root: &Path) -> Result<bool, String> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("failed to run git rev-parse: {}", err)),
    };
    Ok(output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true")
}

fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    let config = load_check_config(root)?;
    let options = parse_check_options(&config, args)?;
    fail_on_mixed_canon_changes(root)?;
    let tree = git_write_tree(root)?;
    let snapshot = StagedSnapshot::create(root, &tree)?;
    let mut runner = AppServerRunner::new(check_config_loads_plugins(&config))?;
    let mut diagnostic_log = DiagnosticLogWriter::create(root)?;
    let records = run_check_with_runner(
        root,
        snapshot.path(),
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
    )?;
    diagnostic_log.finish()?;
    print_check_report(&records);
    if records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err("canon check failed".to_string())
    }
}

fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.iter().any(|arg| arg.to_str() == Some("--fail-fast")) {
        return Err("canon gate does not accept --fail-fast".to_string());
    }
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".to_string());
    }
    let config = load_check_config(root)?;
    let selected = select_expectations(&config, args)?;
    fail_on_mixed_canon_changes(root)?;

    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected {
        match reusable_history_record(root, &config.agent, expectation)? {
            Some(record) if record.passed() => {}
            Some(record) => failing.push(record),
            None => missing.push(expectation.number),
        }
    }

    if missing.is_empty() && failing.is_empty() {
        return Ok(());
    }
    if !missing.is_empty() {
        eprintln!(
            "canon gate: missing cached answers for expectations: {}",
            join_numbers(&missing)
        );
        eprintln!("canon gate: run `canon check` before committing");
    }
    if !failing.is_empty() {
        eprintln!("canon gate: cached failing expectation results:");
        for record in &failing {
            eprint!("{}", render_check_log_record(record));
        }
    }
    Err("canon gate failed".to_string())
}

fn load_check_config(root: &Path) -> Result<CheckConfig, String> {
    let content = staged_file_content(root, CHECK_PATH).or_else(|_| {
        let path = root.join(CHECK_PATH);
        fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))
    })?;
    let config: CheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", CHECK_PATH, err))?;
    validate_check_config(&config)?;
    Ok(config)
}

fn parse_check_options(config: &CheckConfig, args: &[OsString]) -> Result<CheckOptions, String> {
    let mut fail_fast = false;
    let mut ignore_cache = false;
    let mut numbers = Vec::new();
    for arg in args {
        if arg.to_str() == Some("--fail-fast") {
            if fail_fast {
                return Err("duplicate --fail-fast".to_string());
            }
            fail_fast = true;
        } else if arg.to_str() == Some("--ignore-cache") {
            if ignore_cache {
                return Err("duplicate --ignore-cache".to_string());
            }
            ignore_cache = true;
        } else {
            numbers.push(arg.clone());
        }
    }
    Ok(CheckOptions {
        selected: select_expectations(config, &numbers)?,
        fail_fast,
        ignore_cache,
    })
}

fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if config.agent.instructions.trim().is_empty() {
        return Err("check.yml agent.instructions must not be empty".to_string());
    }
    for path in &config.agent.ignore {
        validate_relative_config_path(path, "agent ignore pattern")?;
    }
    for plugin in &config.agent.plugins {
        validate_plugin_config_key(plugin)?;
    }
    if config.expectations.is_empty() {
        return Err("check.yml expectations must not be empty".to_string());
    }
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        if expectation.q.trim().is_empty() {
            return Err(format!("expectation {} has an empty q", number));
        }
        if expectation.a.contains('\n') || expectation.a.contains('\r') {
            return Err(format!(
                "expectation {} expected answer must be single-line",
                number
            ));
        }
    }
    Ok(())
}

fn validate_plugin_config_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("agent has an empty plugin entry".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("agent plugin entries must be single-line strings".to_string());
    }
    if !value.contains('@') {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    }
    Ok(())
}

fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
}

fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    normalize_repo_path(value)
        .map(|_| ())
        .map_err(|err| format!("{}: {}", label, err))
}

fn select_expectations(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    let mut selected_numbers = Vec::new();
    if args.is_empty() {
        selected_numbers.extend(1..=config.expectations.len());
    } else {
        let mut seen = BTreeSet::new();
        for arg in args {
            let text = arg
                .to_str()
                .ok_or("expectation number must be valid UTF-8".to_string())?;
            let number = text
                .parse::<usize>()
                .map_err(|_| format!("invalid expectation number: {}", text))?;
            if number == 0 {
                return Err("expectation numbers are 1-based".to_string());
            }
            if number > config.expectations.len() {
                return Err(format!("expectation number out of range: {}", number));
            }
            if !seen.insert(number) {
                return Err(format!("duplicate expectation number: {}", number));
            }
            selected_numbers.push(number);
        }
    }

    Ok(selected_numbers
        .into_iter()
        .map(|number| {
            let expectation = &config.expectations[number - 1];
            SelectedExpectation {
                number,
                id: expectation_id(&expectation.q, &expectation.a),
                q: expectation.q.clone(),
                a: expectation.a.clone(),
            }
        })
        .collect())
}

fn fail_on_mixed_canon_changes(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .arg("--diff-filter=ACDMRTUXB")
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git diff output must be valid UTF-8".to_string())?;
    let paths = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    fail_on_mixed_canon_paths(&paths)
}

fn fail_on_mixed_canon_paths(paths: &[String]) -> Result<(), String> {
    let has_canon = paths.iter().any(|path| is_canon_project_path(path));
    let has_other = paths.iter().any(|path| !is_canon_project_path(path));
    if has_canon && has_other {
        return Err(
            "canon check failed: .canon/** changes must not be mixed with non-.canon changes"
                .to_string(),
        );
    }
    Ok(())
}

fn is_canon_project_path(path: &str) -> bool {
    path == ".canon" || path.starts_with(".canon/")
}

fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    let mut sessions = BTreeMap::new();
    for expectation in &options.selected {
        if !options.ignore_cache {
            if let Some(record) = reusable_history_record(root, &config.agent, expectation)? {
                let should_stop = options.fail_fast && !record.passed();
                records.push(record);
                if should_stop {
                    return Ok(records);
                }
                continue;
            }
        }

        let mut scope = latest_history_scope(root, expectation)?.unwrap_or_else(full_scope);
        let mut interrogation = interrogate_expectation(
            root,
            snapshot_root,
            config,
            expectation,
            runner,
            &mut sessions,
            &mut diagnostic_log,
            &scope,
        )?;
        if interrogation.record.observed == "idk" && scope != full_scope() {
            scope = full_scope();
            interrogation = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &scope,
            )?;
        }

        let proposed_scope = sanitize_scope(&interrogation.proposed_scope, &config.agent)
            .unwrap_or_else(|_| interrogation.record.scope.clone());
        if is_strict_scope_subset(&proposed_scope, &scope) {
            let narrowed = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &proposed_scope,
            )?;
            if narrowed.record.observed == interrogation.record.observed {
                interrogation = narrowed;
            }
        }

        append_history_record(root, expectation, &interrogation.record)?;
        let should_stop = options.fail_fast && !interrogation.record.passed();
        records.push(interrogation.record);
        if should_stop {
            return Ok(records);
        }
    }
    Ok(records)
}

fn interrogate_expectation<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    expectation: &SelectedExpectation,
    runner: &mut R,
    sessions: &mut BTreeMap<String, String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    let scope = sanitize_scope(enforced_scope, &config.agent)?;
    let scope_hash = staged_scope_hash(root, &config.agent, &scope)?;
    let session_key = scope.join("\n");
    let session_id = if let Some(existing) = sessions.get(&session_key) {
        existing.clone()
    } else {
        let session_id = runner.start_session(
            snapshot_root,
            &config.agent.instructions,
            &config.agent,
            &scope,
        )?;
        sessions.insert(session_key, session_id.clone());
        session_id
    };
    let prompt = question_prompt(config, &expectation.q, &scope);
    let response = ask_with_repairs(runner, &session_id, &prompt)?;
    let proposed_scope = response.scope.clone();
    let record = record_from_response(expectation, response, scope, scope_hash)?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_record(&record)?;
    }
    Ok(InterrogationResult {
        record,
        proposed_scope,
    })
}

fn record_from_response(
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    if response.answer == "malformed" {
        eprintln!(
            "canon check: expectation {}: {}",
            expectation.number, MALFORMED_REVIEW_WARNING
        );
    }
    if response.evidence.trim().is_empty() {
        eprintln!(
            "canon check: expectation {}: evidence is empty after retry",
            expectation.number
        );
    }
    let result = if response.answer == expectation.a && response.answer != "malformed" {
        "pass"
    } else {
        "fail"
    };
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
        number: expectation.number,
        result: result.to_string(),
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: response.answer,
        evidence: response.evidence,
        scope: enforced_scope,
        scope_hash,
    })
}

fn ask_with_repairs<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    prompt: &str,
) -> Result<ParsedAnswer, String> {
    let first = runner.ask(session_id, prompt)?;
    let mut parsed = match parse_evaluator_response(&first) {
        Ok(answer) => answer,
        Err(_) => {
            let repaired = runner.ask(session_id, malformed_repair_prompt())?;
            parse_evaluator_response(&repaired)?
        }
    };

    if parsed.answer == "malformed" {
        let repaired = runner.ask(session_id, malformed_answer_repair_prompt())?;
        if let Ok(answer) = parse_evaluator_response(&repaired) {
            parsed = answer;
        }
    }

    if parsed.evidence.trim().is_empty() {
        let repaired = runner.ask(session_id, evidence_repair_prompt())?;
        if let Ok(answer) = parse_evaluator_response(&repaired) {
            parsed = answer;
        }
    }

    Ok(parsed)
}

fn question_prompt(config: &CheckConfig, question: &str, scope: &[String]) -> String {
    format!(
        "{}\n\nRuntime check.yml metadata summary:\n{}\nThis summary is provided by canon itself, is not file-read access to `.canon/check.yml`, and does not include expected answers.\n\nAllowed scope:\n{}\n\nExpectation:\n{}\n\nReply using this exact format:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths containing the evidence>\n",
        config.agent.instructions.trim(),
        check_config_summary(config),
        serde_json::to_string(scope).expect("scope is serializable"),
        question
    )
}

fn check_config_summary(config: &CheckConfig) -> String {
    format!(
        "- version: {}\n- top-level `agent` section: present\n- top-level `agents` section: absent\n- single evaluator agent fields: instructions, ignore, plugins\n- configured ignore patterns: {}\n- configured plugins: {}\n- expectation count: {}",
        config.version,
        serde_json::to_string(&config.agent.ignore).expect("ignore patterns are serializable"),
        serde_json::to_string(&config.agent.plugins).expect("plugins are serializable"),
        config.expectations.len()
    )
}

fn malformed_repair_prompt() -> &'static str {
    "Your previous response could not be parsed. Reply again using exactly:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths containing the evidence>\n"
}

fn malformed_answer_repair_prompt() -> &'static str {
    "Your previous answer was `malformed`. Retry once. If the question is truly malformed, answer `malformed` again. Reply using exactly:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths containing the evidence>\n"
}

fn evidence_repair_prompt() -> &'static str {
    "Your previous response had an answer but no evidence. Reply again with the same format and include evidence if the available files support it:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths containing the evidence>\n"
}

fn parse_evaluator_response(text: &str) -> Result<ParsedAnswer, String> {
    let lines = text.lines().collect::<Vec<_>>();
    let first = lines
        .first()
        .ok_or("missing ANSWER line".to_string())?
        .trim_end();
    let answer = first
        .strip_prefix("ANSWER: ")
        .ok_or("missing ANSWER line".to_string())?
        .to_string();
    let second = lines
        .get(1)
        .ok_or("missing EVIDENCE block".to_string())?
        .trim_end();
    if second != "EVIDENCE:" {
        return Err("missing EVIDENCE block".to_string());
    }
    let scope_line_index = lines
        .iter()
        .enumerate()
        .skip(2)
        .rev()
        .find_map(|(index, line)| line.trim_end().strip_prefix("SCOPE: ").map(|_| index))
        .ok_or("missing SCOPE line".to_string())?;
    let scope_text = lines[scope_line_index]
        .trim_end()
        .strip_prefix("SCOPE: ")
        .ok_or("missing SCOPE line".to_string())?;
    Ok(ParsedAnswer {
        answer,
        evidence: lines[2..scope_line_index].join("\n"),
        scope: parse_scope_json(scope_text)?,
    })
}

fn parse_scope_json(text: &str) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("failed to parse SCOPE JSON: {}", err))?;
    let array = value
        .as_array()
        .ok_or("SCOPE must be a JSON array".to_string())?;
    if array.len() > 4 {
        return Err("SCOPE must contain at most 4 paths".to_string());
    }
    let mut scope = Vec::new();
    for item in array {
        let raw = item
            .as_str()
            .ok_or("SCOPE entries must be strings".to_string())?;
        let normalized = normalize_repo_path(raw)?;
        if normalized != raw.trim() {
            return Err(format!("SCOPE entry must be normalized: {}", raw));
        }
        if normalized != "." && (normalized.contains('*') || normalized.contains('?')) {
            return Err(format!("SCOPE entry must not be a glob: {}", raw));
        }
        scope.push(normalized);
    }
    Ok(scope)
}

fn print_check_report(records: &[CheckRecord]) {
    for record in records {
        print!("{}", render_check_log_record(record));
    }
}

impl CheckRecord {
    fn passed(&self) -> bool {
        self.result == "pass"
    }
}

#[cfg(test)]
fn write_diagnostic_log(root: &Path, records: &[CheckRecord]) -> Result<PathBuf, String> {
    let mut writer = DiagnosticLogWriter::create(root)?;
    for record in records {
        writer.write_record(record)?;
    }
    let path = writer.path.clone();
    writer.finish()?;
    Ok(path)
}

struct DiagnosticLogWriter {
    path: PathBuf,
    file: Option<fs::File>,
}

impl DiagnosticLogWriter {
    fn create(root: &Path) -> Result<DiagnosticLogWriter, String> {
        let log_dir = git_path(root, GIT_CANON_LOG_DIR)?;
        ensure_dir(&log_dir)?;
        rotate_diagnostic_logs_if_needed(&log_dir)?;
        let path = log_dir.join("0.jsonl");
        Ok(DiagnosticLogWriter { path, file: None })
    }

    fn write_record(&mut self, record: &CheckRecord) -> Result<(), String> {
        if self.file.is_none() {
            self.file = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .map_err(|err| format!("failed to open {}: {}", self.path.display(), err))?,
            );
        }
        let line = render_check_log_record(record);
        let file = self.file.as_mut().expect("diagnostic log file is open");
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", self.path.display(), err))?;
        file.flush()
            .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))
    }

    fn finish(mut self) -> Result<(), String> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
                .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))?;
        }
        Ok(())
    }
}

fn render_check_log_record(record: &CheckRecord) -> String {
    let mut output = String::new();
    output.push('{');
    let mut first = true;
    append_json_field(
        &mut output,
        &mut first,
        "timestamp",
        json!(record.timestamp),
    );
    append_json_field(&mut output, &mut first, "number", json!(record.number));
    append_json_field(&mut output, &mut first, "result", json!(record.result));
    append_json_field(&mut output, &mut first, "prompt", json!(record.prompt));
    append_json_field(&mut output, &mut first, "expected", json!(record.expected));
    append_json_field(&mut output, &mut first, "observed", json!(record.observed));
    append_json_field(&mut output, &mut first, "evidence", json!(record.evidence));
    append_json_field(&mut output, &mut first, "scope", json!(record.scope));
    append_json_field(
        &mut output,
        &mut first,
        "scopeHash",
        json!(record.scope_hash),
    );
    output.push_str("}\n");
    output
}

fn append_json_field(output: &mut String, first: &mut bool, key: &str, value: Value) {
    if *first {
        *first = false;
    } else {
        output.push(',');
    }
    output.push_str(&serde_json::to_string(key).expect("check log key is serializable"));
    output.push(':');
    output.push_str(&serde_json::to_string(&value).expect("check log value is serializable"));
}

fn join_numbers(numbers: &[usize]) -> String {
    numbers
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn full_scope() -> Vec<String> {
    vec![".".to_string()]
}

fn expectation_id(prompt: &str, expected: &str) -> String {
    hash_120(format!("q\0{}\0a\0{}", prompt, expected).as_bytes())
}

fn hash_120(input: &[u8]) -> String {
    let first = fnv64_with_seed(FNV_OFFSET, input);
    let second = fnv64_with_seed(FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15, input);
    let mut bytes = [0u8; 15];
    bytes[..8].copy_from_slice(&first.to_be_bytes());
    bytes[8..].copy_from_slice(&second.to_be_bytes()[..7]);
    encode_base64url_no_pad(&bytes)
}

fn fnv64_with_seed(seed: u64, input: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in input {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn encode_base64url_no_pad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 4 + 2) / 3);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        let value = ((a as u32) << 16) | ((b as u32) << 8) | c as u32;
        out.push(B64_URL[((value >> 18) & 0x3f) as usize] as char);
        out.push(B64_URL[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_URL[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_URL[(value & 0x3f) as usize] as char);
        }
    }
    out
}

fn git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--git-path")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to run git rev-parse: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to resolve git path {}: {}",
            path,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let resolved = String::from_utf8(output.stdout)
        .map_err(|_| "git rev-parse output must be valid UTF-8".to_string())?;
    Ok(root.join(resolved.trim()))
}

fn staged_file_content(root: &Path, path: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("show")
        .arg(format!(":{}", path))
        .output()
        .map_err(|err| format!("failed to run git show: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to read staged {}: {}",
            path,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("staged {} must be valid UTF-8", path))
}

fn git_write_tree(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("write-tree")
        .output()
        .map_err(|err| format!("failed to run git write-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to write staged git tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

struct StagedSnapshot {
    path: PathBuf,
}

impl StagedSnapshot {
    fn create(root: &Path, tree: &str) -> Result<StagedSnapshot, String> {
        let path = unique_temp_dir("canon-staged-snapshot")?;
        ensure_dir(&path)?;
        let mut archive = Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("archive")
            .arg("--format=tar")
            .arg(tree)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start git archive: {}", err))?;
        let archive_stdout = archive
            .stdout
            .take()
            .ok_or("failed to capture git archive stdout".to_string())?;
        let tar_status = Command::new("tar")
            .arg("-x")
            .arg("-C")
            .arg(&path)
            .stdin(Stdio::from(archive_stdout))
            .status()
            .map_err(|err| format!("failed to run tar: {}", err))?;
        let archive_status = archive
            .wait()
            .map_err(|err| format!("failed to wait for git archive: {}", err))?;
        if !archive_status.success() {
            return Err("git archive failed".to_string());
        }
        if !tar_status.success() {
            return Err("failed to extract staged git snapshot".to_string());
        }
        Ok(StagedSnapshot { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagedSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))?
        .as_nanos();
    Ok(env::temp_dir().join(format!("{}-{}-{}", prefix, process::id(), nanos)))
}

fn history_path(root: &Path, expectation: &SelectedExpectation) -> Result<PathBuf, String> {
    git_path(
        root,
        &format!("{}/{}/history.jsonl", GIT_CANON_CACHE_DIR, expectation.id),
    )
}

fn read_history_records(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Vec<CheckRecord>, String> {
    let path = history_path(root, expectation)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let mut records = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(record) = serde_json::from_str::<CheckRecord>(line) {
            records.push(record);
        }
    }
    Ok(records)
}

fn reusable_history_record(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<CheckRecord>, String> {
    let records = read_history_records(root, expectation)?;
    for record in records.into_iter().rev() {
        let scope = match sanitize_scope(&record.scope, agent) {
            Ok(scope) => scope,
            Err(_) => continue,
        };
        let current_hash = staged_scope_hash(root, agent, &scope)?;
        if current_hash == record.scope_hash {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

fn latest_history_scope(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Option<Vec<String>>, String> {
    let records = read_history_records(root, expectation)?;
    Ok(records.into_iter().rev().next().map(|record| record.scope))
}

fn append_history_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    let path = history_path(root, expectation)?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let line = render_check_log_record(record);
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))?;
    if should_compact_history()? {
        compact_history(&path)?;
    }
    Ok(())
}

fn should_compact_history() -> Result<bool, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))?
        .subsec_nanos();
    Ok(nanos % 15 == 0)
}

fn compact_history(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let mut lines = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.len() <= 5 {
        return Ok(());
    }
    lines = lines.split_off(lines.len() - 5);
    let mut file = fs::File::create(path)
        .map_err(|err| format!("failed to rewrite {}: {}", path.display(), err))?;
    for line in lines {
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
        file.write_all(b"\n")
            .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    }
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

fn rotate_diagnostic_logs_if_needed(log_dir: &Path) -> Result<(), String> {
    let active = log_dir.join(DIAGNOSTIC_LOG_FILES[0]);
    let should_rotate = active
        .metadata()
        .map(|metadata| metadata.len() > DIAGNOSTIC_LOG_MAX_BYTES)
        .unwrap_or(false);
    if !should_rotate {
        return Ok(());
    }
    let oldest = log_dir.join(DIAGNOSTIC_LOG_FILES[3]);
    if oldest.exists() {
        fs::remove_file(&oldest)
            .map_err(|err| format!("failed to remove {}: {}", oldest.display(), err))?;
    }
    for index in (0..3).rev() {
        let from = log_dir.join(DIAGNOSTIC_LOG_FILES[index]);
        if from.exists() {
            let to = log_dir.join(DIAGNOSTIC_LOG_FILES[index + 1]);
            fs::rename(&from, &to).map_err(|err| {
                format!(
                    "failed to rename {} to {}: {}",
                    from.display(),
                    to.display(),
                    err
                )
            })?;
        }
    }
    Ok(())
}

fn staged_scope_hash(root: &Path, agent: &AgentConfig, scope: &[String]) -> Result<String, String> {
    let scope = sanitize_scope(scope, agent)?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("-s")
        .arg("--");
    if scope != full_scope() {
        for path in &scope {
            command.arg(path);
        }
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run git ls-files: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to inspect staged scope: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git ls-files output must be valid UTF-8".to_string())?;
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if let Some((metadata, path)) = line.split_once('\t') {
            if !is_denied_path(agent, path) {
                entries.push(format!("{}\t{}", metadata, path));
            }
        }
    }
    entries.sort();
    entries.dedup();
    Ok(hash_120(entries.join("\n").as_bytes()))
}

fn sanitize_scope(scope: &[String], agent: &AgentConfig) -> Result<Vec<String>, String> {
    if scope.is_empty() {
        return Ok(full_scope());
    }
    if scope.len() > 4 {
        return Err("scope must contain at most 4 paths".to_string());
    }
    let mut normalized = Vec::new();
    for path in scope {
        let path = normalize_repo_path(path)?;
        if path != "." && (path.contains('*') || path.contains('?')) {
            return Err(format!("scope paths must not be globs: {}", path));
        }
        if path != "." && is_denied_path(agent, &path) {
            return Err(format!("scope path is denied: {}", path));
        }
        if path == "." {
            return Ok(full_scope());
        }
        if !normalized.iter().any(|existing| existing == &path) {
            normalized.push(path);
        }
    }
    if normalized.is_empty() {
        Ok(full_scope())
    } else {
        Ok(normalized)
    }
}

fn normalize_repo_path(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("path must not be empty".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!("path must be relative: {}", value));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("path must be valid UTF-8: {}", value))?;
                parts.push(part.to_string());
            }
            std::path::Component::ParentDir => {
                return Err(format!("path must not contain '..': {}", value));
            }
            _ => return Err(format!("unsupported path component in {}", value)),
        }
    }
    if parts.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

fn is_denied_path(agent: &AgentConfig, path: &str) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern(path, pattern))
}

fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    let path = path.trim_start_matches("./");
    let pattern = pattern.trim_start_matches("./");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{}/", prefix));
    }
    path == pattern
}

fn is_strict_scope_subset(proposed: &[String], current: &[String]) -> bool {
    if proposed == current {
        return false;
    }
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

fn scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

fn format_log_record_timestamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts_from_unix_seconds(seconds);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

fn utc_parts_from_unix_seconds(seconds: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    (year, month, day, hour, minute, second)
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let days = days_since_unix_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

fn effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = vec![
        ".canon".to_string(),
        ".canon/**".to_string(),
        ".git".to_string(),
        ".git/**".to_string(),
    ];
    for pattern in &agent.ignore {
        if !patterns.iter().any(|existing| existing == pattern) {
            patterns.push(pattern.clone());
        }
    }
    patterns
}

fn evaluator_thread_config(agent: &AgentConfig, scope: &[String]) -> Value {
    let mut root_permissions = Map::new();
    if scope == full_scope() {
        root_permissions.insert(".".to_string(), Value::String("read".to_string()));
    } else {
        root_permissions.insert(".".to_string(), Value::String("none".to_string()));
        for path in scope {
            root_permissions.insert(path.clone(), Value::String("read".to_string()));
            root_permissions.insert(format!("{}/**", path), Value::String("read".to_string()));
        }
    }
    for pattern in effective_ignore_patterns(agent) {
        root_permissions.insert(pattern, Value::String("none".to_string()));
    }

    let mut filesystem = Map::new();
    filesystem.insert("/".to_string(), Value::String("read".to_string()));
    filesystem.insert(
        ":project_roots".to_string(),
        Value::Object(root_permissions),
    );
    for path in evaluator_runtime_read_paths() {
        filesystem.insert(path.to_string(), Value::String("read".to_string()));
    }
    filesystem.insert("glob_scan_max_depth".to_string(), json!(32));

    let mut profile = Map::new();
    profile.insert("filesystem".to_string(), Value::Object(filesystem));
    profile.insert("network".to_string(), json!({ "enabled": false }));

    let mut permissions = Map::new();
    permissions.insert("canon_check".to_string(), Value::Object(profile));

    let mut config = Map::new();
    config.insert(
        "default_permissions".to_string(),
        Value::String("canon_check".to_string()),
    );
    config.insert("permissions".to_string(), Value::Object(permissions));
    config.insert("history".to_string(), json!({ "persistence": "none" }));
    if !agent.plugins.is_empty() {
        config.insert("plugins".to_string(), enabled_plugins_config(agent));
    }
    Value::Object(config)
}

fn evaluator_runtime_read_paths() -> &'static [&'static str] {
    &[
        "/bin/**",
        "/usr/bin/**",
        "/usr/lib/**",
        "/usr/libexec/**",
        "/System/**",
        "/Library/**",
        "/opt/homebrew/**",
        "/private/tmp/**",
        "~/.codex/tmp/**",
        ":tmpdir",
        ":slash_tmp",
    ]
}

fn enabled_plugins_config(agent: &AgentConfig) -> Value {
    let mut plugins = Map::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), json!({ "enabled": true }));
    }
    Value::Object(plugins)
}

fn app_server_args(load_plugins: bool) -> Vec<&'static str> {
    let mut args = vec!["app-server"];
    if !load_plugins {
        args.push("--disable");
        args.push("plugins");
    }
    args.push("--listen");
    args.push("stdio://");
    args
}

struct AppServerRunner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl AppServerRunner {
    fn new(load_plugins: bool) -> Result<AppServerRunner, String> {
        let mut command = Command::new("codex");
        command.args(app_server_args(load_plugins));
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to start codex app-server: {}", err))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("failed to open app-server stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or("failed to open app-server stdout".to_string())?;
        let mut runner = AppServerRunner {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        runner.send_request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "canon",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
        )?;
        Ok(runner)
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server request: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server request: {}", err))?;
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("app-server {} failed: {}", method, error));
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("app-server {} response missing result", method));
            }
        }
    }

    fn send_turn_request(&mut self, method: &str, params: Value) -> Result<String, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server request: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server request: {}", err))?;

        let mut saw_response = false;
        let mut saw_completed = false;
        let mut text = String::new();
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("app-server {} failed: {}", method, error));
                }
                saw_response = true;
                if saw_completed {
                    return Ok(text);
                }
                continue;
            }
            match message.get("method").and_then(Value::as_str) {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = message
                        .get("params")
                        .and_then(|params| params.get("delta"))
                        .and_then(Value::as_str)
                    {
                        text.push_str(delta);
                    }
                }
                Some("turn/completed") => {
                    saw_completed = true;
                    if saw_response {
                        return Ok(text);
                    }
                }
                _ => {}
            }
        }
    }

    fn read_message(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .map_err(|err| format!("failed to read app-server response: {}", err))?;
        if bytes == 0 {
            return Err("app-server closed stdout".to_string());
        }
        serde_json::from_str(line.trim_end())
            .map_err(|err| format!("failed to parse app-server JSON: {}", err))
    }
}

impl Drop for AppServerRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        let result = self.send_request(
            "thread/start",
            json!({
                "cwd": root.display().to_string(),
                "developerInstructions": instructions,
                "approvalPolicy": "never",
                "config": evaluator_thread_config(agent, scope),
                "ephemeral": true,
                "sessionStartSource": "startup"
            }),
        )?;
        result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or("thread/start response missing thread.id".to_string())
    }

    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
        self.send_turn_request(
            "turn/start",
            json!({
                "threadId": session_id,
                "input": [
                    {
                        "type": "text",
                        "text": prompt
                    }
                ]
            }),
        )
    }
}

fn print_help() {
    print!(
        "{}",
        "canon - thread-scoped decisions and invariants\n\n\
Usage:\n  canon | canon pwd\n  canon p|path <key>\n  canon r|read <key>\n  canon w|write <key> [text]\n  canon a|append <key> [text]\n  canon d|del|delete|rm <key>\n  canon rg|g <pattern> [rg args...]\n"
            .to_string()
            + "  canon init\n  canon hook install\n  canon check [--fail-fast] [--ignore-cache] [expectation numbers...]\n  canon gate [expectation numbers...]\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(name: &str) -> PathBuf {
        let mut path = PathBuf::from("/tmp");
        path.push(format!("canon-test-{}-{}", name, process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn with_env<F>(name: &str, f: F)
    where
        F: FnOnce(PathBuf),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = temp_home(name);
        env::set_var("CANON_HOME", &home);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(home.clone());
        env::remove_var("CANON_HOME");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(home);
    }

    fn with_tmpdir<F>(name: &str, f: F)
    where
        F: FnOnce(PathBuf),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = temp_home(name);
        env::remove_var("CANON_HOME");
        env::set_var("TMPDIR", &temp);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(temp.clone());
        env::remove_var("TMPDIR");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(temp);
    }

    fn check_config_yaml() -> &'static str {
        r#"
version: 1
agent:
  instructions: |
    Answer from files only.
  ignore:
    - "target/**"
  plugins: []
expectations:
  - q: "First?"
    a: "yes"
  - q: "Second?"
    a: "no"
"#
    }

    fn parse_check_config(yaml: &str) -> Result<CheckConfig, String> {
        let config: CheckConfig = serde_yaml::from_str(yaml).map_err(|err| err.to_string())?;
        validate_check_config(&config)?;
        Ok(config)
    }

    struct FakeRunner {
        answers: VecDeque<String>,
        prompts: Vec<String>,
        sessions: Vec<String>,
        start_roots: Vec<PathBuf>,
        start_ignores: Vec<Vec<String>>,
        start_plugins: Vec<Vec<String>>,
        start_scopes: Vec<Vec<String>>,
        starts: usize,
    }

    impl FakeRunner {
        fn new(answers: &[&str]) -> FakeRunner {
            FakeRunner {
                answers: answers.iter().map(|answer| answer.to_string()).collect(),
                prompts: Vec::new(),
                sessions: Vec::new(),
                start_roots: Vec::new(),
                start_ignores: Vec::new(),
                start_plugins: Vec::new(),
                start_scopes: Vec::new(),
                starts: 0,
            }
        }
    }

    impl EvaluatorRunner for FakeRunner {
        fn start_session(
            &mut self,
            root: &Path,
            _instructions: &str,
            agent: &AgentConfig,
            scope: &[String],
        ) -> Result<String, String> {
            self.starts += 1;
            self.start_roots.push(root.to_path_buf());
            self.start_ignores.push(effective_ignore_patterns(agent));
            self.start_plugins.push(agent.plugins.clone());
            self.start_scopes.push(scope.to_vec());
            Ok(format!("session-{}", self.starts))
        }

        fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
            self.sessions.push(session_id.to_string());
            self.prompts.push(prompt.to_string());
            self.answers
                .pop_front()
                .ok_or("fake runner has no answer".to_string())
        }
    }

    fn git_project(name: &str) -> PathBuf {
        let root = temp_home(name);
        Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output()
            .unwrap();
        fs::write(root.join("README.md"), "hello").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        Command::new("git")
            .arg("add")
            .arg(".")
            .current_dir(&root)
            .output()
            .unwrap();
        root
    }

    fn check_options(
        config: &CheckConfig,
        numbers: &[&str],
        fail_fast: bool,
        ignore_cache: bool,
    ) -> CheckOptions {
        CheckOptions {
            selected: select_expectations(
                config,
                &numbers.iter().map(OsString::from).collect::<Vec<_>>(),
            )
            .unwrap(),
            fail_fast,
            ignore_cache,
        }
    }

    fn answer(answer: &str, evidence: &str, scope: &[&str]) -> String {
        format!(
            "ANSWER: {}\nEVIDENCE:\n{}\nSCOPE: {}",
            answer,
            evidence,
            serde_json::to_string(scope).unwrap()
        )
    }

    fn sample_record(number: usize, result: &str) -> CheckRecord {
        CheckRecord {
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            number,
            result: result.to_string(),
            prompt: "Question?".to_string(),
            expected: "yes".to_string(),
            observed: if result == "pass" { "yes" } else { "no" }.to_string(),
            evidence: "README.md has evidence".to_string(),
            scope: vec![".".to_string()],
            scope_hash: "AAAAAAAAAAAAAAAAAAAA".to_string(),
        }
    }

    #[test]
    fn hash_is_ten_base64url_chars() {
        let hash = hash_key("src/lib.rs");
        assert_eq!(hash.len(), 10);
        assert!(hash
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
    }

    #[test]
    fn missing_thread_id_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CODEX_THREAD_ID");
        env::set_var("CANON_HOME", temp_home("missing-thread"));
        let result = Config::from_env();
        assert!(result.is_err());
        env::remove_var("CANON_HOME");
    }

    #[test]
    fn canon_home_overrides_default_root() {
        with_env("home-override", |home| {
            let config = Config::from_env().unwrap();
            assert_eq!(config.root, home.join("codex").join("thread-test"));
        });
    }

    #[test]
    fn default_root_uses_tmpdir() {
        with_tmpdir("tmpdir-root", |temp| {
            let config = Config::from_env().unwrap();
            assert_eq!(
                config.root,
                temp.join("canon").join("codex").join("thread-test")
            );
        });
    }

    #[test]
    fn default_root_uses_slash_tmp_without_tmpdir() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CANON_HOME");
        env::remove_var("TMPDIR");
        env::set_var("CODEX_THREAD_ID", "thread-test");
        let config = Config::from_env().unwrap();
        assert_eq!(config.root, PathBuf::from("/tmp/canon/codex/thread-test"));
        env::remove_var("CODEX_THREAD_ID");
    }

    #[test]
    fn path_creation_is_deterministic() {
        with_env("deterministic", |_| {
            let config = Config::from_env().unwrap();
            let first = ensure_note(&config, "a/b.rs").unwrap();
            let second = ensure_note(&config, "a/b.rs").unwrap();
            assert_eq!(first.path, second.path);
            assert!(first.path.exists());
        });
    }

    #[test]
    fn write_and_append_preserve_metadata() {
        with_env("write-append", |_| {
            let config = Config::from_env().unwrap();
            write_note(&config, "src/main.rs", "body").unwrap();
            append_note(&config, "src/main.rs", "decision").unwrap();
            let note = note_for_key(&config, "src/main.rs");
            let content = fs::read_to_string(note.path).unwrap();
            assert!(content.starts_with("<!-- canon key=\"src/main.rs\" hash=\""));
            assert!(content.contains("\nbody\n"));
            assert!(content.contains("decision"));
        });
    }

    #[test]
    fn delete_removes_only_target() {
        with_env("delete", |_| {
            let config = Config::from_env().unwrap();
            let first = ensure_note(&config, "one").unwrap();
            let second = ensure_note(&config, "two").unwrap();
            delete_note(&config, "one").unwrap();
            assert!(!first.path.exists());
            assert!(second.path.exists());
            let index = fs::read_to_string(config.root.join("index.tsv")).unwrap();
            assert!(!index.contains("\tone\n"));
            assert!(index.contains("\ttwo\n"));
        });
    }

    #[test]
    fn collision_metadata_mismatch_fails() {
        with_env("collision", |_| {
            let config = Config::from_env().unwrap();
            let note = note_for_key(&config, "expected");
            ensure_dir(&config.root).unwrap();
            fs::write(&note.path, header("actual", &note.hash)).unwrap();
            let result = ensure_note(&config, "expected");
            assert!(result.is_err());
        });
    }

    #[test]
    fn aliases_work() {
        with_env("aliases", |_| {
            run(vec![]).unwrap();
            run(vec!["pwd".into()]).unwrap();
            run(vec!["p".into(), "file.rs".into()]).unwrap();
            run(vec!["path".into(), "file.rs".into()]).unwrap();
            run(vec!["w".into(), "file.rs".into(), "body".into()]).unwrap();
            run(vec!["a".into(), "file.rs".into(), "more".into()]).unwrap();
            run(vec!["read".into(), "file.rs".into()]).unwrap();
            run(vec!["d".into(), "file.rs".into()]).unwrap();
            assert!(run(vec!["-r".into()]).is_err());
            assert!(run(vec!["file.rs".into()]).is_err());
        });
    }

    #[test]
    fn init_creates_template_and_fails_when_existing() {
        let root = temp_home("init");
        run_init(&root).unwrap();
        let check_path = root.join(CHECK_PATH);
        assert_eq!(
            fs::read_to_string(&check_path).unwrap(),
            DEFAULT_CHECK_TEMPLATE
        );
        assert!(!root.join(".gitignore").exists());
        assert!(!root.join(PRE_COMMIT_HOOK_PATH).exists());
        assert!(run_init(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn init_does_not_require_thread_id() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CODEX_THREAD_ID");
        let root = temp_home("init-no-thread");
        run_init(&root).unwrap();
        assert!(root.join(CHECK_PATH).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hook_install_creates_reusable_pre_commit_hook() {
        let root = temp_home("hook-install");
        run_hook_install(&root).unwrap();
        let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
        assert!(!root.join(CHECK_PATH).exists());
        assert!(!root.join(".gitignore").exists());
        assert_eq!(
            fs::read_to_string(&hook_path).unwrap(),
            DEFAULT_PRE_COMMIT_HOOK
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                fs::metadata(&hook_path).unwrap().permissions().mode() & 0o111,
                0
            );
        }

        run_hook_install(&root).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hook_install_refuses_different_existing_pre_commit_hook() {
        let root = temp_home("hook-install-existing");
        let hook_path = root.join(PRE_COMMIT_HOOK_PATH);
        fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
        fs::write(&hook_path, "custom hook").unwrap();

        let err = run_hook_install(&root).unwrap_err();
        assert!(err.contains("already exists with different content"));
        assert!(!root.join(CHECK_PATH).exists());
        assert!(!root.join(".gitignore").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_config_accepts_minimal_schema() {
        let config = parse_check_config(check_config_yaml()).unwrap();
        assert_eq!(config.expectations.len(), 2);
        assert_eq!(config.agent.ignore, vec!["target/**"]);
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
        assert!(
            parse_check_options(&config, &["--fail-fast".into(), "--fail-fast".into()]).is_err()
        );
    }

    #[test]
    fn mixed_canon_and_non_canon_changes_fail() {
        assert!(fail_on_mixed_canon_paths(&[".canon/check.yml".to_string()]).is_ok());
        assert!(fail_on_mixed_canon_paths(&["src/main.rs".to_string()]).is_ok());
        assert!(fail_on_mixed_canon_paths(&[
            ".canon/check.yml".to_string(),
            "src/main.rs".to_string()
        ])
        .is_err());
    }

    #[test]
    fn parser_handles_answer_and_free_form_evidence() {
        let parsed =
            parse_evaluator_response("ANSWER: yes\nEVIDENCE:\nline: one\n- two\nSCOPE: [\".\"]")
                .unwrap();
        assert_eq!(parsed.answer, "yes");
        assert_eq!(parsed.evidence, "line: one\n- two");
        assert_eq!(parsed.scope, vec!["."]);
        assert!(parse_evaluator_response("yes").is_err());
    }

    #[test]
    fn check_runner_hides_expected_answers_and_reuses_session() {
        let root = git_project("check-runner");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1", "2"], false, true);
        let mut runner = FakeRunner::new(&[
            &answer("yes", "README.md says enough", &["."]),
            &answer("no", "README.md says enough", &["."]),
        ]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert!(records.iter().all(CheckRecord::passed));
        assert_eq!(runner.starts, 1);
        assert_eq!(runner.start_roots, vec![root.clone()]);
        assert_eq!(
            runner.start_ignores,
            vec![vec![
                ".canon".to_string(),
                ".canon/**".to_string(),
                ".git".to_string(),
                ".git/**".to_string(),
                "target/**".to_string()
            ]]
        );
        assert_eq!(runner.start_plugins, vec![Vec::<String>::new()]);
        assert_eq!(runner.start_scopes, vec![vec![".".to_string()]]);
        assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
        assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_fails_mismatch_and_treats_idk_as_exact_string() {
        let root = git_project("check-fails");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1", "2"], false, true);
        let mut runner = FakeRunner::new(&[
            &answer("idk", "not enough", &["."]),
            &answer("yes", "wrong", &["."]),
        ]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert!(!records[0].passed());
        assert!(!records[1].passed());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_fail_fast_stops_after_first_failure() {
        let root = git_project("check-fail-fast");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1", "2"], true, true);
        let mut runner = FakeRunner::new(&[&answer("no", "wrong", &["."])]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert_eq!(records.len(), 1);
        assert!(!records[0].passed());
        assert_eq!(runner.prompts.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_repairs_malformed_response_once() {
        let root = git_project("check-repair");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1"], false, true);
        let repaired = answer("yes", "README.md", &["."]);
        let mut runner = FakeRunner::new(&["not parseable", &repaired]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert!(records[0].passed());
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("could not be parsed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_warns_when_evidence_stays_empty() {
        let root = git_project("check-empty-evidence");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1"], false, true);
        let mut runner = FakeRunner::new(&[&answer("yes", "", &["."]), &answer("yes", "", &["."])]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert!(records[0].passed());
        assert!(records[0].evidence.is_empty());
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("no evidence"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_requires_human_review_for_malformed_answer() {
        let root = git_project("check-malformed-answer");
        let config = parse_check_config(check_config_yaml()).unwrap();
        let options = check_options(&config, &["1"], false, true);
        let malformed = answer("malformed", "question is malformed", &["."]);
        let mut runner = FakeRunner::new(&[&malformed, &malformed]);
        let records =
            run_check_with_runner(&root, &root, &config, &options, &mut runner, None).unwrap();
        assert!(!records[0].passed());
        assert_eq!(records[0].observed, "malformed");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn log_timestamp_uses_utc_rfc3339_format() {
        assert_eq!(format_log_record_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn diagnostic_log_is_written_to_numeric_active_file_and_flushed() {
        let root = git_project("check-log");
        let records = vec![sample_record(1, "pass")];
        let path = write_diagnostic_log(&root, &records).unwrap();
        assert_eq!(path, root.join(".git/canon/logs/0.jsonl"));
        let content = fs::read_to_string(&path).unwrap();
        let lines = content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let json: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(json["result"], "pass");
        assert_eq!(json["number"], 1);
        assert_eq!(json["prompt"], "Question?");
        assert_eq!(json["expected"], "yes");
        assert_eq!(json["observed"], "yes");
        assert_eq!(json["evidence"], "README.md has evidence");
        assert_eq!(json["scope"], json!(["."]));
        assert_eq!(json["scopeHash"], "AAAAAAAAAAAAAAAAAAAA");
        let expected_order = [
            "\"timestamp\"",
            "\"number\"",
            "\"result\"",
            "\"prompt\"",
            "\"expected\"",
            "\"observed\"",
            "\"evidence\"",
            "\"scope\"",
            "\"scopeHash\"",
        ];
        let mut previous = 0;
        for key in expected_order {
            let index = lines[0].find(key).unwrap();
            assert!(index >= previous);
            previous = index;
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn diagnostic_log_rotates_at_start_when_active_file_is_large() {
        let root = git_project("check-log-rotate");
        let log_dir = root.join(".git/canon/logs");
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(
            log_dir.join("0.jsonl"),
            "x".repeat((DIAGNOSTIC_LOG_MAX_BYTES + 1) as usize),
        )
        .unwrap();
        fs::write(log_dir.join("1.jsonl"), "one").unwrap();
        fs::write(log_dir.join("2.jsonl"), "two").unwrap();
        fs::write(log_dir.join("3.jsonl"), "three").unwrap();

        let writer = DiagnosticLogWriter::create(&root).unwrap();
        assert_eq!(writer.path, log_dir.join("0.jsonl"));
        assert!(!log_dir.join("0.jsonl").exists());
        assert_eq!(
            fs::read_to_string(log_dir.join("1.jsonl")).unwrap().len(),
            (DIAGNOSTIC_LOG_MAX_BYTES + 1) as usize
        );
        assert_eq!(fs::read_to_string(log_dir.join("2.jsonl")).unwrap(), "one");
        assert_eq!(fs::read_to_string(log_dir.join("3.jsonl")).unwrap(), "two");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn evaluator_permissions_always_deny_canon_and_agent_ignores() {
        let agent = AgentConfig {
            instructions: "Answer from files only.".to_string(),
            ignore: vec!["target/**".to_string()],
            plugins: Vec::new(),
        };
        let config = evaluator_thread_config(&agent, &full_scope());
        let root_permissions = config["permissions"]["canon_check"]["filesystem"][":project_roots"]
            .as_object()
            .unwrap();
        assert_eq!(root_permissions["."], "read");
        assert_eq!(root_permissions[".canon"], "none");
        assert_eq!(root_permissions[".canon/**"], "none");
        assert_eq!(root_permissions[".git"], "none");
        assert_eq!(root_permissions[".git/**"], "none");
        assert_eq!(root_permissions["target/**"], "none");
        assert_eq!(
            config["permissions"]["canon_check"]["filesystem"]["/"],
            "read"
        );
        assert_eq!(
            config["permissions"]["canon_check"]["filesystem"]["~/.codex/tmp/**"],
            "read"
        );
        assert_eq!(config["history"]["persistence"], "none");
        assert!(config.get("plugins").is_none());
    }

    #[test]
    fn evaluator_plugin_list_is_explicitly_configured() {
        let config = parse_check_config(
            r#"
version: 1
agent:
  instructions: x
  ignore: []
  plugins:
    - "canon@codex-plugins"
expectations:
  - q: "Question?"
    a: "yes"
"#,
        )
        .unwrap();
        assert!(check_config_loads_plugins(&config));
        let thread_config = evaluator_thread_config(&config.agent, &full_scope());
        assert_eq!(
            thread_config["plugins"]["canon@codex-plugins"]["enabled"],
            json!(true)
        );
    }

    #[test]
    fn app_server_starts_with_plugins_disabled_by_default() {
        assert_eq!(
            app_server_args(false),
            vec!["app-server", "--disable", "plugins", "--listen", "stdio://"]
        );
        assert_eq!(
            app_server_args(true),
            vec!["app-server", "--listen", "stdio://"]
        );
    }
}
