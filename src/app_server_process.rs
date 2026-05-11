use crate::*;

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
        let stdin = child.stdin.take().ok_or_else(|| {
            terminate_app_server_child(&mut child);
            EvaluatorError::message("failed to open app-server stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_app_server_child(&mut child);
            EvaluatorError::message("failed to open app-server stdout")
        })?;
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
        terminate_app_server_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(unix)]
pub(crate) fn terminate_app_server_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let process_group = child.id() as i32;
    signal_process_group(process_group, 15);
    if wait_for_child_exit(child, Duration::from_secs(2)) {
        return;
    }
    signal_process_group(process_group, 9);
    let _ = child.wait();
}

#[cfg(unix)]
pub(crate) fn signal_process_group(process_group: i32, signal_number: i32) {
    unsafe {
        let _ = crate::kill(-process_group, signal_number);
    }
}

#[cfg(unix)]
pub(crate) fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(unix))]
pub(crate) fn terminate_app_server_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
