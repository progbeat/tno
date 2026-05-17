use crate::path_io_error::PathIoError;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) type DiagnosticLogResult<T> = Result<T, DiagnosticLogError>;

#[derive(Debug)]
pub(crate) enum DiagnosticLogError {
    Command {
        command: &'static str,
        source: io::Error,
    },
    External {
        action: &'static str,
        message: String,
    },
    Io(PathIoError),
    Rename(DiagnosticLogRenameError),
    Json {
        description: &'static str,
        source: serde_json::Error,
    },
    InvalidRuntimeField {
        key: String,
        reason: &'static str,
    },
    InvalidConfig {
        key: &'static str,
        reason: String,
    },
    RecordTooLarge {
        size: u64,
        max_bytes: u64,
    },
    SizeOverflow {
        path: PathBuf,
    },
    EmptyFileList,
}

impl fmt::Display for DiagnosticLogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticLogError::Command { command, source } => {
                write!(formatter, "failed to run {}: {}", command, source)
            }
            DiagnosticLogError::External { action, message } => {
                write!(formatter, "failed to {}: {}", action, message)
            }
            DiagnosticLogError::Io(err) => err.fmt(formatter),
            DiagnosticLogError::Rename(err) => err.fmt(formatter),
            DiagnosticLogError::Json {
                description,
                source,
            } => write!(formatter, "failed to serialize {}: {}", description, source),
            DiagnosticLogError::InvalidRuntimeField { key, reason } => {
                write!(formatter, "runtime log field {:?} is {}", key, reason)
            }
            DiagnosticLogError::InvalidConfig { key, reason } => {
                write!(formatter, "{} {}", key, reason)
            }
            DiagnosticLogError::RecordTooLarge { size, max_bytes } => write!(
                formatter,
                "runtime log record is too large: {} bytes exceeds {} byte limit",
                size, max_bytes
            ),
            DiagnosticLogError::SizeOverflow { path } => {
                write!(formatter, "{} size is too large", path.display())
            }
            DiagnosticLogError::EmptyFileList => {
                formatter.write_str("diagnostic log config must include at least one log file")
            }
        }
    }
}

impl Error for DiagnosticLogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DiagnosticLogError::Command { source, .. } => Some(source),
            DiagnosticLogError::Io(err) => Some(err),
            DiagnosticLogError::Rename(err) => Some(err),
            DiagnosticLogError::Json { source, .. } => Some(source),
            DiagnosticLogError::External { .. }
            | DiagnosticLogError::InvalidRuntimeField { .. }
            | DiagnosticLogError::InvalidConfig { .. }
            | DiagnosticLogError::RecordTooLarge { .. }
            | DiagnosticLogError::SizeOverflow { .. }
            | DiagnosticLogError::EmptyFileList => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DiagnosticLogRenameError {
    from: PathBuf,
    to: PathBuf,
    kind: io::ErrorKind,
    source: io::Error,
}

impl DiagnosticLogRenameError {
    pub(crate) fn new(from: &Path, to: &Path, source: io::Error) -> DiagnosticLogRenameError {
        DiagnosticLogRenameError {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            kind: source.kind(),
            source,
        }
    }
}

impl fmt::Display for DiagnosticLogRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to rename {} to {} ({:?}): {}",
            self.from.display(),
            self.to.display(),
            self.kind,
            self.source
        )
    }
}

impl Error for DiagnosticLogRenameError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub(crate) fn log_io_error(
    action: &'static str,
    path: &Path,
    source: io::Error,
) -> DiagnosticLogError {
    DiagnosticLogError::Io(PathIoError::new(action, path, source))
}

pub(crate) fn external_log_error(action: &'static str, message: String) -> DiagnosticLogError {
    DiagnosticLogError::External { action, message }
}
