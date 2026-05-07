use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};
use tempfile::TempDir;
use walkdir::WalkDir;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
const CHECK_LOG_DIR: &str = ".canon/logs";
const CHECK_LOG_MAX_BYTES: u64 = 100 * 1024 * 1024;
const CHECK_LOG_MAX_FILES: usize = 10;
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
    instructions: String,
    agents: BTreeMap<String, AgentConfig>,
    expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    paths: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
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
    q: String,
    a: String,
}

#[derive(Debug)]
struct ParsedAnswer {
    answer: String,
    evidence: String,
}

#[derive(Debug)]
struct CheckRecord {
    number: usize,
    agent: String,
    prompt: String,
    expected: String,
    observed: String,
    evidence: String,
    warning: Option<String>,
    passed: bool,
}

trait EvaluatorRunner {
    fn start_session(
        &mut self,
        agent_name: &str,
        workspace: &Path,
        instructions: &str,
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
    let selected = select_expectations(&config, args)?;
    fail_on_mixed_canon_changes(root)?;
    let mut runner = AppServerRunner::new()?;
    let records = run_check_with_runner(root, &config, &selected, &mut runner)?;
    print_check_report(&records);
    write_check_log(root, &records)?;
    if records.iter().all(|record| record.passed) {
        Ok(())
    } else {
        Err("canon check failed".to_string())
    }
}

fn load_check_config(root: &Path) -> Result<CheckConfig, String> {
    let path = root.join(CHECK_PATH);
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let config: CheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", path.display(), err))?;
    validate_check_config(&config)?;
    Ok(config)
}

fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if config.instructions.trim().is_empty() {
        return Err("check.yml instructions must not be empty".to_string());
    }
    if config.agents.is_empty() {
        return Err("check.yml agents must not be empty".to_string());
    }
    for (name, agent) in &config.agents {
        if name.trim().is_empty() {
            return Err("agent names must not be empty".to_string());
        }
        if agent.paths.is_empty() {
            return Err(format!("agent {} must have at least one path", name));
        }
        for path in &agent.paths {
            validate_relative_config_path(path, "agent path")?;
        }
        for path in &agent.exclude {
            if path.trim().is_empty() {
                return Err(format!("agent {} has an empty exclude pattern", name));
            }
        }
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

fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty() {
        return Err(format!("{} must not be empty", label));
    }
    if path.is_absolute() {
        return Err(format!("{} must be relative: {}", label, value));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("{} must not contain '..': {}", label, value));
    }
    Ok(())
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
                q: expectation.q.clone(),
                a: expectation.a.clone(),
            }
        })
        .collect())
}

fn fail_on_mixed_canon_changes(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=all")
        .current_dir(root)
        .output()
        .map_err(|err| format!("failed to run git status: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect git changes".to_string());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git status output must be valid UTF-8".to_string())?;
    fail_on_mixed_canon_paths(&changed_paths_from_status(&stdout))
}

fn changed_paths_from_status(status: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in status.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = &line[3..];
        if let Some((left, right)) = path.split_once(" -> ") {
            paths.push(left.to_string());
            paths.push(right.to_string());
        } else {
            paths.push(path.to_string());
        }
    }
    paths
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
    config: &CheckConfig,
    selected: &[SelectedExpectation],
    runner: &mut R,
) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    for (agent_name, agent) in &config.agents {
        let workspace = create_agent_workspace(root, agent)?;
        let session_id =
            runner.start_session(agent_name, workspace.path(), &config.instructions)?;
        for expectation in selected {
            let prompt = question_prompt(&config.instructions, &expectation.q);
            let response = ask_with_repairs(runner, &session_id, &prompt)?;
            let needs_human_review = response.answer == "malformed";
            let passed = !needs_human_review
                && response.answer == expectation.a
                && (response.answer == "skip" && expectation.a == "skip"
                    || response.answer != "skip");
            let warning = if needs_human_review {
                Some(MALFORMED_REVIEW_WARNING.to_string())
            } else if response.evidence.trim().is_empty() {
                Some("evidence is empty after retry".to_string())
            } else {
                None
            };
            records.push(CheckRecord {
                number: expectation.number,
                agent: agent_name.clone(),
                prompt: expectation.q.clone(),
                expected: expectation.a.clone(),
                observed: response.answer,
                evidence: response.evidence,
                warning,
                passed,
            });
        }
    }
    Ok(records)
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

    if parsed.evidence.trim().is_empty() {
        let repaired = runner.ask(session_id, evidence_repair_prompt())?;
        if let Ok(answer) = parse_evaluator_response(&repaired) {
            parsed = answer;
        }
    }

    Ok(parsed)
}

