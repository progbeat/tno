use crate::fs_util::ensure_dir;
use crate::git::resolve_git_path;
use crate::time::parse_record_timestamp;
use crate::types::{CheckRecord, SelectedExpectation};
use crate::GIT_CANON_CACHE_DIR;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LATEST_NON_PASS_FILE: &str = "latest-non-pass.json";

pub(crate) fn latest_recorded_non_pass_timestamp(
    root: &Path,
    expectation: &SelectedExpectation,
) -> Result<Option<u64>, String> {
    let path = latest_non_pass_path(root, expectation)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let record = serde_json::from_str::<LatestNonPassRecord>(&content).map_err(|err| {
        format!(
            "invalid latest non-pass JSON in {}: {}",
            path.display(),
            err
        )
    })?;
    Ok(parse_record_timestamp(&record.timestamp))
}

pub(crate) fn write_latest_non_pass_record(
    root: &Path,
    expectation: &SelectedExpectation,
    record: &CheckRecord,
) -> Result<(), String> {
    if record.passed() {
        return Ok(());
    }
    let path = latest_non_pass_path(root, expectation)?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    // Keep this bounded order state separate from answer history and runtime
    // logs: answer history excludes human-review records, and runtime logs are
    // diagnostic output rather than input to future command behavior.
    let mut line = json!({
        "timestamp": record.timestamp,
        "result": record.result,
        "observed": record.observed,
    })
    .to_string();
    line.push('\n');
    file.write_all(line.as_bytes())
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    file.flush()
        .map_err(|err| format!("failed to flush {}: {}", path.display(), err))
}

fn latest_non_pass_path(root: &Path, expectation: &SelectedExpectation) -> Result<PathBuf, String> {
    Ok(resolve_git_path(root, GIT_CANON_CACHE_DIR)?
        .join(&expectation.id)
        .join(LATEST_NON_PASS_FILE))
}

#[derive(Deserialize)]
struct LatestNonPassRecord {
    timestamp: String,
}
