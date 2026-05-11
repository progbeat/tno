use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Once;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CHECK_PATH: &str = ".canon/check.yml";
const GIT_CANON_CACHE_DIR: &str = "canon/cache";
const GIT_CANON_LOG_DIR: &str = "canon/logs";
const DIAGNOSTIC_LOG_MAX_BYTES: u64 = 1024 * 1024;
const DIAGNOSTIC_LOG_FILES: [&str; 8] = [
    "0.jsonl", "1.jsonl", "2.jsonl", "3.jsonl", "4.jsonl", "5.jsonl", "6.jsonl", "7.jsonl",
];
const HISTORY_COMPACT_KEEP_RECORDS: usize = 5;
const HISTORY_COMPACT_SAMPLE_INTERVAL: u64 = 15;
const APP_SERVER_TURN_TIMEOUT_SECS: u64 = 300;
const DEFAULT_CHECK_TEMPLATE: &str = include_str!("../templates/check.yml");
const AGENTS_PATH: &str = "AGENTS.md";
const DEFAULT_AGENTS_TEMPLATE: &str = include_str!("../AGENTS.md");
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

mod app_server;
mod app_server_process;
mod app_server_protocol;
mod app_server_runner;
mod app_server_transport;
mod check;
mod check_cache;
mod check_command;
mod check_config;
mod check_errors;
mod check_generator_paths;
mod check_generator_templates;
mod check_generators;
mod check_interrogation;
mod check_interrogation_records;
mod check_interrogation_state;
mod check_model_fallback;
mod check_narrowing;
mod check_output;
mod check_preflight;
mod check_query;
mod check_query_command;
mod check_reporting;
mod check_result;
mod check_selection;
mod check_validation;
mod cli;
mod evaluator;
mod evaluator_config;
mod evaluator_json;
mod evaluator_prompt;
mod evaluator_response;
mod evaluator_response_cache;
mod evaluator_scope;
mod evaluator_turn;
mod fs_util;
mod gate;
mod git;
mod hash;
mod history;
mod history_append;
mod history_cache_key;
mod history_cleanup;
mod history_compaction;
mod history_reuse;
mod hooks;
mod logging;
mod notes;
mod notes_cli;
mod notes_header;
mod notes_index;
mod notes_restore;
mod project;
mod repo_inspection;
mod scope;
mod scope_hash;
mod staged_worktree;
mod time;
mod types;

pub(crate) use app_server::*;
pub(crate) use app_server_protocol::*;
pub(crate) use check::*;
pub(crate) use check_cache::*;
pub(crate) use check_command::*;
pub(crate) use check_config::*;
pub(crate) use check_errors::*;
pub(crate) use check_generator_paths::*;
pub(crate) use check_generator_templates::*;
pub(crate) use check_generators::*;
pub(crate) use check_interrogation::*;
pub(crate) use check_interrogation_records::*;
pub(crate) use check_interrogation_state::*;
pub(crate) use check_model_fallback::*;
pub(crate) use check_narrowing::*;
pub(crate) use check_output::*;
pub(crate) use check_preflight::*;
pub(crate) use check_query::*;
pub(crate) use check_query_command::*;
pub(crate) use check_reporting::*;
pub(crate) use check_result::*;
pub(crate) use check_selection::*;
pub(crate) use check_validation::*;
pub(crate) use cli::*;
pub(crate) use evaluator::*;
pub(crate) use evaluator_config::*;
pub(crate) use evaluator_json::*;
pub(crate) use evaluator_prompt::*;
pub(crate) use evaluator_response::*;
pub(crate) use evaluator_response_cache::*;
pub(crate) use evaluator_scope::*;
pub(crate) use evaluator_turn::*;
pub(crate) use fs_util::*;
#[cfg(test)]
pub(crate) use gate::*;
pub(crate) use git::*;
pub(crate) use hash::*;
pub(crate) use history::*;
pub(crate) use history_append::*;
pub(crate) use history_cache_key::*;
pub(crate) use history_cleanup::*;
pub(crate) use history_compaction::*;
pub(crate) use history_reuse::*;
#[cfg(test)]
pub(crate) use hooks::*;
pub(crate) use logging::*;
#[cfg(test)]
pub(crate) use notes::*;
pub(crate) use notes_cli::*;
pub(crate) use notes_header::*;
pub(crate) use notes_index::*;
pub(crate) use notes_restore::*;
pub(crate) use project::*;
pub(crate) use repo_inspection::*;
pub(crate) use scope::*;
pub(crate) use scope_hash::*;
pub(crate) use staged_worktree::*;
pub(crate) use time::*;
pub(crate) use types::*;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;
