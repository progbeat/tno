use crate::check_types::{CheckRecord, ObservedAnswerState, SelectedExpectation};
use crate::config_types::{AgentConfig, CheckConfig};
use crate::fs_util::write_temp_file_then_replace;
use crate::git::{read_git_blobs, staged_tracked_files};
use crate::hash::full_scope;
use crate::history::{read_history_records_from_path, HistoryCache};
use crate::history_compaction::compact_history_temp_path;
use crate::history_reuse::latest_history_scope_with_cache;
use crate::logging::render_check_log_record;
use crate::logging::DiagnosticLogWriter;
use crate::scope::{is_denied_path_bytes, sanitize_scope_for_hash};
use crate::token_usage_types::TokenUsage;
use serde_json::json;
use std::io::Write;
use std::path::Path;

pub(crate) fn apply_lazy_full_scope_reset(
    root: &Path,
    config: &CheckConfig,
    usage: TokenUsage,
    non_selected: &[SelectedExpectation],
    diagnostic_log: &mut DiagnosticLogWriter,
) -> Result<(), String> {
    let reset = plan_lazy_full_scope_reset(
        root,
        &config.agent,
        usage.total_tokens,
        non_selected,
        random_reset_seed(),
    )?;
    diagnostic_log
        .write_event(
            "info",
            "lazy_full_scope_reset",
            &[
                ("projectSizeTokens", json!(reset.project_size_tokens)),
                ("candidates", json!(non_selected.len())),
                ("reset", json!(reset.expectations.len())),
                (
                    "ids",
                    json!(reset
                        .expectations
                        .iter()
                        .map(|expectation| expectation.id.clone())
                        .collect::<Vec<_>>()),
                ),
            ],
        )
        .map_err(|err| err.to_string())?;
    if let Err(error) = reset_non_selected_expectation_histories(root, &reset.expectations) {
        diagnostic_log
            .write_event(
                "error",
                "lazy_full_scope_reset.error",
                &[("message", json!(error.clone()))],
            )
            .map_err(|err| err.to_string())?;
        return Err(error);
    }
    Ok(())
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) project_size_tokens: u64,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

#[derive(Clone)]
struct ScopedNonSelectedExpectation {
    expectation: SelectedExpectation,
    scope: Vec<String>,
}

pub(crate) fn plan_lazy_full_scope_reset(
    root: &Path,
    agent: &AgentConfig,
    total_tokens: u64,
    non_selected: &[SelectedExpectation],
    seed: u64,
) -> Result<LazyFullScopeResetPlan, String> {
    let project_size_tokens = estimate_staged_project_size_tokens(root, agent)?;
    let scoped_non_selected =
        non_selected_expectations_with_current_scope(root, agent, non_selected)?;
    let candidates = lazy_full_scope_reset_candidates(&scoped_non_selected);
    let reset_count =
        lazy_full_scope_reset_count(total_tokens, project_size_tokens, seed, candidates.len());
    Ok(LazyFullScopeResetPlan {
        project_size_tokens,
        expectations: sample_reset_expectations(&candidates, reset_count, seed),
    })
}

fn non_selected_expectations_with_current_scope(
    root: &Path,
    agent: &AgentConfig,
    non_selected: &[SelectedExpectation],
) -> Result<Vec<ScopedNonSelectedExpectation>, String> {
    let mut history_cache = HistoryCache::new();
    let mut scoped = Vec::new();
    for expectation in non_selected {
        let scope = latest_history_scope_with_cache(root, agent, expectation, &mut history_cache)?
            .unwrap_or_else(full_scope);
        scoped.push(ScopedNonSelectedExpectation {
            expectation: expectation.clone(),
            scope,
        });
    }
    Ok(scoped)
}

fn lazy_full_scope_reset_candidates(
    non_selected: &[ScopedNonSelectedExpectation],
) -> Vec<SelectedExpectation> {
    non_selected
        .iter()
        .filter(|expectation| expectation.scope != full_scope())
        .map(|expectation| expectation.expectation.clone())
        .collect()
}

pub(crate) fn estimate_staged_project_size_tokens(
    root: &Path,
    agent: &AgentConfig,
) -> Result<u64, String> {
    let staged_files = staged_tracked_files(root)?
        .into_iter()
        .filter(|file| !is_denied_path_bytes(agent, &file.path))
        .collect::<Vec<_>>();
    let object_ids = staged_files
        .iter()
        .map(|file| file.object_id.clone())
        .collect::<Vec<_>>();
    // Batch all staged blob reads through one subprocess. The project-size
    // estimate may scan many staged files, but `canon check` must not spawn a
    // direct `git` subprocess per file.
    let contents = read_git_blobs(root, &object_ids)?;
    let mut tokens = 0u64;
    for content in contents {
        // `project_size_tokens` covers staged text content; binary blobs do
        // not contribute to the text-token estimate.
        let Ok(text) = std::str::from_utf8(&content) else {
            continue;
        };
        tokens = tokens.saturating_add(estimate_text_tokens(text));
    }
    Ok(tokens)
}

