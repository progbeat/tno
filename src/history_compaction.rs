use crate::*;

pub(crate) fn should_compact_history(path: &Path) -> Result<bool, String> {
    sample_approximately_one_in(HISTORY_COMPACT_SAMPLE_INTERVAL, "history-compact", path)
}

pub(crate) fn sample_approximately_one_in(
    interval: u64,
    label: &str,
    path: &Path,
) -> Result<bool, String> {
    if interval == 0 {
        return Err("sample interval must be greater than zero".to_string());
    }
    let counter = COMPACTION_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system clock is before UNIX_EPOCH: {}", err))?
        .as_nanos();
    let seed = format!(
        "{}\0{}\0{}\0{}\0{}",
        label,
        process::id(),
        counter,
        now,
        path.display()
    );
    Ok(
        fnv64_with_seed(FNV_OFFSET ^ 0xa24b_aed4_963e_e407, seed.as_bytes())
            .is_multiple_of(interval),
    )
}

pub(crate) fn compact_history(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let mut total_lines = 0usize;
    let mut lines = std::collections::VecDeque::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<CheckRecord>(&line).map_err(|err| {
            format!(
                "invalid history JSON in {} line {}: {}",
                path.display(),
                index + 1,
                err
            )
        })?;
        total_lines += 1;
        lines.push_back(line);
        if lines.len() > HISTORY_COMPACT_KEEP_RECORDS {
            lines.pop_front();
        }
    }
    if total_lines <= HISTORY_COMPACT_KEEP_RECORDS {
        return Ok(());
    }
    let temp_path = compact_history_temp_path(path)?;
    let mut file = fs::File::create(&temp_path)
        .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    for line in lines {
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        file.write_all(b"\n")
            .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
    }
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", temp_path.display(), err))?;
    drop(file);
    replace_history_file(&temp_path, path)
}

pub(crate) fn replace_history_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path).map_err(|err| {
        format!(
            "failed to replace {} with {}: {}",
            path.display(),
            temp_path.display(),
            err
        )
    })
}

pub(crate) fn compact_history_temp_path(path: &Path) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("history path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(".tmp");
    Ok(path.with_file_name(temp_name))
}
