use crate::check_types::CheckCommandArgs;
use crate::notes_cli::arg_to_string;
use crate::scope::normalize_repo_path;
use crate::CHECK_PATH;
use std::ffi::OsString;
use std::path::PathBuf;

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
            "canon check -q cannot be combined with expectation selectors, --all, or --ignore-cache"
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

fn normalize_check_config_path(value: &str) -> Result<PathBuf, String> {
    let normalized = normalize_repo_path(value).map_err(|err| format!("--config path: {}", err))?;
    if normalized == "." {
        return Err("--config path must name a file".to_string());
    }
    Ok(PathBuf::from(normalized))
}
