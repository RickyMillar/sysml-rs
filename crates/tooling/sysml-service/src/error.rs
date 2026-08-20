//! Service-layer error types.

use std::path::PathBuf;

use crate::execution::ForkAtTickError;

/// Errors returned by [`SysmlService`](crate::SysmlService) operations.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// File I/O failure (read, write, directory traversal).
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Parse failure from either parser backend.
    #[error("parse error: {0}")]
    Parse(String),

    /// Name resolution or import failure.
    #[error("resolution error: {0}")]
    Resolution(String),

    /// Elaboration failure.
    #[error("elaboration error: {0}")]
    Elaboration(String),

    /// Element not found by ID or name.
    #[error("element not found: {0}")]
    ElementNotFound(String),

    /// Manifest or project discovery failure.
    #[error("project error: {0}")]
    Project(String),

    /// Store (persistence) error.
    #[error("store error: {0}")]
    Store(String),

    /// Execution session error (simulation, action, orchestrator).
    #[error("execution error: {0}")]
    Execution(String),

    /// Visualization or export error.
    #[error("visualization error: {0}")]
    Visualization(String),

    /// Command not found in the registry.
    #[error("not found: {0}")]
    NotFound(String),

    /// Invalid input (e.g. JSON deserialization failure).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Internal error (e.g. serialization failure).
    #[error("internal error: {0}")]
    Internal(String),

    /// Structured error from `sysml.sessions.fork_with_overrides` when the
    /// optional `at_tick` argument cannot be honoured. Serialised to the
    /// wire as a JSON object with a `kind` discriminant so callers can
    /// switch on the error variant instead of parsing strings.
    ///
    /// Display renders the JSON form so transports that surface errors as
    /// opaque strings (MCP, HTTP 500 bodies) still carry the structured
    /// payload.
    #[error("{}", serde_json::to_string(.0).unwrap_or_else(|_| format!("{:?}", .0)))]
    ForkAtTick(ForkAtTickError),
}

impl From<ForkAtTickError> for ServiceError {
    fn from(e: ForkAtTickError) -> Self {
        ServiceError::ForkAtTick(e)
    }
}

impl ServiceError {
    /// Convenience for I/O errors with a path context.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<sysml_store::StoreError> for ServiceError {
    fn from(e: sysml_store::StoreError) -> Self {
        ServiceError::Store(e.to_string())
    }
}
