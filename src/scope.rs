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
        normalized.push(path);
    }
    if normalized.is_empty() {
        Ok(full_scope())
    } else {
        Ok(canonicalize_scope_paths(normalized))
    }
}

fn canonicalize_scope_paths(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    let mut canonical: Vec<String> = Vec::new();
    for path in paths {
        if canonical.iter().any(|parent| scope_contains(parent, &path)) {
            continue;
        }
        canonical.push(path);
    }
    if canonical.is_empty() {
        full_scope()
    } else {
        canonical
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
