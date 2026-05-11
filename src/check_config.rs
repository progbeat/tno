use crate::*;

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let mut config_path = None;
    let mut query = None;
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
        } else if arg == "-q" {
            if query.is_some() {
                return Err("duplicate -q".to_string());
            }
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "-q requires a question".to_string())?;
            let value = arg_to_string(value)?;
            if value.trim().is_empty() {
                return Err("-q question must not be empty".to_string());
            }
            query = Some(value);
        } else {
            option_args.push(args[index].clone());
        }
        index += 1;
    }
    if query.is_some() && !option_args.is_empty() {
        return Err(
            "canon check -q cannot be combined with expectation numbers, --fail-fast, or --ignore-cache"
                .to_string(),
        );
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        query,
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
    let raw: RawCheckConfig = serde_yaml::from_str(content)
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
    let raw: RawCheckConfig = serde_yaml::from_str(content)
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
                let staged = config_path == Path::new(CHECK_PATH);
                let files = match cache.as_deref_mut() {
                    Some(cache) => cache.generator_paths(root, config_path, path, staged),
                    None => expand_generator_paths(root, config_path, path, staged),
                }?;
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
