use crate::*;

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
    diagnostic_log.write_event(
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
    )?;
    for error in reset_non_selected_expectation_histories(root, &reset.expectations) {
        diagnostic_log.write_event(
            "warning",
            "lazy_full_scope_reset.error",
            &[("message", json!(error))],
        )?;
    }
    Ok(())
}

pub(crate) fn apply_lazy_full_scope_reset_or_warn(
    root: &Path,
    config: &CheckConfig,
    usage: TokenUsage,
    non_selected: &[SelectedExpectation],
    diagnostic_log: &mut DiagnosticLogWriter,
) {
    if let Err(err) = apply_lazy_full_scope_reset(root, config, usage, non_selected, diagnostic_log)
    {
        let _ = diagnostic_log.write_event(
            "warning",
            "lazy_full_scope_reset.failed",
            &[("message", json!(err))],
        );
    }
}

pub(crate) struct LazyFullScopeResetPlan {
    pub(crate) project_size_tokens: u64,
    pub(crate) expectations: Vec<SelectedExpectation>,
}

pub(crate) fn plan_lazy_full_scope_reset(
    root: &Path,
    agent: &AgentConfig,
    total_tokens: u64,
    non_selected: &[SelectedExpectation],
    seed: u64,
) -> Result<LazyFullScopeResetPlan, String> {
    let project_size_tokens = estimate_staged_project_size_tokens(root, agent)?;
    let reset_count =
        lazy_full_scope_reset_count(total_tokens, project_size_tokens, seed, non_selected.len());
    Ok(LazyFullScopeResetPlan {
        project_size_tokens,
        expectations: sample_reset_expectations(non_selected, reset_count, seed),
    })
}

pub(crate) fn estimate_staged_project_size_tokens(
    root: &Path,
    agent: &AgentConfig,
) -> Result<u64, String> {
    let mut tokens = 0u64;
    for path_bytes in staged_tracked_path_bytes(root)? {
        // The spec excludes ignored content from the project-size estimate.
        if is_denied_path_bytes(agent, &path_bytes) {
            continue;
        }
        let content = read_staged_file_bytes_from_raw_path(root, &path_bytes)?;
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
    let denominator = 10u128 * project_size_tokens as u128;
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
) -> Vec<String> {
    let mut errors = Vec::new();
    for expectation in expectations {
        let path = match history_path(root, expectation) {
            Ok(path) => path,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        match reset_scope_and_invalidate_cache(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => errors.push(format!("failed to reset {}: {}", path.display(), err)),
        }
    }
    errors
}

fn reset_scope_and_invalidate_cache(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let records = read_history_records_from_path(path)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    let mut output = String::new();
    for record in records {
        if is_reusable_history_record(&record) {
            // Narrowed scope state is derived from reusable history records by
            // `latest_history_scope_with_cache`; exact cache hits are derived
            // from those same records by `reusable_history_record_for_source`.
            // Dropping the reusable records resets the next invocation to
            // full-scope interrogation and invalidates the exact cache without
            // writing synthetic history records that would violate cache shape.
            continue;
        }
        output.push_str(&render_check_log_record(&record));
    }
    fs::write(path, output)
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
