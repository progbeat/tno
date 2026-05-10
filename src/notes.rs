use crate::*;

pub(crate) fn require_key(args: &[OsString], index: usize) -> Result<&str, String> {
    let key = args
        .get(index)
        .ok_or("missing key".to_string())
        .and_then(|arg| arg.to_str().ok_or("key must be valid UTF-8".to_string()))?;
    validate_note_key(key)?;
    Ok(key)
}

pub(crate) fn arg_to_string(arg: &OsString) -> Result<String, String> {
    arg.to_str()
        .map(|value| value.to_string())
        .ok_or("argument must be valid UTF-8".to_string())
}

pub(crate) fn collect_text(args: &[OsString], start: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    let rest = args.get(start..).ok_or_else(|| {
        format!(
            "text start index {} exceeds argument count {}",
            start,
            args.len()
        )
    })?;
    for arg in rest {
        parts.push(arg.to_str().ok_or("text must be valid UTF-8".to_string())?);
    }
    Ok(parts.join(" "))
}

pub(crate) fn collect_text_or_stdin(args: &[OsString], start: usize) -> Result<String, String> {
    if args.len() > start {
        return collect_text(args, start);
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| format!("failed to read stdin: {}", err))?;
    Ok(text)
}

pub(crate) fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create {}: {}", path.display(), err))
}

pub(crate) fn ensure_note(config: &Config, key: &str) -> Result<Note, String> {
    ensure_dir(&config.root)?;
    let note = note_for_key(config, key)?;
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

pub(crate) fn note_for_key(config: &Config, key: &str) -> Result<Note, String> {
    validate_note_key(key)?;
    let hash = hash_key(key);
    let path = config.root.join(format!("{}.md", hash));
    Ok(Note {
        key: key.to_string(),
        hash,
        path,
    })
}

pub(crate) fn read_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key)?;
    if !note.path.exists() {
        return Err(format!("canon not found for key: {}", key));
    }
    let content = fs::read_to_string(&note.path)
        .map_err(|err| format!("failed to read {}: {}", note.path.display(), err))?;
    verify_note_key_from_first_line(&note.path, content.lines().next().unwrap_or(""), key)?;
    print!("{}", content);
    Ok(())
}

pub(crate) fn write_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
    let note = ensure_note(config, key)?;
    let content = format!(
        "{}{}\n",
        header(&note.key, &note.hash),
        normalize_body(text)
    );
    fs::write(&note.path, content)
        .map_err(|err| format!("failed to write {}: {}", note.path.display(), err))
}

pub(crate) fn append_note(config: &Config, key: &str, text: &str) -> Result<(), String> {
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

pub(crate) fn delete_note(config: &Config, key: &str) -> Result<(), String> {
    let note = note_for_key(config, key)?;
    if note.path.exists() {
        verify_note_key(&note.path, key)?;
        fs::remove_file(&note.path)
            .map_err(|err| format!("failed to delete {}: {}", note.path.display(), err))?;
    }
    remove_index(config, &note.hash, key)
}

pub(crate) fn validate_note_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    if key.contains('\t') || key.contains('\n') || key.contains('\r') {
        return Err("key must not contain tabs or newlines".to_string());
    }
    Ok(())
}

pub(crate) fn run_rg(config: &Config, rg_args: &[OsString]) -> Result<(), String> {
    if rg_args.is_empty() {
        return Err("missing rg pattern".to_string());
    }
    ensure_dir(&config.root)?;
    let mut command = Command::new("rg");
    command.args(rg_args);
    command.arg("--");
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

pub(crate) fn initial_content(key: &str, hash: &str) -> String {
    header(key, hash)
}

pub(crate) fn header(key: &str, hash: &str) -> String {
    format!(
        "<!-- canon key=\"{}\" hash=\"{}\" -->\n# {}\n",
        escape_attr(key),
        hash,
        key
    )
}

pub(crate) fn normalize_body(text: &str) -> String {
    let mut value = text.to_string();
    while value.ends_with('\n') {
        value.pop();
    }
    value
}

pub(crate) fn verify_note_key(path: &Path, expected_key: &str) -> Result<(), String> {
    let first = first_line(path)?;
    verify_note_key_from_first_line(path, &first, expected_key)
}

pub(crate) fn verify_note_key_from_first_line(
    path: &Path,
    first: &str,
    expected_key: &str,
) -> Result<(), String> {
    let actual_key = parse_key_from_header(first)
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

pub(crate) fn first_line(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

pub(crate) fn parse_key_from_header(line: &str) -> Option<String> {
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
                    _ => return None,
                }
            }
            other => out.push(other),
        }
    }
    None
}

pub(crate) fn escape_attr(value: &str) -> String {
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

pub(crate) fn upsert_index(config: &Config, hash: &str, key: &str) -> Result<(), String> {
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(_, existing_key)| existing_key != key);
    entries.push((hash.to_string(), key.to_string()));
    write_index(&path, &entries)
}

pub(crate) fn remove_index(config: &Config, _hash: &str, key: &str) -> Result<(), String> {
    ensure_dir(&config.root)?;
    let path = config.root.join("index.tsv");
    let mut entries = read_index(&path)?;
    entries.retain(|(_, existing_key)| existing_key != key);
    write_index(&path, &entries)
}

pub(crate) fn read_index(path: &Path) -> Result<Vec<(String, String)>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read line {} in {}: {}",
                line_number + 1,
                path.display(),
                err
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let Some((hash, key)) = line.split_once('\t') else {
            return Err(format!(
                "malformed index line {} in {}",
                line_number + 1,
                path.display()
            ));
        };
        validate_index_entry(hash, key).map_err(|err| {
            format!(
                "malformed index line {} in {}: {}",
                line_number + 1,
                path.display(),
                err
            )
        })?;
        entries.push((hash.to_string(), key.to_string()));
    }
    Ok(entries)
}

pub(crate) fn validate_index_entry(hash: &str, key: &str) -> Result<(), String> {
    if hash.is_empty() {
        return Err("hash must not be empty".to_string());
    }
    if hash.contains('\t') || hash.contains('\n') || hash.contains('\r') {
        return Err("hash must not contain tabs or newlines".to_string());
    }
    validate_note_key(key)
}

pub(crate) fn write_index(path: &Path, entries: &[(String, String)]) -> Result<(), String> {
    let mut content = String::new();
    for (hash, key) in entries {
        validate_index_entry(hash, key)
            .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
        content.push_str(hash);
        content.push('\t');
        content.push_str(key);
        content.push('\n');
    }
    fs::write(path, content).map_err(|err| format!("failed to write {}: {}", path.display(), err))
}

pub(crate) fn unix_timestamp() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))
}

pub(crate) fn hash_key(key: &str) -> String {
    let mut hash = FNV_OFFSET;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    encode_60_bits(hash & ((1u64 << 60) - 1))
}

pub(crate) fn encode_60_bits(value: u64) -> String {
    let mut out = String::with_capacity(10);
    for shift in (0..60).step_by(6).rev() {
        let index = ((value >> shift) & 0x3f) as usize;
        out.push(B64_URL[index] as char);
    }
    out
}