fn question_prompt(instructions: &str, question: &str) -> String {
    format!(
        "{}\n\nExpectation:\n{}\n\nReply using this exact format:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\n",
        instructions.trim(),
        question
    )
}

fn malformed_repair_prompt() -> &'static str {
    "Your previous response could not be parsed. Reply again using exactly:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\n"
}

fn evidence_repair_prompt() -> &'static str {
    "Your previous response had an answer but no evidence. Reply again with the same format and include evidence if the available files support it:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence>\n"
}

fn parse_evaluator_response(text: &str) -> Result<ParsedAnswer, String> {
    let mut lines = text.lines();
    let first = lines
        .next()
        .ok_or("missing ANSWER line".to_string())?
        .trim_end();
    let answer = first
        .strip_prefix("ANSWER: ")
        .ok_or("missing ANSWER line".to_string())?
        .to_string();
    let second = lines
        .next()
        .ok_or("missing EVIDENCE block".to_string())?
        .trim_end();
    if second != "EVIDENCE:" {
        return Err("missing EVIDENCE block".to_string());
    }
    Ok(ParsedAnswer {
        answer,
        evidence: lines.collect::<Vec<_>>().join("\n"),
    })
}

fn print_check_report(records: &[CheckRecord]) {
    for record in records {
        let status = if record.passed { "PASS" } else { "FAIL" };
        println!("{} {} [{}]", status, record.number, record.agent);
        println!("Prompt: {}", record.prompt);
        println!("Expected: {}", record.expected);
        println!("Observed: {}", record.observed);
        println!("Evidence:\n{}", record.evidence);
        if let Some(warning) = &record.warning {
            println!("Warning: {}", warning);
        }
        println!("Rerun: canon check {}", record.number);
        println!();
    }
}

fn write_check_log(root: &Path, records: &[CheckRecord]) -> Result<PathBuf, String> {
    let log_dir = root.join(CHECK_LOG_DIR);
    ensure_dir(&log_dir)?;
    let seconds = unix_timestamp()?;
    let filename_timestamp = format_log_filename_timestamp(seconds);
    let record_timestamp = format_log_record_timestamp(seconds);
    let path = log_dir.join(format!("{}.jsonl", filename_timestamp));
    fs::write(&path, render_check_log(records, &record_timestamp))
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    prune_check_logs(&log_dir, CHECK_LOG_MAX_BYTES, CHECK_LOG_MAX_FILES)?;
    Ok(path)
}

fn render_check_log(records: &[CheckRecord], timestamp: &str) -> String {
    let mut output = String::new();
    for record in records {
        let mut value = json!({
            "timestamp": timestamp,
            "number": record.number,
            "result": if record.passed { "pass" } else { "fail" },
            "agent": record.agent,
            "prompt": record.prompt,
            "expected": record.expected,
            "observed": record.observed,
            "evidence": record.evidence,
        });
        if let Some(warning) = &record.warning {
            value["warning"] = Value::String(warning.clone());
        }
        output.push_str(&serde_json::to_string(&value).expect("check log record is serializable"));
        output.push('\n');
    }
    output
}

#[derive(Debug)]
struct CheckLogFile {
    path: PathBuf,
    name: String,
    size: u64,
}

fn prune_check_logs(log_dir: &Path, max_bytes: u64, max_files: usize) -> Result<(), String> {
    let mut logs = check_log_files(log_dir)?;
    logs.sort_by(|left, right| left.name.cmp(&right.name));
    let mut total_size = logs.iter().map(|log| log.size).sum::<u64>();

    while !logs.is_empty() && (logs.len() > max_files || (total_size > max_bytes && logs.len() > 1))
    {
        let oldest = logs.remove(0);
        total_size = total_size.saturating_sub(oldest.size);
        fs::remove_file(&oldest.path)
            .map_err(|err| format!("failed to remove {}: {}", oldest.path.display(), err))?;
    }

    Ok(())
}

