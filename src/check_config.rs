use crate::*;

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let mut config_path = None;
    let mut option_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = arg_to_string(&args[index])?;
        if arg == "--config" || arg == "-c" {
            if config_path.is_some() {
                return Err("duplicate --config".to_string());
            }
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{} requires a path", arg))?;
            let value = arg_to_string(value)?;
            config_path = Some(normalize_check_config_path(&value)?);
        } else if let Some(value) = arg.strip_prefix("--config=") {
            if config_path.is_some() {
                return Err("duplicate --config".to_string());
            }
            if value.is_empty() {
                return Err("--config requires a path".to_string());
            }
            config_path = Some(normalize_check_config_path(value)?);
        } else {
            option_args.push(args[index].clone());
        }
        index += 1;
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        option_args,
    })
}

pub(crate) fn normalize_check_config_path(value: &str) -> Result<PathBuf, String> {
    let normalized = normalize_repo_path(value).map_err(|err| format!("--config path: {}", err))?;
    if normalized == "." {
        return Err("--config path must name a file".to_string());
    }
    Ok(PathBuf::from(normalized))
}

#[cfg(test)]
pub(crate) fn parse_check_config_content(
    config_path: &Path,
    content: &str,
) -> Result<CheckConfig, String> {
    let raw: RawCheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))?;
    let config = expand_raw_check_config(None, config_path, raw, None)?;
    validate_check_config(&config)?;
    Ok(config)
}

pub(crate) fn parse_check_config_content_with_root(
    root: &Path,
    config_path: &Path,
    content: &str,
    cache: &mut RepoInspectionCache,
) -> Result<CheckConfig, String> {
    let raw: RawCheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))?;
    let config = expand_raw_check_config(Some(root), config_path, raw, Some(cache))?;
    validate_check_config(&config)?;
    Ok(config)
}

pub(crate) fn expand_raw_check_config(
    root: Option<&Path>,
    config_path: &Path,
    raw: RawCheckConfig,
    mut cache: Option<&mut RepoInspectionCache>,
) -> Result<CheckConfig, String> {
    let mut expectations = Vec::new();
    let mut expanded_paths = BTreeSet::new();
    for (index, item) in raw.expectations.into_iter().enumerate() {
        match raw_expectation_kind(&item) {
            RawExpectationKind::Explicit => {
                expectations.push(Expectation {
                    q: item.q.unwrap_or_default(),
                    a: item.a,
                    cooldown: item.cooldown,
                    thinking: item.thinking,
                });
            }
            RawExpectationKind::Generator => {
                let item_number = index + 1;
                let path = item.path.as_deref().unwrap_or_default();
                let template = item.q_template.as_deref().unwrap_or_default();
                validate_generator_template(template, item_number)?;
                let root = root.ok_or_else(|| {
                    format!(
                        "expectation {} uses path but config expansion has no project root",
                        item_number
                    )
                })?;
                let files = expand_generator_paths(
                    root,
                    config_path,
                    path,
                    config_path == Path::new(CHECK_PATH),
                )?;
                if files.is_empty() {
                    return Err(format!(
                        "expectation {} path matched no files: {}",
                        item_number, path
                    ));
                }
                for file in files {
                    if !expanded_paths.insert(file.clone()) {
                        return Err(format!(
                            "expectation {} expands duplicate spec path: {}",
                            item_number, file
                        ));
                    }
                    let content = if config_path == Path::new(CHECK_PATH) {
                        match cache.as_deref_mut() {
                            Some(cache) => cache.staged_file_content(root, &file),
                            None => read_staged_file_content(root, &file),
                        }
                    } else {
                        let absolute = root.join(&file);
                        match cache.as_deref_mut() {
                            Some(cache) => cache.read_to_string(&absolute),
                            None => fs::read_to_string(&absolute)
                                .map_err(|err| format!("failed to read {}: {}", file, err)),
                        }
                    }?;
                    expectations.push(Expectation {
                        q: template.replace("{content}", &content),
                        a: item.a.clone(),
                        cooldown: item.cooldown.clone(),
                        thinking: item.thinking.clone(),
                    });
                }
            }
            RawExpectationKind::Invalid(message) => {
                return Err(format!("expectation {} {}", index + 1, message));
            }
        }
    }
    Ok(CheckConfig {
        version: raw.version,
        agent: raw.agent,
        expectations,
    })
}

