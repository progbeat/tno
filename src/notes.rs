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
