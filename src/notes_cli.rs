use crate::*;

pub(crate) const INDEX_LOCK_STALE_AFTER_SECS: u64 = 600;

pub(crate) fn require_key(args: &[OsString], index: usize) -> Result<&str, String> {
    let key = args
        .get(index)
        .ok_or("missing key".to_string())
        .and_then(|arg| arg.to_str().ok_or("key must be valid UTF-8".to_string()))?;
    validate_note_key(key)?;
    Ok(key)
}

pub(crate) fn arg_to_string(arg: &OsString) -> Result<String, String> {
    arg.to_str()
        .map(|value| value.to_string())
        .ok_or("argument must be valid UTF-8".to_string())
}

pub(crate) fn collect_text(args: &[OsString], start: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    let rest = args.get(start..).ok_or_else(|| {
        format!(
            "text start index {} exceeds argument count {}",
            start,
            args.len()
        )
    })?;
    for arg in rest {
        parts.push(arg.to_str().ok_or("text must be valid UTF-8".to_string())?);
    }
    Ok(parts.join(" "))
}

pub(crate) fn collect_text_or_stdin(args: &[OsString], start: usize) -> Result<String, String> {
    if args.len() > start {
        return collect_text(args, start);
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|err| format!("failed to read stdin: {}", err))?;
    Ok(text)
}

pub(crate) fn run_rg(config: &Config, rg_args: &[OsString]) -> Result<(), String> {
    if rg_args.is_empty() {
        return Err("missing rg pattern".to_string());
    }
    ensure_dir(&config.root)?;
    let mut command = Command::new("rg");
    command.args(rg_args);
    command.arg("--");
    command.arg(&config.root);
    let status = command
        .status()
        .map_err(|err| format!("failed to run rg: {}", err))?;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(format!("rg exited with status {}", code)),
        None => Err("rg terminated by signal".to_string()),
    }
}
