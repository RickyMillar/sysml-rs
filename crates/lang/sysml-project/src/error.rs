use std::path::PathBuf;

/// Errors returned by `sysml-project` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing required field `{field}` in {context}")]
    MissingField { field: String, context: String },

    #[error("symbol conflict: `{symbol}` defined in both `{first}` and `{second}`")]
    SymbolConflict {
        symbol: String,
        first: String,
        second: String,
    },

    #[error("checksum mismatch for `{path}`: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("unsupported checksum algorithm: {0}")]
    UnsupportedAlgorithm(String),

    #[error("project not found at {0}")]
    ProjectNotFound(PathBuf),

    #[error("version parse error: {0}")]
    Version(#[from] semver::Error),

    #[cfg(feature = "kpar")]
    #[error("kpar error: {0}")]
    Kpar(#[from] zip::result::ZipError),

    #[cfg(feature = "lock")]
    #[error("lock file parse error: {0}")]
    LockParse(#[from] toml::de::Error),

    #[cfg(feature = "lock")]
    #[error("lock file serialize error: {0}")]
    LockSerialize(#[from] toml::ser::Error),
}
