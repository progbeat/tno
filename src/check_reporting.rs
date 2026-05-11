use crate::*;

pub(crate) fn collect_check_token_usage(
    runner: &mut LazyAppServerRunner,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<TokenUsage, String> {
    runner.drain_token_usage_updates();
    let usage = runner.token_usage().unwrap_or_default();
    diagnostic_log.write_event("info", "token.usage", &token_usage_log_fields(usage))?;
    Ok(usage)
}

pub(crate) fn collect_and_print_check_token_usage(
    runner: &mut LazyAppServerRunner,
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<TokenUsage, String> {
    let usage = collect_check_token_usage(runner, diagnostic_log)?;
    print_token_usage_summary(Some(usage));
    Ok(usage)
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
            skipped: report.skipped,
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
    diagnostic_log.write_event("info", "check.finish", &fields)
}

pub(crate) fn print_token_usage_summary(usage: Option<TokenUsage>) {
    // This stderr line is part of the public check-output contract; the same
    // usage data is written as structured runtime-log data before this prints.
    eprintln!("{}", render_token_usage_summary(usage.unwrap_or_default()));
}
