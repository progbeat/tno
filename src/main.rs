use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Once;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
const GIT_CANON_CACHE_DIR: &str = "canon/cache";
const GIT_CANON_LOG_DIR: &str = "canon/logs";
const DIAGNOSTIC_LOG_MAX_BYTES: u64 = 128 * 1024;
const DIAGNOSTIC_LOG_FILES: [&str; 4] = ["0.jsonl", "1.jsonl", "2.jsonl", "3.jsonl"];
const HISTORY_COMPACT_KEEP_RECORDS: usize = 5;
const HISTORY_COMPACT_SAMPLE_INTERVAL: u64 = 15;
const CACHE_CLEANUP_SAMPLE_INTERVAL: u64 = 15;
const APP_SERVER_TURN_TIMEOUT_SECS: u64 = 120;
const DEFAULT_CHECK_TEMPLATE: &str = include_str!("../templates/check.yml");
const GIT_HOOKS_PATH: &str = ".git/hooks";
const PRE_COMMIT_HOOK_PATH: &str = ".git/hooks/pre-commit";
const DEFAULT_PRE_COMMIT_HOOK: &str = include_str!("../templates/pre-commit");
const RESULT_PASS: &str = "pass";
const RESULT_FAIL: &str = "fail";
const OBSERVED_IDK: &str = "idk";
const OBSERVED_MALFORMED: &str = "malformed";
const MALFORMED_REVIEW_WARNING: &str =
    "human review required: evaluator marked the expectation question as malformed";
const UNPARSEABLE_OBSERVED: &str = "unparseable";
const CHECK_FAILED_EXIT: &str = "__canon_check_failed_exit__";
const GATE_FAILED_EXIT: &str = "__canon_gate_failed_exit__";
static CHECK_INTERRUPTED: AtomicBool = AtomicBool::new(false);
static COMPACTION_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);
static SIGNAL_HANDLER_INIT: Once = Once::new();

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(unix)]
extern "C" fn handle_sigint(_: i32) {
    CHECK_INTERRUPTED.store(true, Ordering::SeqCst);
}

mod check;
mod check_config;
mod cli;
mod evaluator;
mod git;
mod hash;
mod history;
mod hooks;
mod logging;
mod notes;
mod scope;

pub(crate) use check::*;
pub(crate) use check_config::*;
pub(crate) use cli::*;
pub(crate) use evaluator::*;
pub(crate) use git::*;
pub(crate) use hash::*;
pub(crate) use history::*;
pub(crate) use hooks::*;
pub(crate) use logging::*;
pub(crate) use notes::*;
pub(crate) use scope::*;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;