fn check_log_files(log_dir: &Path) -> Result<Vec<CheckLogFile>, String> {
    if !log_dir.exists() {
        return Ok(Vec::new());
    }

    let mut logs = Vec::new();
    for entry in fs::read_dir(log_dir)
        .map_err(|err| format!("failed to read {}: {}", log_dir.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read log entry: {}", err))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| format!("failed to inspect {}: {}", path.display(), err))?;
        if !metadata.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("log filename must be valid UTF-8: {}", path.display()))?
            .to_string();
        logs.push(CheckLogFile {
            path,
            name,
            size: metadata.len(),
        });
    }
    Ok(logs)
}

fn format_log_filename_timestamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts_from_unix_seconds(seconds);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        year, month, day, hour, minute, second
    )
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

fn create_agent_workspace(root: &Path, agent: &AgentConfig) -> Result<TempDir, String> {
    let temp = tempfile::tempdir().map_err(|err| format!("failed to create temp dir: {}", err))?;
    let exclude = build_exclude_set(&agent.exclude)?;
    for configured in &agent.paths {
        let source = root.join(configured);
        if !source.exists() {
            return Err(format!("agent path does not exist: {}", configured));
        }
        copy_visible_path(root, &source, temp.path(), &exclude)?;
    }
    Ok(temp)
}

fn build_exclude_set(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    builder.add(Glob::new(".canon/**").map_err(|err| err.to_string())?);
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|err| format!("bad exclude pattern: {}", err))?);
    }
    builder
        .build()
        .map_err(|err| format!("failed to build excludes: {}", err))
}

fn copy_visible_path(
    root: &Path,
    source: &Path,
    destination_root: &Path,
    exclude: &GlobSet,
) -> Result<(), String> {
    if source.is_file() {
        copy_one_file(root, source, destination_root, exclude)?;
        return Ok(());
    }

    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|err| format!("failed to walk files: {}", err))?;
        let path = entry.path();
        if path == source && source == root.join(".") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| format!("path is outside root: {}", path.display()))?;
        if rel.as_os_str().is_empty() || excluded_path(rel, exclude) {
            if entry.file_type().is_dir() {
                continue;
            }
            continue;
        }
        let target = destination_root.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|err| format!("failed to create {}: {}", target.display(), err))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
            }
            fs::copy(path, &target).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {}",
                    path.display(),
                    target.display(),
                    err
                )
            })?;
        }
    }
    Ok(())
}

fn copy_one_file(
    root: &Path,
    source: &Path,
    destination_root: &Path,
    exclude: &GlobSet,
) -> Result<(), String> {
    let rel = source
        .strip_prefix(root)
        .map_err(|_| format!("path is outside root: {}", source.display()))?;
    if excluded_path(rel, exclude) {
        return Ok(());
    }
    let target = destination_root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }
    fs::copy(source, &target).map_err(|err| {
        format!(
            "failed to copy {} to {}: {}",
            source.display(),
            target.display(),
            err
        )
    })?;
    Ok(())
}

fn excluded_path(path: &Path, exclude: &GlobSet) -> bool {
    let normalized = normalize_path_for_glob(path);
    exclude.is_match(&normalized)
}

