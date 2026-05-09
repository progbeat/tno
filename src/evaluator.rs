use crate::*;

pub(crate) fn evaluator_thread_config(
    agent: &AgentConfig,
    scope: &[String],
    model: Option<&str>,
) -> Value {
    let root_permissions = evaluator_thread_root_permissions(agent, scope);
    let mut config = evaluator_base_config(Value::Object(root_permissions), "read");
    if let Some(model) = model.or(agent.model.primary.as_deref()) {
        config["model"] = Value::String(model.to_string());
    }
    if !agent.plugins.is_empty() {
        config["plugins"] = enabled_plugins_config(agent);
    }
    config
}

pub(crate) fn evaluator_thread_root_permissions(
    agent: &AgentConfig,
    scope: &[String],
) -> Map<String, Value> {
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
    deny_evaluator_project_paths(&mut root_permissions, agent);
    root_permissions
}

pub(crate) fn evaluator_startup_root_permissions(agent: &AgentConfig) -> Map<String, Value> {
    let mut root_permissions = Map::new();
    root_permissions.insert(".".to_string(), Value::String("none".to_string()));
    deny_evaluator_project_paths(&mut root_permissions, agent);
    root_permissions
}

pub(crate) fn deny_evaluator_project_paths(
    root_permissions: &mut Map<String, Value>,
    agent: &AgentConfig,
) {
    // Scope and ignore enforcement must stay in Codex filesystem permissions;
    // do not replace it with filtered project copies or hidden project paths.
    for pattern in evaluator_deny_permission_patterns(agent) {
        root_permissions.insert(pattern, Value::String("none".to_string()));
    }
}

pub(crate) fn evaluator_deny_permission_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = Vec::new();
    for pattern in effective_ignore_patterns(agent) {
        let pattern = normalize_repo_path(&pattern).unwrap_or(pattern);
        // A recursive deny must also deny the directory entry itself, otherwise
        // root listings can still reveal ignored directories like `target/`.
        if let Some(prefix) = pattern.strip_suffix("/**") {
            push_unique_permission_pattern(&mut patterns, prefix.to_string());
        }
        push_unique_permission_pattern(&mut patterns, pattern);
    }
    patterns
}

pub(crate) fn push_unique_permission_pattern(patterns: &mut Vec<String>, pattern: String) {
    if !patterns.iter().any(|existing| existing == &pattern) {
        patterns.push(pattern);
    }
}

pub(crate) fn evaluator_base_config(root_permissions: Value, root_access: &str) -> Value {
    let mut filesystem = Map::new();
    filesystem.insert(":root".to_string(), Value::String(root_access.to_string()));
    filesystem.insert(":project_roots".to_string(), root_permissions);
    for (path, permission) in evaluator_runtime_permissions() {
        filesystem.insert(path, Value::String(permission));
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
    config.insert(
        "model_reasoning_effort".to_string(),
        Value::String("low".to_string()),
    );
    Value::Object(config)
}

pub(crate) fn evaluator_runtime_permissions() -> Vec<(String, String)> {
    let mut permissions = [
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
    .into_iter()
    .map(|path| (path.to_string(), "read".to_string()))
    .collect::<Vec<_>>();
    permissions.push(("~/.codex/sessions".to_string(), "write".to_string()));
    permissions.push(("~/.codex/sessions/**".to_string(), "write".to_string()));
    if let Some(home) = env::var_os("HOME").and_then(|home| home.into_string().ok()) {
        let sessions = format!("{}/.codex/sessions", home.trim_end_matches('/'));
        permissions.push((sessions.clone(), "write".to_string()));
        permissions.push((format!("{}/**", sessions), "write".to_string()));
    }
    permissions
}

pub(crate) fn enabled_plugins_config(agent: &AgentConfig) -> Value {
    let mut plugins = Map::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), json!({ "enabled": true }));
    }
    Value::Object(plugins)
}

pub(crate) fn app_server_args(load_plugins: bool, agent: &AgentConfig) -> Vec<String> {
    let mut args = vec!["app-server".to_string()];
    if !load_plugins {
        args.push("--disable".to_string());
        args.push("plugins".to_string());
    }
    args.extend(app_server_startup_config_args(agent));
    args.push("--listen".to_string());
    args.push("stdio://".to_string());
    args
}

