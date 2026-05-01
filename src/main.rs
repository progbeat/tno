use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

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

fn main() {
    if let Err(err) = run(env::args_os().skip(1).collect()) {
        eprintln!("tno: {}", err);
        process::exit(1);
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    let config = Config::from_env()?;

    if args.is_empty() || (args.len() == 1 && args[0] == "-r") {
        ensure_dir(&config.root)?;
        println!("{}", config.root.display());
        return Ok(());
    }

    let first = arg_to_string(&args[0])?;
    match first.as_str() {
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
            let text = collect_text(&args, 2)?;
            write_note(&config, key, &text)?;
        }
        "a" | "append" => {
            let key = require_key(&args, 1)?;
            let text = collect_text(&args, 2)?;
            append_note(&config, key, &text)?;
        }
        "d" | "del" | "delete" | "rm" => {
            let key = require_key(&args, 1)?;
            delete_note(&config, key)?;
        }
        "rg" | "g" => {
            run_rg(&config, &args[1..])?;
        }
        "-h" | "--help" | "help" => {
            print_help();
        }
        _ => {
            let note = ensure_note(&config, &first)?;
            println!("{}", note.path.display());
        }
    }

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

        if let Some(value) = env::var_os("TNO_HOME") {
            if !value.is_empty() {
                return Ok(Config {
                    root: PathBuf::from(value).join("codex").join(thread_id),
                });
            }
        }

        if let Some(session_file) = find_codex_session_file(&thread_id)? {
            let mut root = session_file;
            root.set_extension("tno");
            return Ok(Config { root });
        }

        let home = env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(Config {
            root: PathBuf::from(home)
                .join(".thread-notes")
                .join("codex")
                .join(thread_id),
        })
    }
}

fn find_codex_session_file(thread_id: &str) -> Result<Option<PathBuf>, String> {
    let codex_home = match env::var_os("CODEX_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            let home = env::var_os("HOME").ok_or("HOME is not set")?;
            PathBuf::from(home).join(".codex")
        }
    };
    let sessions = codex_home.join("sessions");
    if !sessions.exists() {
        return Ok(None);
    }
    find_file_containing(&sessions, thread_id)
}

fn find_file_containing(root: &Path, needle: &str) -> Result<Option<PathBuf>, String> {
    let entries =
        fs::read_dir(root).map_err(|err| format!("failed to read {}: {}", root.display(), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {}", err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to stat {}: {}", path.display(), err))?;
        if file_type.is_dir() {
            if let Some(found) = find_file_containing(&path, needle)? {
                return Ok(Some(found));
            }
        } else if file_type.is_file() {
            let name = entry.file_name();
            if name.to_string_lossy().contains(needle) {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
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
    if args.len() <= start {
        return Err("missing text".to_string());
    }
    let mut parts = Vec::new();
    for arg in &args[start..] {
        parts.push(arg.to_str().ok_or("text must be valid UTF-8".to_string())?);
    }
    Ok(parts.join(" "))
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
        return Err(format!("note not found for key: {}", key));
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
        "<!-- tno key=\"{}\" hash=\"{}\" -->\n# {}\n",
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
        .ok_or_else(|| format!("missing tno metadata in {}", path.display()))?;
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
    let prefix = "<!-- tno key=\"";
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

fn print_help() {
    println!(
        "tno - thread-scoped notes\n\n\
Usage:\n  tno | tno -r\n  tno <key>\n  tno p|path <key>\n  tno r|read <key>\n  tno w|write <key> <text>\n  tno a|append <key> <text>\n  tno d|del|delete|rm <key>\n  tno rg|g <pattern> [rg args...]\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_home(name: &str) -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!("tno-test-{}-{}", name, process::id()));
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
        env::set_var("TNO_HOME", &home);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(home.clone());
        env::remove_var("TNO_HOME");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(home);
    }

    fn with_codex_home<F>(name: &str, f: F)
    where
        F: FnOnce(PathBuf),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = temp_home(name);
        let codex_home = home.join("codex-home");
        let session_dir = codex_home.join("sessions/2026/05/01");
        fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("rollout-2026-05-01T00-00-00-thread-test.jsonl");
        fs::write(&session_file, "{}\n").unwrap();
        env::remove_var("TNO_HOME");
        env::set_var("CODEX_HOME", &codex_home);
        env::set_var("CODEX_THREAD_ID", "thread-test");
        f(session_file.clone());
        env::remove_var("CODEX_HOME");
        env::remove_var("CODEX_THREAD_ID");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn hash_is_ten_base64url_chars() {
        let hash = hash_key("swap/src/swap/main.py");
        assert_eq!(hash.len(), 10);
        assert!(hash
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
    }

    #[test]
    fn missing_thread_id_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        env::remove_var("CODEX_THREAD_ID");
        env::set_var("TNO_HOME", temp_home("missing-thread"));
        let result = Config::from_env();
        assert!(result.is_err());
        env::remove_var("TNO_HOME");
    }

    #[test]
    fn tno_home_overrides_default_root() {
        with_env("home-override", |home| {
            let config = Config::from_env().unwrap();
            assert_eq!(config.root, home.join("codex").join("thread-test"));
        });
    }

    #[test]
    fn codex_session_sidecar_is_default_root() {
        with_codex_home("sidecar", |session_file| {
            let config = Config::from_env().unwrap();
            let mut expected = session_file;
            expected.set_extension("tno");
            assert_eq!(config.root, expected);
        });
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
            assert!(content.starts_with("<!-- tno key=\"src/main.rs\" hash=\""));
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
            run(vec!["p".into(), "file.rs".into()]).unwrap();
            run(vec!["path".into(), "file.rs".into()]).unwrap();
            run(vec!["w".into(), "file.rs".into(), "body".into()]).unwrap();
            run(vec!["a".into(), "file.rs".into(), "more".into()]).unwrap();
            run(vec!["read".into(), "file.rs".into()]).unwrap();
            run(vec!["d".into(), "file.rs".into()]).unwrap();
        });
    }
}
