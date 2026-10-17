use std::fmt;
use std::path::PathBuf;

/// Errors produced by the config store.
#[derive(Debug)]
pub enum Error {
    /// Underlying filesystem failure, tagged with the path we were touching.
    Io { path: PathBuf, source: std::io::Error },
    /// The stored document was not valid JSON.
    Parse { path: PathBuf, source: serde_json::Error },
    /// The on-disk document's top-level value was not a JSON object.
    ///
    /// LightBurn's `prefs.ini` is a single JSON object; so is ours.
    NotAnObject { path: PathBuf },
    /// The file was written by a newer version of the app than this build
    /// knows how to read. Mirrors LightBurn's
    /// "This file was saved with a newer version of %1 (%2)" guard.
    NewerVersion { path: PathBuf, file_version: u64, app_version: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => {
                write!(f, "I/O error at {}: {source}", path.display())
            }
            Error::Parse { path, source } => {
                write!(f, "failed to parse JSON at {}: {source}", path.display())
            }
            Error::NotAnObject { path } => {
                write!(f, "document at {} is not a JSON object", path.display())
            }
            Error::NewerVersion { path, file_version, app_version } => write!(
                f,
                "{} was saved by a newer version (save version {file_version}, this build reads {app_version})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Parse { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