pub(crate) enum RawExpectationKind {
    Explicit,
    Generator,
    Invalid(&'static str),
}

pub(crate) fn raw_expectation_kind(item: &RawExpectationItem) -> RawExpectationKind {
    match (&item.q, &item.q_template, &item.path) {
        (Some(_), None, None) => RawExpectationKind::Explicit,
        (None, Some(_), Some(_)) => RawExpectationKind::Generator,
        (Some(_), Some(_), _) => {
            RawExpectationKind::Invalid("must not contain both q and q_template")
        }
        (Some(_), None, Some(_)) => {
            RawExpectationKind::Invalid("must not contain path on an explicit expectation")
        }
        (None, Some(_), None) => RawExpectationKind::Invalid("generator must contain path"),
        (None, None, Some(_)) => RawExpectationKind::Invalid("generator must contain q_template"),
        (None, None, None) => RawExpectationKind::Invalid("must contain q or q_template"),
    }
}

pub(crate) fn validate_generator_template(template: &str, number: usize) -> Result<(), String> {
    if template.matches("{content}").count() != 1 {
        return Err(format!(
            "expectation {} q_template must contain exactly one {{content}} placeholder",
            number
        ));
    }
    let remainder = template.replace("{content}", "");
    if remainder.contains('{') || remainder.contains('}') {
        return Err(format!(
            "expectation {} q_template must not contain placeholders other than {{content}}",
            number
        ));
    }
    Ok(())
}

pub(crate) fn expand_generator_paths(
    root: &Path,
    config_path: &Path,
    path: &str,
    staged: bool,
) -> Result<Vec<String>, String> {
    validate_relative_config_path(path, "expectation generator path")?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new(""));
    let joined = normalize_repo_path(&join_repo_path(config_dir, path))?;
    if staged {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("ls-files")
            .arg("--")
            .arg(&joined)
            .output()
            .map_err(|err| format!("failed to expand spec path {}: {}", path, err))?;
        if !output.status.success() {
            return Err(format!(
                "failed to expand spec path {}: {}",
                path,
                command_output_trimmed(&output.stderr, "git ls-files stderr")?
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| "git ls-files output must be valid UTF-8".to_string())?;
        let mut files = stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        files.sort();
        files.dedup();
        return Ok(files);
    }
    expand_filesystem_generator_paths(root, &joined)
}

pub(crate) fn join_repo_path(config_dir: &Path, path: &str) -> String {
    if config_dir.as_os_str().is_empty() {
        path.to_string()
    } else {
        format!(
            "{}/{}",
            config_dir.to_string_lossy().trim_end_matches('/'),
            path
        )
    }
}

pub(crate) fn expand_filesystem_generator_paths(
    root: &Path,
    pattern: &str,
) -> Result<Vec<String>, String> {
    let Some(star_index) = pattern.find('*') else {
        let path = root.join(pattern);
        return if path.is_file() {
            Ok(vec![pattern.to_string()])
        } else {
            Ok(Vec::new())
        };
    };
    if pattern[star_index + 1..].contains('*') {
        return Err(format!("generator path supports only one *: {}", pattern));
    }
    let slash_index = pattern[..star_index]
        .rfind('/')
        .map(|index| index + 1)
        .unwrap_or(0);
    let dir = &pattern[..slash_index].trim_end_matches('/');
    let file_pattern = &pattern[slash_index..];
    let (prefix, suffix) = file_pattern
        .split_once('*')
        .ok_or_else(|| format!("invalid generator path: {}", pattern))?;
    let dir_path = if dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(dir)
    };
    if !dir_path.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir_path)
        .map_err(|err| format!("failed to read {}: {}", dir_path.display(), err))?
    {
        let entry =
            entry.map_err(|err| format!("failed to read {}: {}", dir_path.display(), err))?;
        let file_name = entry.file_name();
        let file_name = file_name
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 spec path in {}", dir_path.display()))?;
        if file_name.starts_with(prefix) && file_name.ends_with(suffix) && entry.path().is_file() {
            let repo_path = if dir.is_empty() {
                file_name.to_string()
            } else {
                format!("{}/{}", dir, file_name)
            };
            files.push(repo_path);
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn parse_check_options(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<CheckOptions, String> {
    let mut fail_fast = false;
    let mut ignore_cache = false;
    let mut numbers = Vec::new();
    for arg in args {
        if arg.to_str() == Some("--fail-fast") {
            if fail_fast {
                return Err("duplicate --fail-fast".to_string());
            }
            fail_fast = true;
        } else if arg.to_str() == Some("--ignore-cache") {
            if ignore_cache {
                return Err("duplicate --ignore-cache".to_string());
            }
            ignore_cache = true;
        } else {
            numbers.push(arg.clone());
        }
    }
    Ok(CheckOptions {
        selected: select_expectations(config, &numbers)?,
        fail_fast,
        ignore_cache,
    })
}

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
    if !value.contains('@') {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    }
    Ok(())
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

pub(crate) fn select_expectations(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    let mut selected_numbers = Vec::new();
    if args.is_empty() {
        selected_numbers.extend(1..=config.expectations.len());
    } else {
        let mut seen = BTreeSet::new();
        for arg in args {
            let text = arg
                .to_str()
                .ok_or("expectation number must be valid UTF-8".to_string())?;
            let number = text
                .parse::<usize>()
                .map_err(|_| format!("invalid expectation number: {}", text))?;
            if number == 0 {
                return Err("expectation numbers are 1-based".to_string());
            }
            if number > config.expectations.len() {
                return Err(format!("expectation number out of range: {}", number));
            }
            if !seen.insert(number) {
                return Err(format!("duplicate expectation number: {}", number));
            }
            selected_numbers.push(number);
        }
    }

    Ok(selected_numbers
        .into_iter()
        .map(|number| {
            let expectation = &config.expectations[number - 1];
            SelectedExpectation {
                number,
                id: expectation_id(&expectation.q, &expectation.a),
                q: expectation.q.clone(),
                a: expectation.a.clone(),
                cooldown: expectation
                    .cooldown
                    .as_deref()
                    .map(parse_cooldown)
                    .transpose()
                    .expect("validated cooldown must parse"),
                thinking: expectation.thinking.clone(),
            }
        })
        .collect())
}

pub(crate) fn parse_cooldown(value: &str) -> Result<Cooldown, String> {
    let value = value.trim();
    if value.len() < 2 {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must start with an integer".to_string());
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "duration integer is too large".to_string())?;
    if amount == 0 {
        return Err("must be greater than zero".to_string());
    }
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        _ => return Err("unit must be one of s, m, h, d, or w".to_string()),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_string())?;
    Ok(Cooldown { seconds })
}