pub(crate) fn app_server_startup_config_args(agent: &AgentConfig) -> Vec<String> {
    let mut args = Vec::new();
    push_config_arg(&mut args, "default_permissions=\"canon_check\"");
    push_config_arg(&mut args, "history.persistence=\"none\"");
    push_config_arg(&mut args, "model_reasoning_effort=\"low\"");
    push_config_arg(&mut args, "permissions.canon_check.network.enabled=false");
    push_config_arg(&mut args, &app_server_startup_filesystem_arg(agent));
    args
}

pub(crate) fn app_server_startup_filesystem_arg(agent: &AgentConfig) -> String {
    let mut entries = Vec::new();
    entries.push(toml_assignment(":root", &toml_string("read")));
    let mut project_root_entries = Vec::new();
    for (path, value) in evaluator_startup_root_permissions(agent) {
        project_root_entries.push(toml_assignment(
            &path,
            &toml_string(
                value
                    .as_str()
                    .expect("startup project root permissions are strings"),
            ),
        ));
    }
    entries.push(format!(
        "{}={{{}}}",
        toml_key_segment(":project_roots"),
        project_root_entries.join(",")
    ));
    for (path, permission) in evaluator_runtime_permissions() {
        entries.push(toml_assignment(&path, &toml_string(&permission)));
    }
    entries.push(format!("{}=32", toml_key_segment("glob_scan_max_depth")));
    format!(
        "permissions.canon_check.filesystem={{{}}}",
        entries.join(",")
    )
}

pub(crate) fn push_config_arg(args: &mut Vec<String>, value: &str) {
    args.push("-c".to_string());
    args.push(value.to_string());
}

pub(crate) fn toml_key_segment(value: &str) -> String {
    toml_string(value)
}

pub(crate) fn toml_assignment(key: &str, value: &str) -> String {
    format!("{}={}", toml_key_segment(key), value)
}

pub(crate) fn toml_string(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(ch),
        }
    }
    output.push('"');
    output
}

pub(crate) struct AppServerRunner {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
    token_usage_by_turn: BTreeMap<String, TokenUsage>,
}

pub(crate) struct LazyAppServerRunner {
    load_plugins: bool,
    agent: AgentConfig,
    inner: Option<AppServerRunner>,
}

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

impl LazyAppServerRunner {
    pub(crate) fn new(load_plugins: bool, agent: &AgentConfig) -> LazyAppServerRunner {
        LazyAppServerRunner {
            load_plugins,
            agent: agent.clone(),
            inner: None,
        }
    }

    fn inner(&mut self) -> Result<&mut AppServerRunner, String> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(self.load_plugins, &self.agent)?);
        }
        Ok(self
            .inner
            .as_mut()
            .expect("app-server runner is initialized"))
    }
}

