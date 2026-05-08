#[cfg(test)]
fn write_diagnostic_log(root: &Path, records: &[CheckRecord]) -> Result<PathBuf, String> {
    let mut writer = DiagnosticLogWriter::create(root)?;
    for record in records {
        writer.write_record(record)?;
    }
    let path = writer.path.clone();
    writer.finish()?;
    Ok(path)
}

struct DiagnosticLogWriter {
    path: PathBuf,
    file: Option<fs::File>,
}

impl DiagnosticLogWriter {
    fn create(root: &Path) -> Result<DiagnosticLogWriter, String> {
        let log_dir = git_path(root, GIT_CANON_LOG_DIR)?;
        ensure_dir(&log_dir)?;
        rotate_diagnostic_logs_if_needed(&log_dir)?;
        let path = log_dir.join("0.jsonl");
        Ok(DiagnosticLogWriter { path, file: None })
    }

    fn write_record(&mut self, record: &CheckRecord) -> Result<(), String> {
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
        let file = self.file.as_mut().expect("diagnostic log file is open");
        file.write_all(line.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", self.path.display(), err))?;
        file.flush()
            .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))
    }

    fn finish(mut self) -> Result<(), String> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
                .map_err(|err| format!("failed to flush {}: {}", self.path.display(), err))?;
        }
        Ok(())
    }
}

fn render_check_log_record(record: &CheckRecord) -> String {
    let mut output = String::new();
    output.push('{');
    let mut first = true;
    append_json_field(
        &mut output,
        &mut first,
        "timestamp",
        json!(record.timestamp),
    );
    append_json_field(&mut output, &mut first, "number", json!(record.number));
    append_json_field(&mut output, &mut first, "result", json!(record.result));
    append_json_field(&mut output, &mut first, "prompt", json!(record.prompt));
    append_json_field(&mut output, &mut first, "expected", json!(record.expected));
    append_json_field(&mut output, &mut first, "observed", json!(record.observed));
    append_json_field(&mut output, &mut first, "evidence", json!(record.evidence));
    append_json_field(&mut output, &mut first, "scope", json!(record.scope));
    append_json_field(
        &mut output,
        &mut first,
        "scopeHash",
        json!(record.scope_hash),
    );
    output.push_str("}\n");
    output
}

fn append_json_field(output: &mut String, first: &mut bool, key: &str, value: Value) {
    if *first {
        *first = false;
    } else {
        output.push(',');
    }
    output.push_str(&serde_json::to_string(key).expect("check log key is serializable"));
    output.push(':');
    output.push_str(&serde_json::to_string(&value).expect("check log value is serializable"));
}

fn join_numbers(numbers: &[usize]) -> String {
    numbers
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
