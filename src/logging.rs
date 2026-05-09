use crate::*;

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
        if self.file.is_none() {
            self.file = Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .map_err(|err| format!("failed to open {}: {}", self.path.display(), err))?,
            );
        }
        let line = render_check_log_record(record);
        let Some(file) = self.file.as_mut() else {
            return Err("diagnostic log file is not open".to_string());
        };
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", self.path.display(), err))?;
        file.flush()
            .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))
    }
}

pub(crate) fn render_check_log_record(record: &CheckRecord) -> String {
    let mut output = String::new();
    output.push('{');
    let mut first = true;
    append_json_string_field(&mut output, &mut first, "timestamp", &record.timestamp);
    append_json_usize_field(&mut output, &mut first, "number", record.number);
    append_json_string_field(&mut output, &mut first, "result", &record.result);
    append_json_string_field(&mut output, &mut first, "prompt", &record.prompt);
    append_json_string_field(&mut output, &mut first, "expected", &record.expected);
    append_json_string_field(&mut output, &mut first, "observed", &record.observed);
    append_json_string_field(&mut output, &mut first, "evidence", &record.evidence);
    append_json_string_array_field(&mut output, &mut first, "scope", &record.scope);
    append_json_string_field(&mut output, &mut first, "scopeHash", &record.scope_hash);
    output.push_str("}\n");
    output
}

pub(crate) fn append_json_separator(output: &mut String, first: &mut bool) {
    if *first {
        *first = false;
    } else {
        output.push(',');
    }
}

pub(crate) fn append_json_string_field(
    output: &mut String,
    first: &mut bool,
    key: &str,
    value: &str,
) {
    append_json_separator(output, first);
    push_json_string(output, key);
    output.push(':');
    push_json_string(output, value);
}

pub(crate) fn append_json_usize_field(
    output: &mut String,
    first: &mut bool,
    key: &str,
    value: usize,
) {
    append_json_separator(output, first);
    push_json_string(output, key);
    output.push(':');
    output.push_str(&value.to_string());
}

pub(crate) fn append_json_string_array_field(
    output: &mut String,
    first: &mut bool,
    key: &str,
    values: &[String],
) {
    append_json_separator(output, first);
    push_json_string(output, key);
    output.push(':');
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
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            ch if ch <= '\u{1f}' => push_json_control_escape(output, ch),
            ch => output.push(ch),
        }
    }
    output.push('"');
}

pub(crate) fn push_json_control_escape(output: &mut String, ch: char) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let code = ch as usize;
    output.push_str("\\u00");
    output.push(HEX[(code >> 4) & 0x0f] as char);
    output.push(HEX[code & 0x0f] as char);
}

pub(crate) fn join_numbers(numbers: &[usize]) -> String {
    numbers
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
