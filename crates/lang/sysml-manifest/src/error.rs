//! Manifest error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("failed to serialize manifest: {0}")]
    Serialize(String),

    #[error("invalid version constraint '{constraint}': {message}")]
    InvalidVersion { constraint: String, message: String },

    #[error("missing required field '{field}' in {path}")]
    MissingField { field: String, path: PathBuf },

    #[error("{0}")]
    Other(String),
}

impl ManifestError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ManifestError::Io {
            path: path.into(),
            source,
        }
    }

    pub fn parse(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        ManifestError::Parse {
            path: path.into(),
            message: message.into(),
        }
    }
}
