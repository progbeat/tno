use crate::fs_util::ensure_dir;
use crate::git::resolve_git_path;
use crate::project::command_output_trimmed;
use crate::repo_inspection::RepoInspectionCache;
use crate::time::{format_record_timestamp, unix_timestamp};
use crate::types::{CheckRecord, CheckResult, SelectedExpectation};
use crate::{DiagnosticLogConfig, DEFAULT_DIAGNOSTIC_LOG_CONFIG, GIT_CANON_LOG_DIR};
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const LOG_MAX_SIZE_CONFIG_KEY: &str = "canon.logs.maxSize";

#[cfg(test)]
pub(crate) fn write_diagnostic_log(
    root: &Path,
    records: &[CheckRecord],
) -> Result<PathBuf, String> {
    let mut writer = DiagnosticLogWriter::create(root)?;
    for record in records {
        writer.write_record(record)?;
    }
    let path = writer.path.clone();
    Ok(path)
}

pub(crate) struct DiagnosticLogWriter {
    pub(crate) path: PathBuf,
    log_dir: PathBuf,
    config: DiagnosticLogConfig,
    file: Option<fs::File>,
}

impl DiagnosticLogWriter {
    #[cfg(test)]
    pub(crate) fn create(root: &Path) -> Result<DiagnosticLogWriter, String> {
        let mut cache = RepoInspectionCache::new();
        DiagnosticLogWriter::create_with_cache(root, &mut cache)
    }

    pub(crate) fn create_with_cache(
        root: &Path,
        cache: &mut RepoInspectionCache,
    ) -> Result<DiagnosticLogWriter, String> {
        let log_dir = cache.git_path(root, GIT_CANON_LOG_DIR)?;
        ensure_dir(&log_dir)?;
        let config = diagnostic_log_config(root)?;
        rotate_diagnostic_logs_with_config(&log_dir, &config)?;
        let path = log_dir.join("0.jsonl");
        Ok(DiagnosticLogWriter {
            path,
            log_dir,
            config,
            file: None,
        })
    }

    pub(crate) fn write_record(&mut self, record: &CheckRecord) -> Result<(), String> {
        self.write_record_event("expectation.result", record)
    }

    pub(crate) fn write_interrogation_record(
        &mut self,
        record: &CheckRecord,
    ) -> Result<(), String> {
        self.write_record_event("interrogation.result", record)
    }

    fn write_record_event(&mut self, event: &str, record: &CheckRecord) -> Result<(), String> {
        self.write_event(
            "info",
            event,
            &[
                ("id", json!(record.id)),
                ("result", json!(record.result)),
                ("observed", json!(record.observed)),
                ("evidence", json!(record.evidence)),
                ("scope", json!(record.scope)),
                ("scopeHash", json!(record.scope_hash)),
                ("prompt", json!(record.prompt)),
                ("expected", json!(record.expected)),
            ],
        )
    }

    pub(crate) fn write_event(
        &mut self,
        level: &str,
        event: &str,
        fields: &[(&str, Value)],
    ) -> Result<(), String> {
        if self.file.is_none() {
            self.file = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .map_err(|err| format!("failed to open {}: {}", self.path.display(), err))?,
            );
        }
        let line = render_runtime_log_event(level, event, fields)?;
        let Some(file) = self.file.as_mut() else {
            return Err("diagnostic log file is not open".to_string());
        };
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", self.path.display(), err))?;
        file.flush()
            .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))?;
        self.file = None;
        prune_diagnostic_logs_to_limit(&self.log_dir, &self.config)
    }
}

pub(crate) fn append_runtime_log_event(
    root: &Path,
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), String> {
    let log_dir = resolve_git_path(root, GIT_CANON_LOG_DIR)?;
    ensure_dir(&log_dir)?;
    let config = diagnostic_log_config(root)?;
    rotate_diagnostic_logs_with_config(&log_dir, &config)?;
    let path = log_dir.join(config.files[0]);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let line = render_runtime_log_event(level, event, fields)?;
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))?;
    drop(file);
    prune_diagnostic_logs_to_limit(&log_dir, &config)
}

fn rotate_diagnostic_logs_with_config(
    log_dir: &Path,
    config: &DiagnosticLogConfig,
) -> Result<(), String> {
    let active = log_dir.join(config.files[0]);
    let active_limit = active_log_max_bytes(config);
    let should_rotate = active
        .metadata()
        .map(|metadata| metadata.len() > active_limit)
        .unwrap_or(false);
    if should_rotate {
        let oldest = log_dir.join(config.files[config.files.len() - 1]);
        if oldest.exists() {
            fs::remove_file(&oldest)
                .map_err(|err| format!("failed to remove {}: {}", oldest.display(), err))?;
        }
        for index in (0..config.files.len() - 1).rev() {
            let from = log_dir.join(config.files[index]);
            if from.exists() {
                let to = log_dir.join(config.files[index + 1]);
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
    }
    prune_diagnostic_logs_to_limit(log_dir, config)?;
    Ok(())
}

pub(crate) fn diagnostic_log_config(root: &Path) -> Result<DiagnosticLogConfig, String> {
    Ok(DiagnosticLogConfig {
        max_bytes: configured_log_max_size(root)?,
        files: DEFAULT_DIAGNOSTIC_LOG_CONFIG.files,
    })
}

fn configured_log_max_size(root: &Path) -> Result<u64, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--get")
        .arg(LOG_MAX_SIZE_CONFIG_KEY)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    let stdout = command_output_trimmed(&output.stdout, "git config stdout")?;
    let stderr = command_output_trimmed(&output.stderr, "git config stderr")?;
    if output.status.success() {
        return parse_log_max_size(stdout);
    }
    if stdout.is_empty() && stderr.is_empty() {
        return Ok(DEFAULT_DIAGNOSTIC_LOG_CONFIG.max_bytes);
    }
    Err(format!(
        "failed to read git config {}: {}",
        LOG_MAX_SIZE_CONFIG_KEY, stderr
    ))
}

fn parse_log_max_size(value: &str) -> Result<u64, String> {
    if value.is_empty() {
        return Err(format!("{} must not be empty", LOG_MAX_SIZE_CONFIG_KEY));
    }
    let (digits, multiplier) = match value.as_bytes().last().copied() {
        Some(b'M') => (&value[..value.len() - 1], 1024 * 1024),
        Some(b'G') => (&value[..value.len() - 1], 1024 * 1024 * 1024),
        Some(byte) if byte.is_ascii_digit() => (value, 1),
        _ => {
            return Err(format!(
                "{} must be a byte count with optional M or G suffix",
                LOG_MAX_SIZE_CONFIG_KEY
            ));
        }
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "{} must be a byte count with optional M or G suffix",
            LOG_MAX_SIZE_CONFIG_KEY
        ));
    }
    digits
        .parse::<u64>()
        .map_err(|_| format!("{} value is too large", LOG_MAX_SIZE_CONFIG_KEY))?
        .checked_mul(multiplier)
        .ok_or_else(|| format!("{} value is too large", LOG_MAX_SIZE_CONFIG_KEY))
}