fn normalize_path_for_glob(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

struct AppServerRunner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl AppServerRunner {
    fn new() -> Result<AppServerRunner, String> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
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
        _agent_name: &str,
        workspace: &Path,
        instructions: &str,
    ) -> Result<String, String> {
        let result = self.send_request(
            "thread/start",
            json!({
                "cwd": workspace.display().to_string(),
                "developerInstructions": instructions,
                "approvalPolicy": "never",
                "sandbox": "read-only",
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
                ],
                "sandboxPolicy": {
                    "type": "readOnly",
                    "networkAccess": false
                }
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
            + "  canon init\n  canon hook install\n  canon check [expectation numbers...]\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(name: &str) -> PathBuf {
        let mut path = env::temp_dir();
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
instructions: |
  Answer from files only.
agents:
  project:
    paths:
      - "."
    exclude:
      - ".canon/**"
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
        starts: usize,
    }

    impl FakeRunner {
        fn new(answers: &[&str]) -> FakeRunner {
            FakeRunner {
                answers: answers.iter().map(|answer| answer.to_string()).collect(),
                prompts: Vec::new(),
                sessions: Vec::new(),
                starts: 0,
            }
        }
    }

    impl EvaluatorRunner for FakeRunner {
        fn start_session(
            &mut self,
            _agent_name: &str,
            _workspace: &Path,
            _instructions: &str,
        ) -> Result<String, String> {
            self.starts += 1;
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
        assert!(config.agents.contains_key("project"));
    }

    #[test]
    fn check_config_rejects_missing_required_fields() {
        assert!(parse_check_config("version: 1\n").is_err());
        assert!(
            parse_check_config("version: 1\ninstructions: x\nagents: {}\nexpectations: []\n")
                .is_err()
        );
        assert!(parse_check_config(
            "version: 1\ninstructions: x\nagents:\n  a:\n    paths: []\nexpectations:\n  - q: x\n    a: y\n"
        )
        .is_err());
    }

    #[test]
    fn check_config_rejects_unsupported_expectation_fields() {
        let yaml = r#"
version: 1
instructions: x
agents:
  project:
    paths: ["."]
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
        let parsed = parse_evaluator_response("ANSWER: yes\nEVIDENCE:\nline: one\n- two").unwrap();
        assert_eq!(parsed.answer, "yes");
        assert_eq!(parsed.evidence, "line: one\n- two");
        assert!(parse_evaluator_response("yes").is_err());
    }

    #[test]
    fn check_runner_hides_expected_answers_and_reuses_session() {
        let root = temp_home("check-runner");
        fs::write(root.join("README.md"), "hello").unwrap();
        let config = parse_check_config(check_config_yaml()).unwrap();
        let selected = select_expectations(&config, &["1".into(), "2".into()]).unwrap();
        let mut runner = FakeRunner::new(&[
            "ANSWER: yes\nEVIDENCE:\nREADME.md says enough",
            "ANSWER: no\nEVIDENCE:\nREADME.md says enough",
        ]);
        let records = run_check_with_runner(&root, &config, &selected, &mut runner).unwrap();
        assert!(records.iter().all(|record| record.passed));
        assert_eq!(runner.starts, 1);
        assert_eq!(runner.sessions, vec!["session-1", "session-1"]);
        assert!(runner.prompts.iter().all(|prompt| !prompt.contains("a:")));
        assert!(runner
            .prompts
            .iter()
            .all(|prompt| !prompt.contains("yes\n")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_fails_mismatch_and_skip() {
        let root = temp_home("check-fails");
        fs::write(root.join("README.md"), "hello").unwrap();
        let config = parse_check_config(check_config_yaml()).unwrap();
        let selected = select_expectations(&config, &["1".into(), "2".into()]).unwrap();
        let mut runner = FakeRunner::new(&[
            "ANSWER: skip\nEVIDENCE:\nnot enough",
            "ANSWER: yes\nEVIDENCE:\nwrong",
        ]);
        let records = run_check_with_runner(&root, &config, &selected, &mut runner).unwrap();
        assert!(!records[0].passed);
        assert!(!records[1].passed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_repairs_malformed_response_once() {
        let root = temp_home("check-repair");
        fs::write(root.join("README.md"), "hello").unwrap();
        let config = parse_check_config(check_config_yaml()).unwrap();
        let selected = select_expectations(&config, &["1".into()]).unwrap();
        let mut runner = FakeRunner::new(&["not parseable", "ANSWER: yes\nEVIDENCE:\nREADME.md"]);
        let records = run_check_with_runner(&root, &config, &selected, &mut runner).unwrap();
        assert!(records[0].passed);
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("could not be parsed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_warns_when_evidence_stays_empty() {
        let root = temp_home("check-empty-evidence");
        fs::write(root.join("README.md"), "hello").unwrap();
        let config = parse_check_config(check_config_yaml()).unwrap();
        let selected = select_expectations(&config, &["1".into()]).unwrap();
        let mut runner = FakeRunner::new(&["ANSWER: yes\nEVIDENCE:\n", "ANSWER: yes\nEVIDENCE:\n"]);
        let records = run_check_with_runner(&root, &config, &selected, &mut runner).unwrap();
        assert!(records[0].passed);
        assert!(records[0].warning.is_some());
        assert_eq!(runner.prompts.len(), 2);
        assert!(runner.prompts[1].contains("no evidence"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_runner_requires_human_review_for_malformed_answer() {
        let root = temp_home("check-malformed-answer");
        fs::write(root.join("README.md"), "hello").unwrap();
        let config = parse_check_config(check_config_yaml()).unwrap();
        let selected = select_expectations(&config, &["1".into()]).unwrap();
        let mut runner = FakeRunner::new(&["ANSWER: malformed\nEVIDENCE:\nquestion is malformed"]);
        let records = run_check_with_runner(&root, &config, &selected, &mut runner).unwrap();
        assert!(!records[0].passed);
        assert_eq!(
            records[0].warning.as_deref(),
            Some(MALFORMED_REVIEW_WARNING)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn log_timestamp_uses_yyyymmdd_hhmmss_format() {
        assert_eq!(format_log_filename_timestamp(0), "19700101-000000");
        assert_eq!(format_log_filename_timestamp(86_399), "19700101-235959");
        assert_eq!(format_log_filename_timestamp(86_400), "19700102-000000");
        assert_eq!(format_log_record_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn check_log_is_written_and_old_logs_are_pruned() {
        let root = temp_home("check-log");
        let log_dir = root.join(CHECK_LOG_DIR);
        fs::create_dir_all(&log_dir).unwrap();
        for day in 1..=CHECK_LOG_MAX_FILES {
            fs::write(
                log_dir.join(format!("200001{:02}-000000.jsonl", day)),
                "old",
            )
            .unwrap();
        }

        let records = vec![CheckRecord {
            number: 1,
            agent: "Smith".to_string(),
            prompt: "Question?".to_string(),
            expected: "yes".to_string(),
            observed: "yes".to_string(),
            evidence: "README.md has evidence".to_string(),
            warning: None,
            passed: true,
        }];
        let path = write_check_log(&root, &records).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename.len(), "YYYYMMDD-HHMMSS.jsonl".len());
        assert_eq!(&filename[8..9], "-");
        assert!(filename.ends_with(".jsonl"));
        assert!(filename[..8].chars().all(|ch| ch.is_ascii_digit()));
        assert!(filename[9..15].chars().all(|ch| ch.is_ascii_digit()));
        let content = fs::read_to_string(&path).unwrap();
        let lines = content.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let json: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(json["result"], "pass");
        assert_eq!(json["number"], 1);
        assert_eq!(json["agent"], "Smith");
        assert_eq!(json["prompt"], "Question?");
        assert_eq!(json["expected"], "yes");
        assert_eq!(json["observed"], "yes");
        assert_eq!(json["evidence"], "README.md has evidence");
        let timestamp = json["timestamp"].as_str().unwrap();
        assert_eq!(timestamp.len(), "YYYY-MM-DDTHH:MM:SSZ".len());
        assert!(timestamp.ends_with('Z'));
        assert_eq!(
            check_log_files(&log_dir).unwrap().len(),
            CHECK_LOG_MAX_FILES
        );
        assert!(!log_dir.join("20000101-000000.jsonl").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_log_pruning_uses_total_size_limit() {
        let root = temp_home("check-log-size");
        let log_dir = root.join(CHECK_LOG_DIR);
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(log_dir.join("20000101-000000.jsonl"), "12345").unwrap();
        fs::write(log_dir.join("20000102-000000.jsonl"), "12345").unwrap();
        fs::write(log_dir.join("20000103-000000.jsonl"), "12345").unwrap();

        prune_check_logs(&log_dir, 10, 10).unwrap();

        assert!(!log_dir.join("20000101-000000.jsonl").exists());
        assert!(log_dir.join("20000102-000000.jsonl").exists());
        assert!(log_dir.join("20000103-000000.jsonl").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn agent_workspace_always_excludes_canon() {
        let root = temp_home("workspace");
        fs::create_dir_all(root.join(".canon")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".canon/check.yml"), "secret").unwrap();
        fs::write(root.join("src/main.rs"), "code").unwrap();
        let agent = AgentConfig {
            paths: vec![".".to_string()],
            exclude: Vec::new(),
        };
        let workspace = create_agent_workspace(&root, &agent).unwrap();
        assert!(workspace.path().join("src/main.rs").exists());
        assert!(!workspace.path().join(".canon/check.yml").exists());
        let _ = fs::remove_dir_all(root);
    }
}