impl AppServerRunner {
    fn new(load_plugins: bool, agent: &AgentConfig) -> Result<AppServerRunner, String> {
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
            "failed to open app-server stdin".to_string()
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_app_server_child(&mut child);
            "failed to open app-server stdout".to_string()
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

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
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
            if check_interrupted() {
                return Err("interrupted".to_string());
            }
            let message = self.read_message()?;
            self.record_token_usage(&message);
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
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
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
        let mut completed_text = String::new();
        let thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut turn_id: Option<String> = None;
        let mut interrupted = false;
        let mut interrupt_sent = false;
        loop {
            self.maybe_interrupt_turn(
                &mut interrupted,
                &mut interrupt_sent,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )?;
            let Some(message) = self.read_message_or_timeout()? else {
                continue;
            };
            self.record_token_usage(&message);
            if let Some(started_turn_id) = turn_started_id(&message) {
                turn_id = Some(started_turn_id);
                self.maybe_interrupt_turn(
                    &mut interrupted,
                    &mut interrupt_sent,
                    thread_id.as_deref(),
                    turn_id.as_deref(),
                )?;
            }
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("app-server {} failed: {}", method, error));
                }
                saw_response = true;
                if saw_completed {
                    return Ok(turn_text(text, completed_text));
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
                Some("item/completed") | Some("item/agentMessage/completed") => {
                    append_completed_agent_text(&message, &mut completed_text);
                }
                Some("turn/completed") => {
                    if interrupted {
                        return Err("interrupted".to_string());
                    }
                    if let Some(error) = app_server_error_message(&message) {
                        return Err(format!("app-server {} failed: {}", method, error));
                    }
                    saw_completed = true;
                    if saw_response {
                        return Ok(turn_text(text, completed_text));
                    }
                }
                Some(_) => {
                    if let Some(error) = app_server_error_message(&message) {
                        return Err(format!("app-server {} failed: {}", method, error));
                    }
                }
                _ => {}
            }
        }
    }

    fn maybe_interrupt_turn(
        &mut self,
        interrupted: &mut bool,
        interrupt_sent: &mut bool,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(), String> {
        if !check_interrupted() {
            return Ok(());
        }
        *interrupted = true;
        if *interrupt_sent {
            return Ok(());
        }
        let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
            return Ok(());
        };
        self.send_turn_interrupt(thread_id, turn_id)?;
        *interrupt_sent = true;
        Ok(())
    }

    fn send_turn_interrupt(&mut self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "turn/interrupt",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id
            }
        });
        writeln!(self.stdin, "{}", request)
            .map_err(|err| format!("failed to write app-server interrupt: {}", err))?;
        self.stdin
            .flush()
            .map_err(|err| format!("failed to flush app-server interrupt: {}", err))
    }

    fn read_message(&mut self) -> Result<Value, String> {
        loop {
            match self.read_message_or_timeout()? {
                Some(message) => return Ok(message),
                None if check_interrupted() => return Err("interrupted".to_string()),
                None => {}
            }
        }
    }

    fn read_message_or_timeout(&mut self) -> Result<Option<Value>, String> {
        match self.messages.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                if check_interrupted() {
                    Err("interrupted".to_string())
                } else {
                    Err("app-server closed stdout".to_string())
                }
            }
        }
    }

    fn record_token_usage(&mut self, message: &Value) {
        let Some((turn_id, usage)) = token_usage_update(message) else {
            return;
        };
        self.token_usage_by_turn.insert(turn_id, usage);
    }

    fn token_usage(&self) -> Option<TokenUsage> {
        let mut usage = TokenUsage::default();
        for turn_usage in self.token_usage_by_turn.values() {
            usage = usage.add(*turn_usage);
        }
        if usage.total_tokens == 0 {
            None
        } else {
            Some(usage)
        }
    }

    fn drain_token_usage_updates(&mut self) {
        loop {
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(message)) => self.record_token_usage(&message),
                Ok(Err(_)) => return,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

impl LazyAppServerRunner {
    pub(crate) fn token_usage(&self) -> Option<TokenUsage> {
        self.inner.as_ref().and_then(AppServerRunner::token_usage)
    }

    pub(crate) fn drain_token_usage_updates(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.drain_token_usage_updates();
        }
    }
}

impl TokenUsage {
    fn add(self, other: TokenUsage) -> TokenUsage {
        TokenUsage {
            total_tokens: self.total_tokens + other.total_tokens,
            input_tokens: self.input_tokens + other.input_tokens,
            cached_input_tokens: self.cached_input_tokens + other.cached_input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens + other.reasoning_output_tokens,
        }
    }
}

pub(crate) fn token_usage_update(message: &Value) -> Option<(String, TokenUsage)> {
    if message.get("method").and_then(Value::as_str) != Some("thread/tokenUsage/updated") {
        return None;
    }
    let params = message.get("params")?;
    let turn_id = params.get("turnId").and_then(Value::as_str)?.to_string();
    let usage = params.get("tokenUsage")?.get("last")?;
    Some((turn_id, parse_token_usage(usage)?))
}

