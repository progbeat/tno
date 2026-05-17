use crate::check_validation::codex_reasoning_effort;
use crate::config_types::AgentConfig;
use crate::hash::full_scope;
use crate::scope::{effective_ignore_patterns, normalize_repo_path};
use crate::thread_reuse_config::{thread_reuse_config, ThreadReuseConfig};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::path::Path;

pub(crate) fn evaluator_thread_config(
    agent: &AgentConfig,
    scope: &[String],
    model: Option<&str>,
    thinking: &str,
) -> Value {
    let root_permissions = evaluator_thread_root_permissions(agent, scope);
    let mut config = evaluator_base_config(
        permission_map_value(&root_permissions),
        "read",
        codex_reasoning_effort(thinking),
    );
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
) -> BTreeMap<String, String> {
    let mut root_permissions = BTreeMap::new();
    if scope == full_scope() {
        root_permissions.insert(".".to_string(), "read".to_string());
    } else {
        root_permissions.insert(".".to_string(), "none".to_string());
        for path in scope {
            root_permissions.insert(path.clone(), "read".to_string());
            root_permissions.insert(format!("{}/**", path), "read".to_string());
        }
    }
    deny_evaluator_project_paths(&mut root_permissions, agent);
    root_permissions
}

pub(crate) fn evaluator_startup_root_permissions(agent: &AgentConfig) -> BTreeMap<String, String> {
    let mut root_permissions = BTreeMap::new();
    root_permissions.insert(".".to_string(), "none".to_string());
    deny_evaluator_project_paths(&mut root_permissions, agent);
    root_permissions
}

pub(crate) fn deny_evaluator_project_paths(
    root_permissions: &mut BTreeMap<String, String>,
    agent: &AgentConfig,
) {
    // Scope and ignore enforcement must stay in Codex filesystem permissions;
    // do not replace it with filtered project copies or hidden project paths.
    for pattern in evaluator_deny_permission_patterns(agent) {
        root_permissions.insert(pattern, "none".to_string());
    }
}

pub(crate) fn evaluator_deny_permission_patterns(agent: &AgentConfig) -> Vec<String> {
    let mut patterns = Vec::new();
    // `effective_ignore_patterns` includes the mandatory `.git/canon/logs` and
    // `.git/canon/logs/**` denies, so even a full-scope `.` read cannot expose
    // runtime logs to evaluator sessions.
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

pub(crate) fn permission_map_value(permissions: &BTreeMap<String, String>) -> Value {
    let mut object = Map::new();
    for (path, permission) in permissions {
        object.insert(path.clone(), Value::String(permission.clone()));
    }
    Value::Object(object)
}

pub(crate) fn evaluator_base_config(
    root_permissions: Value,
    root_access: &str,
    reasoning_effort: Option<&str>,
) -> Value {
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
    if let Some(reasoning_effort) = reasoning_effort {
        config.insert(
            "model_reasoning_effort".to_string(),
            Value::String(reasoning_effort.to_string()),
        );
    }
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
    ]
    .into_iter()
    .map(|path| (path.to_string(), "read".to_string()))
    .collect::<Vec<_>>();
    deny_runtime_tree(&mut permissions, "~/.codex/sessions");
    deny_runtime_tree(&mut permissions, "~/.codex/memories");
    if let Some(home) = env::var_os("HOME").and_then(|home| home.into_string().ok()) {
        let codex_home = format!("{}/.codex", home.trim_end_matches('/'));
        deny_runtime_tree(&mut permissions, &format!("{}/sessions", codex_home));
        deny_runtime_tree(&mut permissions, &format!("{}/memories", codex_home));
    }
    permissions
}

fn deny_runtime_tree(permissions: &mut Vec<(String, String)>, path: &str) {
    permissions.push((path.to_string(), "none".to_string()));
    permissions.push((format!("{}/**", path), "none".to_string()));
}

pub(crate) fn enabled_plugins_config(agent: &AgentConfig) -> Value {
    let mut plugins = Map::new();
    for plugin in &agent.plugins {
        plugins.insert(plugin.clone(), json!({ "enabled": true }));
    }
    Value::Object(plugins)
}

pub(crate) fn app_server_args(
    root: &Path,
    load_plugins: bool,
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    let mut args = vec!["app-server".to_string()];
    if !load_plugins {
        args.push("--disable".to_string());
        args.push("plugins".to_string());
    }
    args.extend(app_server_startup_config_args(root, agent)?);
    args.push("--listen".to_string());
    args.push("stdio://".to_string());
    Ok(args)
}

pub(crate) fn app_server_startup_config_args(
    root: &Path,
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    let thread_reuse = thread_reuse_config(root)?;
    let mut args = Vec::new();
    push_config_arg(&mut args, "default_permissions=\"canon_check\"");
    push_config_arg(&mut args, "history.persistence=\"none\"");
    if let Some(reasoning_effort) = codex_reasoning_effort(&agent.thinking) {
        push_config_arg(
            &mut args,
            &format!("model_reasoning_effort={}", toml_string(reasoning_effort)),
        );
    }
    push_config_arg(&mut args, "permissions.canon_check.network.enabled=false");
    push_config_arg(&mut args, &app_server_startup_filesystem_arg(agent));
    push_config_arg(
        &mut args,
        &thread_reuse_carryover_token_target_arg(&thread_reuse),
    );
    Ok(args)
}

pub(crate) fn thread_reuse_carryover_token_target_arg(config: &ThreadReuseConfig) -> String {
    format!(
        "thread_reuse.carryover_token_target=[{},{}]",
        config.carryover_token_target.min, config.carryover_token_target.max
    )
}

pub(crate) fn app_server_model_key(model: Option<&str>) -> String {
    model.unwrap_or("<default>").to_string()
}

pub(crate) fn app_server_startup_filesystem_arg(agent: &AgentConfig) -> String {
    let mut entries = Vec::new();
    entries.push(toml_assignment(":root", &toml_string("read")));
    let mut project_root_entries = Vec::new();
    for (path, permission) in evaluator_startup_root_permissions(agent) {
        project_root_entries.push(toml_assignment(&path, &toml_string(&permission)));
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
    // TOML basic strings use the same delimiters and escape forms needed for
    // the values canon emits here, so the JSON string serializer gives us a
    // battle-tested quoted string. JSON may leave DEL/C1 controls literal, so
    // patch only those TOML-forbidden characters after JSON has handled the
    // common string grammar.
    let mut encoded =
        serde_json::to_string(value).expect("serializing a TOML basic string cannot fail");
    for ch in value.chars().filter(|ch| ch.is_control() && *ch > '\u{1f}') {
        encoded = encoded.replace(ch, &format!("\\u{:04X}", ch as u32));
    }
    encoded
}
