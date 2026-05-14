use crate::fs_util::ensure_dir;
use crate::notes_header::validate_note_key;
use crate::output::{write_stderr_bytes, write_stdout_bytes};
use crate::types::Config;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::thread;

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
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run rg: {}", err))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture rg stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture rg stderr".to_string())?;
    let stdout_thread = thread::spawn(move || stream_rg_stdout(stdout));
    let stderr_thread = thread::spawn(move || stream_rg_stderr(stderr));
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for rg: {}", err))?;
    stdout_thread
        .join()
        .map_err(|_| "rg stdout streaming thread panicked".to_string())??;
    stderr_thread
        .join()
        .map_err(|_| "rg stderr streaming thread panicked".to_string())??;
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        Some(code) => Err(format!("rg exited with status {}", code)),
        None => Err("rg terminated by signal".to_string()),
    }
}

fn stream_rg_stdout(stdout: impl Read) -> Result<(), String> {
    stream_rg_output(stdout, write_stdout_bytes)
}

fn stream_rg_stderr(stderr: impl Read) -> Result<(), String> {
    stream_rg_output(stderr, write_stderr_bytes)
}

fn stream_rg_output(
    reader: impl Read,
    mut write_chunk: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    // `rg` owns the search semantics, so canon treats each newline-delimited
    // byte line from the child pipe as the next known contiguous text segment.
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let bytes_read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|err| format!("failed to read rg output: {}", err))?;
        if bytes_read == 0 {
            return Ok(());
        }
        write_chunk(&buffer)?;
    }
}
