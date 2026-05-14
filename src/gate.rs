use crate::check_cache::final_selected_after_current_pass_cache;
use crate::check_preflight::{
    is_canon_only_staged_change_bytes, is_canon_project_path_bytes, staged_changed_path_bytes,
};
use crate::check_selection::{final_selected_expectations, select_expectations};
use crate::cli::CommandError;
use crate::history::HistoryCache;
use crate::history_reuse::reusable_history_record_for_source;
use crate::logging::{join_display_ids, render_check_log_record};
use crate::output::{write_stderr, write_stderr_line};
use crate::repo_inspection::RepoInspectionCache;
use crate::scope_hash::{ScopeHashCache, ScopeHashSource};
use crate::time::unix_timestamp;
use crate::types::{AgentConfig, CheckConfig, CheckRecord, SelectedExpectation};
use crate::CHECK_PATH;
use std::ffi::OsString;
use std::path::Path;

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), CommandError> {
    // CLI validation happens before the gate pass/fail decision. These
    // unsupported-option errors are usage errors, not `GateFailed` outcomes.
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".into());
    }
    let changed_paths = staged_changed_path_bytes(root)?;
    let has_canon_change = changed_paths
        .iter()
        .any(|path| is_canon_project_path_bytes(path));
    if has_canon_change && !is_canon_only_staged_change_bytes(&changed_paths) {
        write_stderr_line(
            "canon gate: .canon/** changes must not be mixed with non-.canon changes",
        )?;
        return Err(CommandError::GateFailed);
    }
    if has_canon_change {
        return Ok(());
    }

    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH))?;
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    let now = unix_timestamp()?;
    // The gate spec's `selected_expectations` parameter is not raw CLI selector
    // expansion. The CLI first builds the same command-final selected set as
    // `canon check`: selector expansion, cooldown removal, and current
    // passing-cache deselection. The loop below is only gate's HEAD-vs-index
    // decision over that already-final set.
    let selected_expectations = select_expectations_for_gate(
        root,
        &config,
        args,
        &mut history_cache,
        &mut scope_hash_cache,
        now,
    )?;
    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected_expectations {
        let previous = exact_gate_cache_result_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Head,
            &mut history_cache,
            &mut scope_hash_cache,
        )?;
        let current = exact_gate_cache_result_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Index,
            &mut history_cache,
            &mut scope_hash_cache,
        )?;
        match current {
            GateCacheResult::Fail(_) if previous.is_fail() => {}
            GateCacheResult::Fail(record) => failing.push(*record),
            GateCacheResult::Missing => missing.push(expectation.clone()),
            GateCacheResult::Pass => {}
        }
    }

    if missing.is_empty() && failing.is_empty() {
        return Ok(());
    }
    if !failing.is_empty() {
        write_stderr_line("canon gate: expectations regressed to cached fail:")?;
        for record in &failing {
            write_stderr(&render_check_log_record(record))?;
        }
    }
    if !missing.is_empty() {
        write_stderr_line(&format!(
            "canon gate: missing cached answers for expectations: {}",
            join_display_ids(&missing)
        ))?;
        if let Some(advice) = gate_missing_cache_advice(!failing.is_empty()) {
            write_stderr_line(advice)?;
        }
    }
    Err(CommandError::GateFailed)
}

pub(crate) fn gate_missing_cache_advice(has_regressions: bool) -> Option<&'static str> {
    // Regressions are the blocking action. When regressions and missing cache
    // records coexist, do not spend tokens filling unrelated missing records.
    if has_regressions {
        Some("canon gate: fix staged regressions before filling missing cache")
    } else {
        Some("canon gate: run `canon check` before committing")
    }
}

pub(crate) fn select_expectations_for_gate(
    root: &Path,
    config: &CheckConfig,
    args: &[OsString],
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
    now: u64,
) -> Result<Vec<SelectedExpectation>, String> {
    let selected = select_expectations(config, args)?;
    let selected = final_selected_expectations(root, &config.agent, selected, history_cache, now)
        .map(|selection| selection.selected)
        .map_err(|err| err.error)?;
    final_selected_after_current_pass_cache(
        root,
        &config.agent,
        selected,
        history_cache,
        scope_hash_cache,
    )
    .map(|selection| selection.selected)
}

#[derive(Debug, Clone)]
pub(crate) enum GateCacheResult {
    Pass,
    Fail(Box<CheckRecord>),
    Missing,
}

impl GateCacheResult {
    pub(crate) fn is_fail(&self) -> bool {
        matches!(self, GateCacheResult::Fail(_))
    }
}

pub(crate) fn exact_gate_cache_result_for_source(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    source: ScopeHashSource,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<GateCacheResult, String> {
    match reusable_history_record_for_source(
        root,
        agent,
        expectation,
        source,
        history_cache,
        scope_hash_cache,
    )? {
        Some(record) if record.passed() => Ok(GateCacheResult::Pass),
        Some(record) => Ok(GateCacheResult::Fail(Box::new(record))),
        None => Ok(GateCacheResult::Missing),
    }
}
