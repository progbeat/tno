fn evaluator_thread_config(agent: &AgentConfig, scope: &[String]) -> Value {
    let mut root_permissions = Map::new();
    if scope == full_scope() {
        root_permissions.insert(".".to_string(), Value::String("read".to_string()));
    } else {
        root_permissions.insert(".".to_string(), Value::String("none".to_string()));
        for path in scope {
            root_permissions.insert(path.clone(), Value::String("read".to_string()));
            root_permissions.insert(format!("{}/**", path), Value::String("read".to_string()));
        }
    }
    for pattern in effective_ignore_patterns(agent) {
        root_permissions.insert(pattern, Value::String("none".to_string()));
    }

    let mut filesystem = Map::new();
    filesystem.insert("/".to_string(), Value::String("read".to_string()));
    filesystem.insert(
        ":project_roots".to_string(),
        Value::Object(root_permissions),
    );
    for path in evaluator_runtime_read_paths() {
        filesystem.insert(path.to_string(), Value::String("read".to_string()));
    }
    filesystem.insert("glob_scan_max_depth".to_string(), json!(32));

    let mut profile = Map::new();
    profile.insert("filesystem".to_string(), Value::Object(filesystem));
    profile.insert("network".to_string(), json!({ "enabled": false }));

    let mut permissions = Map::new();
    permissions.insert("canon_check".to_string(), Value::Object(profile));

    let mut config = Map::new();
    config.insert(
        "default_permissions".to_string(),
        Value::String("canon_check".to_string()),
    );
    config.insert("permissions".to_string(), Value::Object(permissions));
    config.insert("history".to_string(), json!({ "persistence": "none" }));
    if let Some(model) = &agent.model {
        config.insert("model".to_string(), Value::String(model.clone()));
    }
    if !agent.plugins.is_empty() {
        config.insert("plugins".to_string(), enabled_plugins_config(agent));
    }
    Value::Object(config)
}

fn evaluator_runtime_read_paths() -> &'static [&'static str] {
    &[
        "/bin/**",
        "/usr/bin/**",
        "/usr/lib/**",
        "/usr/libexec/**",
        "/System/**",
        "/Library/**",
        "/opt/homebrew/**",
        "/private/tmp/**",
        "~/.codex/tmp/**",
        ":tmpdir",
        ":slash_tmp",
    ]
}

fn enabled_plugins_config(agent: &AgentConfig) -> Value {
    let mut plugins = Map::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), json!({ "enabled": true }));
    }
    Value::Object(plugins)
}

fn app_server_args(load_plugins: bool) -> Vec<&'static str> {
    let mut args = vec!["app-server"];
    if !load_plugins {
        args.push("--disable");
        args.push("plugins");
    }
    args.push("--listen");
    args.push("stdio://");
    args
}

struct AppServerRunner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

struct LazyAppServerRunner {
    load_plugins: bool,
    inner: Option<AppServerRunner>,
}

impl LazyAppServerRunner {
    fn new(load_plugins: bool) -> LazyAppServerRunner {
        LazyAppServerRunner {
            load_plugins,
            inner: None,
        }
    }

    fn inner(&mut self) -> Result<&mut AppServerRunner, String> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(self.load_plugins)?);
        }
        Ok(self
            .inner
            .as_mut()
            .expect("app-server runner is initialized"))
    }
}

impl AppServerRunner {
    fn new(load_plugins: bool) -> Result<AppServerRunner, String> {
        let mut command = Command::new("codex");
        command.args(app_server_args(load_plugins));
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("failed to start codex app-server: {}", err))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("failed to open app-server stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or("failed to open app-server stdout".to_string())?;
        let mut runner = AppServerRunner {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
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

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server request: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server request: {}", err))?;
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("app-server {} failed: {}", method, error));
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("app-server {} response missing result", method));
            }
        }
    }

    fn send_turn_request(&mut self, method: &str, params: Value) -> Result<String, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server request: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server request: {}", err))?;

        let mut saw_response = false;
        let mut saw_completed = false;
        let mut text = String::new();
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("app-server {} failed: {}", method, error));
                }
                saw_response = true;
                if saw_completed {
                    return Ok(text);
                }
                continue;
            }
            match message.get("method").and_then(Value::as_str) {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = message
                        .get("params")
                        .and_then(|params| params.get("delta"))
                        .and_then(Value::as_str)
                    {
                        text.push_str(delta);
                    }
                }
                Some("turn/completed") => {
                    saw_completed = true;
                    if saw_response {
                        return Ok(text);
                    }
                }
                _ => {}
            }
        }
    }

    fn read_message(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .map_err(|err| format!("failed to read app-server response: {}", err))?;
        if bytes == 0 {
            return Err("app-server closed stdout".to_string());
        }
        serde_json::from_str(line.trim_end())
            .map_err(|err| format!("failed to parse app-server JSON: {}", err))
    }
}

impl Drop for AppServerRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        let result = self.send_request(
            "thread/start",
            json!({
                "cwd": root.display().to_string(),
                "developerInstructions": instructions,
                "approvalPolicy": "never",
                "config": evaluator_thread_config(agent, scope),
                "ephemeral": true,
                "sessionStartSource": "startup"
            }),
        )?;
        result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or("thread/start response missing thread.id".to_string())
    }

    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
        self.send_turn_request(
            "turn/start",
            json!({
                "threadId": session_id,
                "input": [
                    {
                        "type": "text",
                        "text": prompt
                    }
                ]
            }),
        )
    }
}

impl EvaluatorRunner for LazyAppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        self.inner()?
            .start_session(root, instructions, agent, scope)
    }

    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
        self.inner()?.ask(session_id, prompt)
    }
}

fn print_help() {
    print!(
        "{}",
        "canon - thread-scoped decisions and invariants\n\n\
Usage:\n  canon | canon pwd\n  canon p|path <key>\n  canon r|read <key>\n  canon w|write <key> [text]\n  canon a|append <key> [text]\n  canon d|del|delete|rm <key>\n  canon rg|g <pattern> [rg args...]\n"
            .to_string()
            + "  canon init\n  canon hook install\n  canon check [-c|--config <path>] [--fail-fast] [--ignore-cache] [expectation numbers...]\n  canon gate [expectation numbers...]\n"
    );
}
