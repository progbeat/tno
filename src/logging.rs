use crate::fs_util::ensure_dir;
use crate::git::resolve_git_path;
use crate::repo_inspection::RepoInspectionCache;
use crate::time::{format_log_record_timestamp, unix_timestamp};
use crate::types::{CheckRecord, CheckResult, SelectedExpectation};
use crate::{DiagnosticLogConfig, DEFAULT_DIAGNOSTIC_LOG_CONFIG, GIT_CANON_LOG_DIR};
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
        rotate_diagnostic_logs_if_needed(&log_dir)?;
        let path = log_dir.join("0.jsonl");
        Ok(DiagnosticLogWriter { path, file: None })
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
            .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))
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
    rotate_diagnostic_logs_if_needed(&log_dir)?;
    let path = log_dir.join(diagnostic_log_config().files[0]);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let line = render_runtime_log_event(level, event, fields)?;
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

pub(crate) fn rotate_diagnostic_logs_if_needed(log_dir: &Path) -> Result<(), String> {
    let config = diagnostic_log_config();
    let active = log_dir.join(config.files[0]);
    let should_rotate = active
        .metadata()
        .map(|metadata| metadata.len() > config.max_bytes)
        .unwrap_or(false);
    if !should_rotate {
        return Ok(());
    }
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
    Ok(())
}

pub(crate) fn diagnostic_log_config() -> &'static DiagnosticLogConfig {
    // The public Logs spec deliberately avoids concrete retention values. The
    // configured policy lives here as implementation defaults.
    &DEFAULT_DIAGNOSTIC_LOG_CONFIG
}

pub(crate) fn render_runtime_log_event(
    level: &str,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<String, String> {
    let event = RuntimeLogEvent {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
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
