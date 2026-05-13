use crate::*;

pub(crate) fn sanitize_scope(scope: &[String], agent: &AgentConfig) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope, Some(agent))
}

pub(crate) fn sanitize_scope_for_hash(scope: &[String]) -> Result<Vec<String>, String> {
    sanitize_scope_paths(scope, None)
}

fn sanitize_scope_paths(
    scope: &[String],
    denied_agent: Option<&AgentConfig>,
) -> Result<Vec<String>, String> {
    if scope.is_empty() {
        return Err("scope must not be empty".to_string());
    }
    let mut normalized = Vec::new();
    let mut has_full_scope = false;
    for path in scope {
        let path = normalize_repo_path(path)?;
        if path != "." && (path.contains('*') || path.contains('?')) {
            return Err(format!("scope paths must not be globs: {}", path));
        }
        if path != "." && denied_agent.is_some_and(|agent| is_denied_path(agent, &path)) {
            return Err(format!("scope path is denied: {}", path));
        }
        if path == "." {
            has_full_scope = true;
            continue;
        }
        normalized.push(path);
    }
    if has_full_scope || normalized.is_empty() {
        Ok(full_scope())
    } else {
        Ok(canonicalize_scope_paths(normalized))
    }
}

pub(crate) fn canonicalize_scope_paths(mut paths: Vec<String>) -> Vec<String> {
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

pub(crate) fn normalize_repo_path(value: &str) -> Result<String, String> {
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

pub(crate) fn is_denied_path(agent: &AgentConfig, path: &str) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern(path, pattern))
}

pub(crate) fn is_denied_path_bytes(agent: &AgentConfig, path: &[u8]) -> bool {
    effective_ignore_patterns(agent)
        .iter()
        .any(|pattern| path_matches_pattern_bytes(path, pattern.as_bytes()))
}

pub(crate) fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    let path = path.trim_start_matches("./");
    let pattern = pattern.trim_start_matches("./");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{}/", prefix));
    }
    path == pattern
}

fn path_matches_pattern_bytes(path: &[u8], pattern: &[u8]) -> bool {
    let path = trim_dot_slash_bytes(path);
    let pattern = trim_dot_slash_bytes(pattern);
    if let Some(prefix) = pattern.strip_suffix(b"/**") {
        return path == prefix
            || (path.len() > prefix.len()
                && path.starts_with(prefix)
                && path[prefix.len()] == b'/');
    }
    path == pattern
}

fn trim_dot_slash_bytes(mut path: &[u8]) -> &[u8] {
    while path.starts_with(b"./") {
        path = &path[2..];
    }
    path
}

pub(crate) fn is_strict_scope_subset(proposed: &[String], current: &[String]) -> bool {
    let proposed = canonicalize_scope_paths(proposed.to_vec());
    let current = canonicalize_scope_paths(current.to_vec());
    if proposed == current {
        return false;
    }
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

pub(crate) fn scope_is_within(proposed: &[String], current: &[String]) -> bool {
    proposed
        .iter()
        .all(|path| current.iter().any(|base| scope_contains(base, path)))
}

pub(crate) fn scope_contains(base: &str, path: &str) -> bool {
    base == "." || path == base || path.starts_with(&format!("{}/", base))
}

pub(crate) fn effective_ignore_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = MANDATORY_EVALUATOR_DENY_PATTERNS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    for pattern in &agent.ignore {
        if !patterns.iter().any(|existing| existing == pattern) {
            patterns.push(pattern.clone());
        }
    }
    patterns
}
pub(crate) const MANDATORY_EVALUATOR_DENY_PATTERNS: &[&str] = &[
    ".canon",
    ".canon/**",
    ".git/canon",
    ".git/canon/**",
    ".git/canon/logs",
    ".git/canon/logs/**",
];
