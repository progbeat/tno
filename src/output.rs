use std::io::{self, Write};

pub(crate) fn write_stdout(text: &str) -> Result<(), String> {
    write_stdout_bytes(text.as_bytes())
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> Result<(), String> {
    let stdout = io::stdout();
    write_and_flush(stdout.lock(), "stdout", bytes)
}

pub(crate) fn write_stdout_line(text: &str) -> Result<(), String> {
    let mut output = String::from(text);
    output.push('\n');
    write_stdout(&output)
}

pub(crate) fn write_stderr(text: &str) -> Result<(), String> {
    write_stderr_bytes(text.as_bytes())
}

pub(crate) fn write_stderr_bytes(bytes: &[u8]) -> Result<(), String> {
    let stderr = io::stderr();
    write_and_flush(stderr.lock(), "stderr", bytes)
}

pub(crate) fn write_stderr_line(text: &str) -> Result<(), String> {
    let mut output = String::from(text);
    output.push('\n');
    write_stderr(&output)
}

fn write_and_flush(mut writer: impl Write, stream: &str, bytes: &[u8]) -> Result<(), String> {
    writer
        .write_all(bytes)
        .map_err(|err| format!("failed to write to {}: {}", stream, err))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush {}: {}", stream, err))
}
