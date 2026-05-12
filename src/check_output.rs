use crate::*;

pub(crate) fn write_and_flush_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        let line = render_check_output_record(record);
        writer
            .write_all(line.as_bytes())
            .map_err(|err| format!("failed to write check result to stdout: {}", err))?;
        writer
            .flush()
            .map_err(|err| format!("failed to flush check result to stdout: {}", err))?;
    }
    Ok(())
}

pub(crate) fn write_summary_line(
    result_output: &mut dyn Write,
    report: &CheckRunReport,
    elapsed: Duration,
) -> Result<(), String> {
    let line = render_check_summary(report, elapsed);
    result_output
        .write_all(line.as_bytes())
        .map_err(|err| format!("failed to write check summary to stdout: {}", err))?;
    result_output
        .flush()
        .map_err(|err| format!("failed to flush check summary to stdout: {}", err))
}

pub(crate) fn write_query_output(
    result_output: &mut dyn Write,
    answer: &ParsedAnswer,
) -> Result<(), String> {
    // Query output is intentionally separate from the selected-expectation
    // check output contract because query mode has no expectation number,
    // expected answer, reusable history write, or final check summary.
    let output = render_query_output(answer);
    result_output
        .write_all(output.as_bytes())
        .map_err(|err| format!("failed to write query result to stdout: {}", err))?;
    result_output
        .flush()
        .map_err(|err| format!("failed to flush query result to stdout: {}", err))
}

pub(crate) fn render_query_output(answer: &ParsedAnswer) -> String {
    let mut output = String::new();
    output.push_str("Observed: ");
    output.push_str(&escape_check_output_text(&answer.answer));
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&answer.evidence));
    output.push('\n');
    output.push_str("Scope: ");
    output.push_str(&compact_json_string_array(&answer.scope));
    output.push('\n');
    output
}

pub(crate) fn render_check_output_record(record: &CheckRecord) -> String {
    // Check-output line counts are part of the public contract:
    // pass => 1 line, failed => 6 lines including Scope, error => 5 lines
    // without Scope. Every evaluator-supplied text field is escaped here
    // before stdout sees it.
    if record.passed() {
        return format!("{}. OK\n", record.number);
    }
    let status = if record_requires_human_review(record) {
        "ERROR"
    } else {
        "FAILED"
    };
    let mut output = String::new();
    output.push_str(&format!("{}. {}\n", record.number, status));
    // This is the spec's `<escaped question>` line, not an extra line beyond
    // the six-line failed and five-line error layouts.
    output.push_str(&escape_check_output_text(&record.prompt));
    output.push('\n');
    output.push_str("Expected: ");
    output.push_str(&escape_check_output_text(&record.expected));
    output.push('\n');
    output.push_str("Observed: ");
    output.push_str(&escape_check_output_text(&record.observed));
    output.push('\n');
    output.push_str("Evidence: ");
    output.push_str(&escape_check_output_text(&record.evidence));
    output.push('\n');
    if status == "FAILED" {
        output.push_str("Scope: ");
        output.push_str(&compact_json_string_array(&record.scope));
        output.push('\n');
    }
    output
}

pub(crate) fn render_check_summary(report: &CheckRunReport, elapsed: Duration) -> String {
    // Summary order is fixed to match the spec and pytest-style labels:
    // failed, error/errors, passed, skipped.
    // `report.skipped` is the selection complement, so
    // `report.selected + report.skipped` covers the active check configuration.
    // The public summary's skipped label is this same non-selected count.
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut errors = 0usize;
    for record in &report.records {
        if record.passed() {
            passed += 1;
        } else if record_requires_human_review(record) {
            errors += 1;
        } else {
            failed += 1;
        }
    }
    let mut outcomes = Vec::new();
    if failed > 0 {
        outcomes.push(format!("{} failed", failed));
    }
    if errors > 0 {
        outcomes.push(format!(
            "{} {}",
            errors,
            if errors == 1 { "error" } else { "errors" }
        ));
    }
    if passed > 0 {
        outcomes.push(format!("{} passed", passed));
    }
    let skipped = report_output_skipped_count(report);
    if skipped > 0 {
        outcomes.push(format!("{} skipped", skipped));
    }
    if outcomes.is_empty() {
        outcomes.push("0 passed".to_string());
    }
    let inner = format!(" {} in {:.2}s ", outcomes.join(", "), elapsed.as_secs_f64());
    format!("{}\n", pad_summary_line(&inner))
}

pub(crate) fn pad_summary_line(inner: &str) -> String {
    const WIDTH: usize = 80;
    if inner.len() >= WIDTH {
        return format!("={inner}=");
    }
    let padding = WIDTH - inner.len();
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", "=".repeat(left), inner, "=".repeat(right))
}

pub(crate) fn record_requires_human_review(record: &CheckRecord) -> bool {
    // Restricted-scope `idk` records are retried at full scope before output.
    // Any `idk` that reaches final check rendering is therefore the
    // human-review state described by the check-output contract.
    record.observed == OBSERVED_MALFORMED
        || record.observed == UNPARSEABLE_OBSERVED
        || record.observed == EMPTY_EVIDENCE_OBSERVED
        || record.observed == OBSERVED_IDK
}

pub(crate) fn compact_json_string_array(values: &[String]) -> String {
    let mut output = String::new();
    append_json_string_array(&mut output, values);
    output
}

pub(crate) fn escape_check_output_text(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => {
                let mut escaped = String::new();
                push_json_control_escape(&mut escaped, ch);
                output.push_str(&escaped);
            }
            ch => output.push(ch),
        }
    }
    output
}
