use crate::check::CheckRunCaches;
use crate::check_lazy_reset::apply_lazy_full_scope_reset_or_warn;
use crate::check_reporting::{write_check_finish_report_event, write_check_token_usage_event};
use crate::check_selection::ExpectationIdentity;
use crate::check_types::{CheckRecord, CheckRunReport, SelectedExpectation};
use crate::cli::CommandError;
use crate::config_types::{AgentConfig, CheckConfig};
use crate::gate::{
    exact_gate_cache_result_for_tree, gate_would_pass_with_config, GateCacheResult,
    GateComparisonTree,
};
use crate::history::HistoryCache;
use crate::scope_hash::ScopeHashCache;
use crate::token_usage_types::TokenUsage;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

// Success and error reports share cleanup, finish logging, and the post-summary
// improvement notice. The optional error only changes the finish log payload
// and final command result.
pub(crate) struct CheckReportFinishContext<'a, 'b> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a CheckConfig,
    pub(crate) identities: &'a [ExpectationIdentity],
    pub(crate) diagnostic_log: &'b mut crate::logging::DiagnosticLogWriter,
    pub(crate) result_output: &'b mut dyn Write,
    pub(crate) check_caches: &'b mut CheckRunCaches,
}

pub(crate) fn finish_check_report(
    context: CheckReportFinishContext<'_, '_>,
    usage: TokenUsage,
    report: &CheckRunReport,
    error: Option<&str>,
) -> Result<(), CommandError> {
    // Per-expectation output, the public token-usage line, and the public
    // summary are written and flushed before this finish step is called. This
    // function only handles pieces that become computable after those steps:
    // the optional pass-improvement notice and structured finish log records.
    write_pass_improvement_notice_if_needed(
        context.root,
        context.config,
        context.identities,
        report,
        context.result_output,
        context.check_caches,
    )?;
    write_check_token_usage_event(context.diagnostic_log, usage)?;
    apply_lazy_full_scope_reset_or_warn(
        context.root,
        context.config,
        usage,
        &report.non_selected,
        context.diagnostic_log,
    );
    write_check_finish_report_event(context.diagnostic_log, false, report, error)?;
    Ok(())
}

pub(crate) fn pass_improvement_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("▷ +1 pass compared to HEAD. Commit staged changes!".to_string()),
        count => Some(format!(
            "▷ +{} passes compared to HEAD. Commit staged changes!",
            count
        )),
    }
}

#[cfg(test)]
pub(crate) fn staged_passes_not_pass_at_head_count(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
) -> Result<usize, String> {
    let mut history_cache = HistoryCache::new();
    let mut scope_hash_cache = ScopeHashCache::new();
    staged_passes_not_pass_at_head_count_with_cache(
        root,
        agent,
        report,
        &mut history_cache,
        &mut scope_hash_cache,
    )
}

fn staged_passes_not_pass_at_head_count_with_cache(
    root: &Path,
    agent: &AgentConfig,
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for record in report.records.iter().filter(|record| record.passed()) {
        let Some(expectation) = selected_expectation_from_record(record) else {
            continue;
        };
        match exact_gate_cache_result_for_tree(
            root,
            agent,
            &expectation,
            GateComparisonTree::Head,
            history_cache,
            scope_hash_cache,
        )? {
            GateCacheResult::Pass => {}
            GateCacheResult::Fail(_) | GateCacheResult::Missing => count += 1,
        }
    }
    Ok(count)
}

fn write_pass_improvement_notice_if_needed(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    report: &CheckRunReport,
    output: &mut dyn Write,
    caches: &mut CheckRunCaches,
) -> Result<(), CommandError> {
    // The notice is knowable only after simulating `canon gate`, because the
    // public output contract suppresses it unless the staged tree would pass.
    // Once the count is known, write and flush the public line before returning
    // to any internal finish work.
    let count = staged_pass_notice_count_if_gate_passes(
        root,
        config,
        identities,
        report,
        &mut caches.history,
        &mut caches.scope_hash,
    )?;
    if let Some(notice) = pass_improvement_notice(count) {
        writeln!(output, "{}", notice)
            .and_then(|()| output.flush())
            .map_err(|err| format!("failed to write check result to stdout: {}", err))?;
    }
    Ok(())
}

pub(crate) fn staged_pass_notice_count_if_gate_passes(
    root: &Path,
    config: &CheckConfig,
    identities: &[ExpectationIdentity],
    report: &CheckRunReport,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<usize, String> {
    let gate_args: &[OsString] = &[];
    if !gate_would_pass_with_config(
        root,
        config,
        identities,
        gate_args,
        history_cache,
        scope_hash_cache,
    )? {
        return Ok(0);
    }
    staged_passes_not_pass_at_head_count_with_cache(
        root,
        &config.agent,
        report,
        history_cache,
        scope_hash_cache,
    )
}

fn selected_expectation_from_record(record: &CheckRecord) -> Option<SelectedExpectation> {
    Some(SelectedExpectation {
        number: record.number,
        id: record.id.clone(),
        display_id: record.display_id.clone(),
        q: record.prompt.clone()?,
        a: record.expected.clone()?,
        cooldown: None,
        thinking: None,
    })
}
