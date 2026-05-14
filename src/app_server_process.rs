use crate::app_server::AppServerRunner;
use crate::evaluator_config::app_server_args;
use crate::output::write_stderr_line;
use crate::types::{AgentConfig, EvaluatorError};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::io::{self, BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
        load_plugins: bool,
        agent: &AgentConfig,
    ) -> Result<AppServerRunner, EvaluatorError> {
        let mut command = Command::new("codex");
        command.args(app_server_args(load_plugins, agent));
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to start codex app-server: {}", err))?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                return Err(cleanup_error_after_missing_pipe(
                    &mut child,
                    "failed to open app-server stdin",
                ));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                return Err(cleanup_error_after_missing_pipe(
                    &mut child,
                    "failed to open app-server stdout",
                ));
            }
        };
        let (messages, reader) = spawn_app_server_reader(stdout);
        let mut runner = AppServerRunner {
            child,
            stdin,
            messages,
            reader: Some(reader),
            next_id: 1,
            token_usage_by_turn: BTreeMap::new(),
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
        if let Err(err) = terminate_app_server_child(&mut self.child) {
            let _ = write_stderr_line(&format!("warning: {}", err));
        }
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

#[cfg(unix)]
pub(crate) fn terminate_app_server_child(child: &mut Child) -> Result<(), String> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(err) => return Err(format!("failed to poll app-server child: {}", err)),
    }
    let process_group = child.id() as i32;
    let mut errors = Vec::new();
    if let Err(err) = signal_process_group(process_group, 15) {
        errors.push(err);
        if let Err(err) = child.kill() {
            errors.push(format!("failed to kill app-server child: {}", err));
        }
    }
    if wait_for_child_exit(child, Duration::from_secs(2))? {
        return finish_app_server_cleanup(errors);
    }
    if let Err(err) = signal_process_group(process_group, 9) {
        errors.push(err);
        if let Err(err) = child.kill() {
            errors.push(format!("failed to kill app-server child: {}", err));
        }
    }
    child
        .wait()
        .map_err(|err| format!("failed to wait for app-server child: {}", err))?;
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
pub(crate) fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(true),
            Ok(None) => {}
            Err(err) => return Err(format!("failed to poll app-server child: {}", err)),
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

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
    child
        .wait()
        .map_err(|err| format!("failed to wait for app-server child: {}", err))?;
    Ok(())
}
