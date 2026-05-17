use crate::app_server::LazyAppServerRunner;
use crate::app_server_protocol::render_token_usage_summary;
use crate::check_result::{
    report_error_count, report_failed_count, report_output_skipped_count, report_passed_count,
};
use crate::check_types::{CheckRunReport, NarrowingStats};
use crate::evaluator_turn::token_usage_log_fields;
use crate::logging::DiagnosticLogWriter;
use crate::output::write_stderr_line;
use crate::token_usage_types::TokenUsage;
use serde_json::json;

pub(crate) fn collect_check_token_usage(
    runner: &mut LazyAppServerRunner,
) -> Result<TokenUsage, String> {
    runner.drain_token_usage_updates();
    Ok(runner.token_usage().unwrap_or_default())
}

pub(crate) fn write_check_token_usage_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    usage: TokenUsage,
) -> Result<(), String> {
    diagnostic_log
        .write_event("info", "token.usage", &token_usage_log_fields(usage))
        .map_err(|err| err.to_string())?;
    Ok(())
}

pub(crate) fn write_check_finish_report_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), String> {
    write_check_finish_event(
        diagnostic_log,
        query,
        CheckFinishStats {
            passed: report_passed_count(report),
            failed: report_failed_count(report),
            errors: report_error_count(report),
            skipped: report_output_skipped_count(report),
            narrowing: report.narrowing,
        },
        error,
    )
}

#[derive(Clone, Copy, Default)]
pub(crate) struct CheckFinishStats {
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) errors: usize,
    pub(crate) skipped: usize,
    pub(crate) narrowing: NarrowingStats,
}

pub(crate) fn write_check_finish_event(
    diagnostic_log: &mut DiagnosticLogWriter,
    query: bool,
    stats: CheckFinishStats,
    error: Option<&str>,
) -> Result<(), String> {
    let mut fields = Vec::new();
    if query {
        fields.push(("query", json!(true)));
    }
    fields.extend([
        ("passed", json!(stats.passed)),
        ("failed", json!(stats.failed)),
        ("errors", json!(stats.errors)),
        ("skipped", json!(stats.skipped)),
        ("narrowingAttempted", json!(stats.narrowing.attempted)),
        ("narrowingAccepted", json!(stats.narrowing.accepted)),
        ("narrowingRejected", json!(stats.narrowing.rejected)),
    ]);
    if let Some(error) = error {
        fields.push(("error", json!(error)));
    }
    diagnostic_log
        .write_event("info", "check.finish", &fields)
        .map_err(|err| err.to_string())
}

pub(crate) fn print_token_usage_summary(usage: Option<TokenUsage>) -> Result<(), String> {
    // This stderr line is part of the public check-output contract; the same
    // usage data is written as structured runtime-log data before this prints.
    write_stderr_line(&render_token_usage_summary(usage.unwrap_or_default()))
}
