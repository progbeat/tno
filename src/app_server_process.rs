use crate::app_server::AppServerRunner;
use crate::config_types::AgentConfig;
use crate::evaluator_config::app_server_args;
use crate::evaluator_types::EvaluatorError;
use crate::thread_reuse_config::thread_reuse_config;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::time::{Duration, Instant};

pub(crate) fn spawn_app_server_reader(
    stdout: std::process::ChildStdout,
) -> (Receiver<Result<Value, String>>, JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {
                    let parsed = serde_json::from_str(line.trim_end())
                        .map_err(|err| format!("failed to parse app-server JSON: {}", err));
                    if sender.send(parsed).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ =
                        sender.send(Err(format!("failed to read app-server response: {}", err)));
                    return;
                }
            }
        }
    });
    (receiver, reader)
}

impl AppServerRunner {
    pub(crate) fn new(
        root: &Path,
        load_plugins: bool,
        agent: &AgentConfig,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let thread_reuse = thread_reuse_config(root)?;
        let mut command = Command::new("codex");
        command.args(app_server_args(root, load_plugins, agent)?);
        // `canon check` can itself run inside a Codex thread. The evaluator
        // app-server must create independent ephemeral threads, not attach to
        // the parent conversation through inherited thread identity.
        command.env_remove("CODEX_THREAD_ID");
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to start codex app-server: {}", err))?;
        let stdin = take_child_pipe(
            &mut child,
            |child| child.stdin.take(),
            "failed to open app-server stdin",
        )?;
        let stdout = take_child_pipe(
            &mut child,
            |child| child.stdout.take(),
            "failed to open app-server stdout",
        )?;
        let (messages, reader) = spawn_app_server_reader(stdout);
        let mut runner = AppServerRunner {
            child,
            stdin,
            messages,
            reader: Some(reader),
            next_id: 1,
            token_usage_by_turn: BTreeMap::new(),
            token_usage_updates_by_turn: BTreeMap::new(),
            context_compaction_events_by_turn: BTreeMap::new(),
            last_turn_usage: None,
            carryover_token_target: thread_reuse.carryover_token_target,
            turn_carryover_by_thread: BTreeMap::new(),
            retired_sessions: Default::default(),
        };
        runner.send_request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "canon",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true
                }
            }),
        )?;
        Ok(runner)
    }
}

impl Drop for AppServerRunner {
    fn drop(&mut self) {
        let _ = terminate_app_server_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn cleanup_error_after_missing_pipe(child: &mut Child, message: &str) -> EvaluatorError {
    match terminate_app_server_child(child) {
        Ok(()) => EvaluatorError::message(message),
        Err(err) => EvaluatorError::message(format!("{}; cleanup failed: {}", message, err)),
    }
}

fn take_child_pipe<T>(
    child: &mut Child,
    take: impl FnOnce(&mut Child) -> Option<T>,
    message: &str,
) -> Result<T, EvaluatorError> {
    take(child).ok_or_else(|| cleanup_error_after_missing_pipe(child, message))
}

#[cfg(unix)]
pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    if poll_app_server_child(child)?.is_some() {
        return Ok(());
    }
    let process_group = child.id() as i32;
    let mut errors = Vec::new();
    signal_process_group_or_kill_child(child, process_group, 15, &mut errors);
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    signal_process_group_or_kill_child(child, process_group, 9, &mut errors);
    wait_for_app_server_child(child)?;
    finish_app_server_cleanup(errors)
}

#[cfg(unix)]
pub(crate) fn signal_process_group(process_group: i32, signal_number: i32) -> Result<(), String> {
    let result = unsafe { crate::kill(-process_group, signal_number) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "failed to send signal {} to app-server process group {}: {}",
            signal_number,
            process_group,
            io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn signal_process_group_or_kill_child(
    child: &mut Child,
    process_group: i32,
    signal_number: i32,
    errors: &mut Vec<String>,
) {
    if let Err(err) = signal_process_group(process_group, signal_number) {
        errors.push(err);
        if let Err(err) = child.kill() {
            errors.push(format!("failed to kill app-server child: {}", err));
        }
    }
}

#[cfg(unix)]
pub(crate) fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if poll_app_server_child(child)?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn poll_app_server_child(child: &mut Child) -> Result<Option<ExitStatus>, String> {
    child
        .try_wait()
        .map_err(|err| format!("failed to poll app-server child: {}", err))
}

#[cfg(unix)]
fn finish_app_server_cleanup(errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(not(unix))]
pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|err| format!("failed to kill app-server child: {}", err))?;
    wait_for_app_server_child(child)?;
    Ok(())
}

fn wait_for_app_server_child(child: &mut Child) -> Result<(), String> {
    child
        .wait()
        .map(|_| ())
        .map_err(|err| format!("failed to wait for app-server child: {}", err))
}
