use crate::*;

pub(crate) fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if config.agent.instructions.trim().is_empty() {
        return Err("check.yml agent.instructions must not be empty".to_string());
    }
    validate_optional_model(config.agent.model.primary.as_deref(), "agent.model.primary")?;
    for (index, model) in config.agent.model.fallbacks.iter().enumerate() {
        validate_optional_model(
            Some(model.as_str()),
            &format!("agent.model.fallbacks[{}]", index),
        )?;
    }
    validate_thinking(&config.agent.thinking)?;
    for path in &config.agent.ignore {
        validate_relative_config_path(path, "agent ignore pattern")?;
    }
    for plugin in &config.agent.plugins {
        validate_plugin_config_key(plugin)?;
    }
    if config.expectations.is_empty() {
        return Err("check.yml expectations must not be empty".to_string());
    }
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        if expectation.q.trim().is_empty() {
            return Err(format!("expectation {} has an empty q", number));
        }
        if expectation.a.contains('\n') || expectation.a.contains('\r') {
            return Err(format!(
                "expectation {} expected answer must be single-line",
                number
            ));
        }
        if !expected_answer_is_allowed(&expectation.a) {
            return Err(format!(
                "expectation {} expected answer must be yes, no, or one lowercase ASCII option letter",
                number
            ));
        }
        if let Some(cooldown) = expectation.cooldown.as_deref() {
            parse_cooldown(cooldown)
                .map_err(|err| format!("expectation {} cooldown: {}", number, err))?;
        }
        if let Some(thinking) = expectation.thinking.as_deref() {
            validate_thinking(thinking)
                .map_err(|err| format!("expectation {} thinking: {}", number, err))?;
        }
    }
    Ok(())
}

pub(crate) fn validate_plugin_config_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("agent has an empty plugin entry".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("agent plugin entries must be single-line strings".to_string());
    }
    let Some((plugin, marketplace)) = value.split_once('@') else {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    };
    if plugin.is_empty() || marketplace.is_empty() || marketplace.contains('@') {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    }
    Ok(())
}

fn expected_answer_is_allowed(answer: &str) -> bool {
    matches!(answer, "yes" | "no")
        || matches!(answer.as_bytes(), [letter] if letter.is_ascii_lowercase())
}

pub(crate) fn validate_optional_model(value: Option<&str>, label: &str) -> Result<(), String> {
    let Some(model) = value else {
        return Ok(());
    };
    if model.trim().is_empty() {
        return Err(format!("check.yml {} must not be empty", label));
    }
    if model.contains('\n') || model.contains('\r') {
        return Err(format!("check.yml {} must be a single-line string", label));
    }
    Ok(())
}

pub(crate) fn validate_thinking(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("check.yml agent.thinking must not be empty".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("check.yml agent.thinking must be a single-line string".to_string());
    }
    match value {
        "off" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "adaptive" | "max" => {
            Ok(())
        }
        _ => Err(format!("unsupported check.yml agent.thinking: {}", value)),
    }
}

pub(crate) fn codex_reasoning_effort(thinking: &str) -> Option<&str> {
    match thinking {
        "adaptive" => None,
        "off" | "none" => Some("none"),
        "max" => Some("xhigh"),
        value => Some(value),
    }
}

pub(crate) fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
}

pub(crate) fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    normalize_repo_path(value)
        .map(|_| ())
        .map_err(|err| format!("{}: {}", label, err))
}
