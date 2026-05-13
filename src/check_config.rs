use crate::*;

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let mut config_path = None;
    let mut query = None;
    let mut option_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = arg_to_string(&args[index])?;
        if arg == "--config" || arg == "-c" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{} requires a path", arg))?;
            let value = arg_to_string(value)?;
            set_check_config_path(&mut config_path, &value)?;
        } else if let Some(value) = arg.strip_prefix("--config=") {
            if value.is_empty() {
                return Err("--config requires a path".to_string());
            }
            set_check_config_path(&mut config_path, value)?;
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

fn set_check_config_path(config_path: &mut Option<PathBuf>, value: &str) -> Result<(), String> {
    if config_path.is_some() {
        return Err("duplicate --config".to_string());
    }
    *config_path = Some(normalize_check_config_path(value)?);
    Ok(())
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
    cache: Option<&mut RepoInspectionCache>,
) -> Result<CheckConfig, String> {
    let mut expansion = RawExpectationExpansion {
        root,
        cache,
        staged: config_path == Path::new(CHECK_PATH),
        include_stack: Vec::new(),
        expanded_paths: BTreeSet::new(),
        expectations: Vec::new(),
    };
    expansion.expand_items(config_path, raw.expectations)?;
    Ok(CheckConfig {
        version: raw.version,
        agent: raw.agent,
        expectations: expansion.expectations,
    })
}

struct RawExpectationExpansion<'a> {
    root: Option<&'a Path>,
    cache: Option<&'a mut RepoInspectionCache>,
    staged: bool,
    include_stack: Vec<String>,
    expanded_paths: BTreeSet<String>,
    expectations: Vec<Expectation>,
}

impl RawExpectationExpansion<'_> {
    fn expand_items(
        &mut self,
        config_path: &Path,
        items: Vec<RawExpectationItem>,
    ) -> Result<(), String> {
        for (index, item) in items.into_iter().enumerate() {
            match raw_expectation_kind(&item) {
                RawExpectationKind::Explicit => self.expectations.push(Expectation {
                    q: item.q.unwrap_or_default(),
                    a: item.a.unwrap_or_default(),
                    cooldown: item.cooldown,
                    thinking: item.thinking,
                }),
                RawExpectationKind::Generator => {
                    self.expand_path_generator(config_path, index, item)?
                }
                RawExpectationKind::Include => self.expand_include(config_path, index, item)?,
                RawExpectationKind::Invalid(message) => {
                    return Err(format!("expectation {} {}", index + 1, message));
                }
            }
        }
        Ok(())
    }

    fn expand_path_generator(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawExpectationItem,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let path = item.path.as_deref().unwrap_or_default();
        let template = item.q_template.as_deref().unwrap_or_default();
        validate_generator_template(template, item_number)?;
        let files = self.expand_paths(config_path, path, item_number, "path")?;
        for file in files {
            if !self.expanded_paths.insert(file.clone()) {
                return Err(format!(
                    "expectation {} expands duplicate spec path: {}",
                    item_number, file
                ));
            }
            let content = self.read_expanded_file(&file)?;
            self.expectations.push(Expectation {
                q: template.replace("{content}", &content),
                a: item.a.clone().unwrap_or_default(),
                cooldown: item.cooldown.clone(),
                thinking: item.thinking.clone(),
            });
        }
        Ok(())
    }

    fn expand_include(
        &mut self,
        config_path: &Path,
        index: usize,
        item: RawExpectationItem,
    ) -> Result<(), String> {
        let item_number = index + 1;
        let include = item.include.as_deref().unwrap_or_default();
        let files = self.expand_paths(config_path, include, item_number, "include")?;
        for file in files {
            if self.include_stack.contains(&file) {
                return Err(format!("recursive expectation include: {}", file));
            }
            let content = self.read_expanded_file(&file)?;
            let included: Vec<RawExpectationItem> = serde_yaml::from_str(&content)
                .map_err(|err| format!("failed to parse {}: {}", file, err))?;
            self.include_stack.push(file.clone());
            self.expand_items(Path::new(&file), included)?;
            self.include_stack.pop();
        }
        Ok(())
    }

    fn expand_paths(
        &mut self,
        config_path: &Path,
        path: &str,
        item_number: usize,
        label: &str,
    ) -> Result<Vec<String>, String> {
        let root = self.root.ok_or_else(|| {
            format!(
                "expectation {} uses {} but config expansion has no project root",
                item_number, label
            )
        })?;
        let files = match self.cache.as_deref_mut() {
            Some(cache) => cache.generator_paths(root, config_path, path, self.staged),
            None => expand_generator_paths(root, config_path, path, self.staged),
        }?;
        if files.is_empty() {
            return Err(format!(
                "expectation {} {} matched no files: {}",
                item_number, label, path
            ));
        }
        Ok(files)
    }

    fn read_expanded_file(&mut self, file: &str) -> Result<String, String> {
        let root = self
            .root
            .ok_or_else(|| "config expansion has no project root".to_string())?;
        if self.staged {
            match self.cache.as_deref_mut() {
                Some(cache) => cache.staged_file_content(root, file),
                None => read_staged_file_content(root, file),
            }
        } else {
            let absolute = root.join(file);
            match self.cache.as_deref_mut() {
                Some(cache) => cache.read_to_string(&absolute),
                None => fs::read_to_string(&absolute)
                    .map_err(|err| format!("failed to read {}: {}", file, err)),
            }
        }
    }
}
