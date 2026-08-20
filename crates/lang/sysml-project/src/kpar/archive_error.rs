//! KPAR error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum KparError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid KPAR archive {path}: {message}")]
    InvalidArchive { path: PathBuf, message: String },

    #[error("missing {file} in KPAR archive {archive}")]
    MissingFile { archive: PathBuf, file: String },

    #[error("failed to parse {file} in KPAR archive: {message}")]
    JsonParse { file: String, message: String },

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("{0}")]
    Other(String),
}

impl KparError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        KparError::Io {
            path: path.into(),
            source,
        }
    }

    pub fn invalid(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        KparError::InvalidArchive {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn missing_file(archive: impl Into<PathBuf>, file: impl Into<String>) -> Self {
        KparError::MissingFile {
            archive: archive.into(),
            file: file.into(),
        }
    }
}
