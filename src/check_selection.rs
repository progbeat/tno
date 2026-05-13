use crate::*;

pub(crate) fn parse_check_options(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<CheckOptions, String> {
    let mut fail_fast = false;
    let mut ignore_cache = false;
    let mut numbers = Vec::new();
    for arg in args {
        if arg.to_str() == Some("--fail-fast") {
            if fail_fast {
                return Err("duplicate --fail-fast".to_string());
            }
            fail_fast = true;
        } else if arg.to_str() == Some("--ignore-cache") {
            if ignore_cache {
                return Err("duplicate --ignore-cache".to_string());
            }
            ignore_cache = true;
        } else {
            numbers.push(arg.clone());
        }
    }
    let selected = select_expectations(config, &numbers)?;
    let skipped = config.expectations.len().saturating_sub(selected.len());
    Ok(CheckOptions {
        selected,
        skipped,
        fail_fast,
        ignore_cache,
    })
}

pub(crate) fn select_expectations(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    // This expands command-line expectation numbers into the candidate set.
    // The final selected set is resolved later, after cooldown and reusable
    // passing cache-hit deselection.
    let mut selected_numbers = Vec::new();
    if args.is_empty() {
        selected_numbers.extend(1..=config.expectations.len());
    } else {
        let mut seen = BTreeSet::new();
        for arg in args {
            let text = arg
                .to_str()
                .ok_or("expectation number must be valid UTF-8".to_string())?;
            let number = text
                .parse::<usize>()
                .map_err(|_| format!("invalid expectation number: {}", text))?;
            if number == 0 {
                return Err("expectation numbers are 1-based".to_string());
            }
            if number > config.expectations.len() {
                return Err(format!("expectation number out of range: {}", number));
            }
            if !seen.insert(number) {
                return Err(format!("duplicate expectation number: {}", number));
            }
            selected_numbers.push(number);
        }
    }

    selected_numbers
        .into_iter()
        .map(|number| -> Result<SelectedExpectation, String> {
            let expectation = &config.expectations[number - 1];
            Ok(SelectedExpectation {
                number,
                id: expectation_id(&expectation.q, &expectation.a),
                q: expectation.q.clone(),
                a: expectation.a.clone(),
                cooldown: expectation
                    .cooldown
                    .as_deref()
                    .map(parse_cooldown)
                    .transpose()?,
                thinking: expectation.thinking.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) struct FinalSelection {
    pub(crate) selected: Vec<SelectedExpectation>,
    pub(crate) skipped: Vec<SelectedExpectation>,
}

pub(crate) struct FinalSelectionError {
    pub(crate) error: String,
    pub(crate) skipped: Vec<SelectedExpectation>,
}

pub(crate) fn final_selected_expectations(
    root: &Path,
    agent: &AgentConfig,
    selected: Vec<SelectedExpectation>,
    history_cache: &mut HistoryCache,
    now: u64,
) -> Result<FinalSelection, FinalSelectionError> {
    // CLI number filtering happens before this function. This is the shared
    // final-selection step for `canon check` and `canon gate`: cooldown removes
    // matching expectations from the selected set before cache reuse or gate
    // comparison.
    let mut remaining = Vec::new();
    let mut skipped = Vec::new();
    for expectation in selected {
        match cooldown_history_record(root, agent, &expectation, history_cache, now) {
            Ok(None) => remaining.push(expectation),
            Ok(Some(_)) => skipped.push(expectation),
            Err(error) => return Err(FinalSelectionError { error, skipped }),
        }
    }
    Ok(FinalSelection {
        selected: remaining,
        skipped,
    })
}

pub(crate) fn parse_cooldown(value: &str) -> Result<Cooldown, String> {
    if value.trim() != value {
        return Err("must use compact duration syntax without surrounding whitespace".to_string());
    }
    let Some((unit_index, unit)) = value.char_indices().next_back() else {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    };
    if unit_index == 0 {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    }
    let digits = &value[..unit_index];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must start with an integer".to_string());
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "duration integer is too large".to_string())?;
    if amount == 0 {
        return Err("must be greater than zero".to_string());
    }
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => return Err("unit must be one of s, m, h, d, or w".to_string()),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_string())?;
    Ok(Cooldown { seconds })
}
