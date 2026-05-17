use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct PathIoError {
    action: &'static str,
    path: PathBuf,
    kind: io::ErrorKind,
    source: io::Error,
}

impl PathIoError {
    pub(crate) fn new(action: &'static str, path: &Path, source: io::Error) -> PathIoError {
        PathIoError {
            action,
            path: path.to_path_buf(),
            kind: source.kind(),
            source,
        }
    }
}

impl fmt::Display for PathIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to {} {} ({:?}): {}",
            self.action,
            self.path.display(),
            self.kind,
            self.source
        )
    }
}

impl Error for PathIoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}