fn estimate_text_tokens(text: &str) -> u64 {
    // The lazy-reset policy only needs a stable project-size estimate, not an
    // evaluator-model-specific tokenizer. Count BPE-like word chunks by UTF-8
    // length and punctuation as standalone token-like units.
    let mut tokens = 0u64;
    let mut word_bytes = 0u64;
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word_bytes = word_bytes.saturating_add(ch.len_utf8() as u64);
            continue;
        }
        tokens = tokens.saturating_add(estimate_word_tokens(word_bytes));
        word_bytes = 0;
        if !ch.is_whitespace() {
            tokens = tokens.saturating_add(1);
        }
    }
    tokens.saturating_add(estimate_word_tokens(word_bytes))
}

fn estimate_word_tokens(bytes: u64) -> u64 {
    if bytes == 0 {
        0
    } else {
        bytes.div_ceil(4)
    }
}

pub(crate) fn lazy_full_scope_reset_count(
    total_tokens: u64,
    project_size_tokens: u64,
    seed: u64,
    candidate_count: usize,
) -> usize {
    // With no staged text tokens, the policy's ratio has no defined
    // denominator and there is no meaningful project-size budget to sample.
    if total_tokens == 0 || project_size_tokens == 0 || candidate_count == 0 {
        return 0;
    }
    let denominator = 20u128 * project_size_tokens as u128;
    let numerator = total_tokens as u128;
    let mut count = (numerator / denominator) as usize;
    let remainder = numerator % denominator;
    if remainder != 0 {
        let mut rng = ResetRng::new(seed);
        if rng.next_bounded_u128(denominator) < remainder {
            count += 1;
        }
    }
    std::cmp::min(count, candidate_count)
}

pub(crate) fn sample_reset_expectations(
    non_selected: &[SelectedExpectation],
    count: usize,
    seed: u64,
) -> Vec<SelectedExpectation> {
    if count == 0 {
        return Vec::new();
    }
    let mut sampled = non_selected.to_vec();
    let mut rng = ResetRng::new(seed ^ 0x9e37_79b9_7f4a_7c15);
    for index in 0..sampled.len() {
        let remaining = sampled.len() - index;
        let swap = index + rng.next_bounded(remaining as u64) as usize;
        sampled.swap(index, swap);
    }
    sampled.truncate(count);
    sampled
}

pub(crate) fn reset_non_selected_expectation_histories(
    root: &Path,
    expectations: &[SelectedExpectation],
) -> Result<(), String> {
    let mut history_cache = HistoryCache::new();
    for expectation in expectations {
        reset_expectation_history_to_full_scope_seed(root, expectation, &mut history_cache)?;
    }
    Ok(())
}

fn reset_expectation_history_to_full_scope_seed(
    root: &Path,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
) -> Result<(), String> {
    let path = history_cache.path(root, expectation)?;
    if !path.exists() {
        return Ok(());
    }
    let mut records = read_history_records_from_path(&path)?;
    let Some(index) = records
        .iter()
        .rposition(|record| reusable_record_scope(record, expectation).is_some())
    else {
        return Ok(());
    };
    if reusable_record_scope(&records[index], expectation)
        .is_some_and(|scope| scope == full_scope())
    {
        return Ok(());
    }

    // Lazy reset changes only the next interrogation scope seed. Keep the
    // original scopeTreeOid with the old answer so exact-cache lookup cannot
    // treat a narrowed answer as a reusable full-scope result.
    records[index].scope = full_scope();
    let temp_path = compact_history_temp_path(&path)?;
    write_temp_file_then_replace(&temp_path, &path, |file| {
        for record in records {
            let line = render_check_log_record(&record).map_err(|err| err.to_string())?;
            file.write_all(line.as_bytes())
                .map_err(|err| format!("failed to write {}: {}", temp_path.display(), err))?;
        }
        Ok(())
    })?;
    history_cache.records.remove(&path);
    history_cache.reusable_records.clear();
    Ok(())
}

fn reusable_record_scope(
    record: &CheckRecord,
    expectation: &SelectedExpectation,
) -> Option<Vec<String>> {
    if !ObservedAnswerState::from_expected_and_observed(&expectation.a, &record.observed)
        .is_reusable_history()
    {
        return None;
    }
    sanitize_scope_for_hash(&record.scope).ok()
}

fn random_reset_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

struct ResetRng {
    state: u64,
}

impl ResetRng {
    fn new(seed: u64) -> ResetRng {
        ResetRng { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_bounded(&mut self, upper: u64) -> u64 {
        self.next_bounded_u128(upper as u128) as u64
    }

    fn next_bounded_u128(&mut self, upper: u128) -> u128 {
        if upper <= 1 {
            return 0;
        }
        let threshold = upper.wrapping_neg() % upper;
        loop {
            let value = self.next_u128();
            if value >= threshold {
                return value % upper;
            }
        }
    }

    fn next_u128(&mut self) -> u128 {
        ((self.next_u64() as u128) << 64) | self.next_u64() as u128
    }
}
