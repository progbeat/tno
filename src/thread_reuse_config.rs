use crate::project::command_output_trimmed;
use std::path::Path;
use std::process::Command;

const THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY: &str =
    "canon.threadReuse.carryoverTokenTarget";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CarryoverTokenTarget {
    pub(crate) min: u64,
    pub(crate) max: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThreadReuseConfig {
    pub(crate) carryover_token_target: CarryoverTokenTarget,
}

pub(crate) const DEFAULT_THREAD_REUSE_CONFIG: ThreadReuseConfig = ThreadReuseConfig {
    carryover_token_target: CarryoverTokenTarget {
        min: 10_000,
        max: 30_000,
    },
};

pub(crate) fn thread_reuse_config(root: &Path) -> Result<ThreadReuseConfig, String> {
    Ok(ThreadReuseConfig {
        carryover_token_target: configured_carryover_token_target(root)?,
    })
}

fn configured_carryover_token_target(root: &Path) -> Result<CarryoverTokenTarget, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .arg("--get")
        .arg(THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY)
        .output()
        .map_err(|err| format!("failed to run git config: {}", err))?;
    let stdout = command_output_trimmed(&output.stdout, "git config stdout")?;
    let stderr = command_output_trimmed(&output.stderr, "git config stderr")?;
    if output.status.success() {
        return parse_carryover_token_target(stdout);
    }
    if stdout.is_empty() && stderr.is_empty() {
        return Ok(DEFAULT_THREAD_REUSE_CONFIG.carryover_token_target);
    }
    Err(format!(
        "{} could not be read: {}",
        THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY, stderr
    ))
}

pub(crate) fn parse_carryover_token_target(value: &str) -> Result<CarryoverTokenTarget, String> {
    let Some((min, max)) = value.split_once(',') else {
        return Err(invalid_carryover_token_target(
            "must be a MIN,MAX token range",
        ));
    };
    if min.is_empty() || max.is_empty() {
        return Err(invalid_carryover_token_target(
            "must be a MIN,MAX token range",
        ));
    }
    let min = parse_positive_token_count(min)?;
    let max = parse_positive_token_count(max)?;
    if min > max {
        return Err(invalid_carryover_token_target(
            "MIN must be less than or equal to MAX",
        ));
    }
    Ok(CarryoverTokenTarget { min, max })
}

fn parse_positive_token_count(value: &str) -> Result<u64, String> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_carryover_token_target(
            "MIN and MAX must be positive integers",
        ));
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| invalid_carryover_token_target("value is too large"))?;
    if parsed == 0 {
        return Err(invalid_carryover_token_target(
            "MIN and MAX must be greater than zero",
        ));
    }
    Ok(parsed)
}

fn invalid_carryover_token_target(reason: &str) -> String {
    format!(
        "{} {}",
        THREAD_REUSE_CARRYOVER_TOKEN_TARGET_CONFIG_KEY, reason
    )
}