pub(crate) fn turn_started_id(message: &Value) -> Option<String> {
    if message.get("method").and_then(Value::as_str) != Some("turn/started") {
        return None;
    }
    message
        .get("params")?
        .get("turn")?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn parse_token_usage(value: &Value) -> Option<TokenUsage> {
    Some(TokenUsage {
        total_tokens: value.get("totalTokens").and_then(Value::as_u64)?,
        input_tokens: value.get("inputTokens").and_then(Value::as_u64)?,
        cached_input_tokens: value
            .get("cachedInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value.get("outputTokens").and_then(Value::as_u64)?,
        reasoning_output_tokens: value
            .get("reasoningOutputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

pub(crate) fn render_token_usage_summary(usage: TokenUsage) -> String {
    format!(
        "Token usage: total={} input={} (+ {} cached) output={} (reasoning {})",
        format_number(usage.total_tokens),
        format_number(usage.input_tokens),
        format_number(usage.cached_input_tokens),
        format_number(usage.output_tokens),
        format_number(usage.reasoning_output_tokens)
    )
}

pub(crate) fn format_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            output.push(',');
        }
        output.push(ch);
    }
    output.chars().rev().collect()
}

pub(crate) fn app_server_error_message(message: &Value) -> Option<String> {
    let method = message.get("method").and_then(Value::as_str)?;
    if method != "error" && method != "turn/failed" && method != "turn/error" {
        if method == "turn/completed"
            && string_at_path(message, &["params", "turn", "status"]) == Some("failed")
        {
            return string_at_path(message, &["params", "turn", "error", "message"])
                .or_else(|| string_at_path(message, &["params", "turn", "error"]))
                .map(str::to_string)
                .or_else(|| Some("turn failed".to_string()));
        }
        return None;
    }
    string_at_path(message, &["params", "error", "message"])
        .or_else(|| string_at_path(message, &["params", "message"]))
        .or_else(|| string_at_path(message, &["params", "error", "codexErrorInfo"]))
        .or_else(|| string_at_path(message, &["error", "message"]))
        .or_else(|| string_at_path(message, &["message"]))
        .or_else(|| string_at_path(message, &["params", "error"]))
        .map(str::to_string)
        .or_else(|| Some(method.to_string()))
}

pub(crate) fn string_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

pub(crate) fn turn_text(delta_text: String, completed_text: String) -> String {
    if delta_text.trim().is_empty() {
        completed_text
    } else {
        delta_text
    }
}

pub(crate) fn append_completed_agent_text(message: &Value, output: &mut String) {
    let Some(params) = message.get("params") else {
        return;
    };
    if let Some(item) = params.get("item") {
        if is_assistant_message_item(item) {
            append_text_fields(item, output);
        }
    } else if message.get("method").and_then(Value::as_str) == Some("item/agentMessage/completed") {
        append_text_fields(params, output);
    }
}

pub(crate) fn is_assistant_message_item(item: &Value) -> bool {
    item.get("role").and_then(Value::as_str) == Some("assistant")
        || item
            .get("type")
            .and_then(Value::as_str)
            .map(|kind| kind.contains("agent") && kind.contains("message"))
            .unwrap_or(false)
}

pub(crate) fn append_text_fields(value: &Value, output: &mut String) {
    match value {
        Value::Array(items) => {
            for item in items {
                append_text_fields(item, output);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "text" {
                    if let Some(text) = value.as_str() {
                        output.push_str(text);
                    }
                } else {
                    append_text_fields(value, output);
                }
            }
        }
        _ => {}
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
        let _ = kill(-process_group, signal_number);
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

impl EvaluatorRunner for AppServerRunner {
    fn start_session(
        &mut self,
        root: &Path,
        instructions: &str,
        agent: &AgentConfig,
        model: Option<&str>,
        scope: &[String],
    ) -> Result<String, String> {
        let result = self.send_request(
            "thread/start",
            json!({
                "cwd": root.display().to_string(),
                "developerInstructions": instructions,
                "approvalPolicy": "never",
                "config": evaluator_thread_config(agent, scope, model),
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
        model: Option<&str>,
        scope: &[String],
    ) -> Result<String, String> {
        self.inner()?
            .start_session(root, instructions, agent, model, scope)
    }

    fn ask(&mut self, session_id: &str, prompt: &str) -> Result<String, String> {
        self.inner()?.ask(session_id, prompt)
    }
}

pub(crate) fn print_help() {
    print!(
        "{}",
        "canon - thread-scoped decisions and invariants\n\n\
Usage:\n  canon | canon pwd\n  canon p|path <key>\n  canon r|read <key>\n  canon w|write <key> [text]\n  canon a|append <key> [text]\n  canon d|del|delete|rm <key>\n  canon rg|g <pattern> [rg args...]\n"
            .to_string()
            + "  canon init\n  canon hook install\n  canon check [-c|--config <path>] [--fail-fast] [--ignore-cache] [expectation numbers...]\n  canon gate [expectation numbers...]\n"
    );
}
