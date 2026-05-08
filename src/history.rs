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
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<CheckRecord>(line).map_err(|err| {
            format!(
                "failed to parse {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

fn reusable_history_record(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<CheckRecord>, String> {
    reusable_history_record_for_source(root, agent, expectation, ScopeHashSource::Index)
}

fn reusable_history_record_for_source(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    source: ScopeHashSource,
) -> Result<Option<CheckRecord>, String> {
    let records = read_history_records(root, expectation)?;
    for mut record in records.into_iter().rev() {
        if record.observed == UNPARSEABLE_OBSERVED {
            continue;
        }
        let scope = match sanitize_scope(&record.scope, agent) {
            Ok(scope) => scope,
            Err(_) => continue,
        };
        let Some(current_hash) = scope_hash_for_source(root, agent, &scope, source)? else {
            return Ok(None);
        };
        if current_hash == record.scope_hash {
            record.scope = scope;
            record.number = expectation.number;
            record.prompt = expectation.q.clone();
            record.expected = expectation.a.clone();
            return Ok(Some(record));
        }
    }
    Ok(None)
}

fn latest_history_scope(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
) -> Result<Option<Vec<String>>, String> {
    let records = read_history_records(root, expectation)?;
    for record in records.into_iter().rev() {
        if record.observed == "idk"
            || record.observed == "malformed"
            || record.observed == UNPARSEABLE_OBSERVED
        {
            continue;
        }
        if let Ok(scope) = sanitize_scope(&record.scope, agent) {
            return Ok(Some(scope));
        }
    }
    Ok(None)
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
    let mut bytes = [0_u8; 2];
    let mut file = fs::File::open("/dev/urandom")
        .map_err(|err| format!("failed to open OS random source: {}", err))?;
    file.read_exact(&mut bytes)
        .map_err(|err| format!("failed to read OS random source: {}", err))?;
    Ok(u16::from_ne_bytes(bytes) % 15 == 0)
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