fn active_log_max_bytes(config: &DiagnosticLogConfig) -> u64 {
    (config.max_bytes / config.files.len() as u64).max(1)
}

fn prune_diagnostic_logs_to_limit(
    log_dir: &Path,
    config: &DiagnosticLogConfig,
) -> Result<(), String> {
    for index in (1..config.files.len()).rev() {
        if diagnostic_log_dir_size(log_dir, config)? <= config.max_bytes {
            return Ok(());
        }
        let path = log_dir.join(config.files[index]);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| format!("failed to remove {}: {}", path.display(), err))?;
        }
    }
    if diagnostic_log_dir_size(log_dir, config)? > config.max_bytes {
        let active = log_dir.join(config.files[0]);
        if active.exists() {
            fs::remove_file(&active)
                .map_err(|err| format!("failed to remove {}: {}", active.display(), err))?;
        }
    }
    Ok(())
}

fn diagnostic_log_dir_size(log_dir: &Path, config: &DiagnosticLogConfig) -> Result<u64, String> {
    let mut total = 0u64;
    for file_name in config.files {
        let path = log_dir.join(file_name);
        if !path.exists() {
            continue;
        }
        let size = path
            .metadata()
            .map_err(|err| format!("failed to stat {}: {}", path.display(), err))?
            .len();
        total = total
            .checked_add(size)
            .ok_or_else(|| format!("{} size is too large", log_dir.display()))?;
    }
    Ok(total)
}

pub(crate) fn render_runtime_log_event(
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<String, String> {
    let event = RuntimeLogEvent {
        timestamp: format_record_timestamp(unix_timestamp()?),
        level,
        event,
        extra: fields,
    };
    let mut output = serde_json::to_string(&event)
        .map_err(|err| format!("failed to serialize runtime log event: {}", err))?;
    output.push('\n');
    Ok(output)
}

pub(crate) fn render_check_log_record(record: &CheckRecord) -> String {
    // History records intentionally start with the cache.md required field
    // prefix. Extra persisted metadata follows it; expectation references use
    // the resolved full ID, never the display/selector prefix.
    let history = HistoryLogRecord {
        timestamp: &record.timestamp,
        result: record.result,
        observed: &record.observed,
        evidence: &record.evidence,
        scope: &record.scope,
        scope_hash: &record.scope_hash,
        id: &record.id,
        prompt: &record.prompt,
        expected: &record.expected,
        cache_key: record.cache_key.as_deref(),
    };
    let mut output =
        serde_json::to_string(&history).expect("serializing a history log record cannot fail");
    output.push('\n');
    output
}

struct RuntimeLogEvent<'a> {
    timestamp: String,
    level: &'a str,
    event: &'a str,
    extra: &'a [(&'a str, Value)],
}

impl Serialize for RuntimeLogEvent<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(3 + self.extra.len()))?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.serialize_entry("level", self.level)?;
        map.serialize_entry("event", self.event)?;
        for (key, value) in self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct HistoryLogRecord<'a> {
    timestamp: &'a str,
    result: CheckResult,
    observed: &'a str,
    evidence: &'a str,
    scope: &'a [String],
    #[serde(rename = "scopeHash")]
    scope_hash: &'a str,
    id: &'a str,
    prompt: &'a str,
    expected: &'a str,
    #[serde(rename = "cacheKey", skip_serializing_if = "Option::is_none")]
    cache_key: Option<&'a str>,
}

pub(crate) fn append_json_string_array(output: &mut String, values: &[String]) {
    output.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        push_json_string(output, value);
    }
    output.push(']');
}

pub(crate) fn push_json_string(output: &mut String, value: &str) {
    output.push_str(&serde_json::to_string(value).expect("serializing a JSON string cannot fail"));
}

pub(crate) fn push_json_control_escape(output: &mut String, ch: char) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let code = ch as usize;
    output.push_str("\\u00");
    output.push(HEX[(code >> 4) & 0x0f] as char);
    output.push(HEX[code & 0x0f] as char);
}

pub(crate) fn join_display_ids(expectations: &[SelectedExpectation]) -> String {
    expectations
        .iter()
        .map(|expectation| expectation.display_id.clone())
        .collect::<Vec<_>>()
        .join(", ")
}
