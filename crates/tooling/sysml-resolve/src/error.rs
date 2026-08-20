//! Error types for dependency resolution.

use std::path::PathBuf;

use sysml_manifest::ManifestError;

/// Errors that can occur during dependency resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// An I/O error while reading manifests or scanning directories.
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A manifest could not be loaded or parsed.
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),

    /// A cycle was detected in the dependency graph.
    ///
    /// The `cycle` field contains the package names forming the cycle,
    /// in order (last entry depends on first, completing the cycle).
    #[error("dependency cycle detected: {}", .cycle.join(" -> "))]
    Cycle { cycle: Vec<String> },

    /// A required dependency could not be found.
    #[error("dependency '{name}' not found at path '{path}'")]
    MissingDependency { name: String, path: PathBuf },

    /// A cached/fetched artifact's bytes did not match the expected checksum.
    #[error("checksum mismatch at {path}: expected {expected}, got {actual}. {hint}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
        hint: String,
    },

    /// The dependency source type is not yet supported in this phase.
    #[error("dependency source type not yet supported: {dep_type} (dependency '{name}')")]
    UnsupportedSource { name: String, dep_type: String },
}

impl ResolveError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ResolveError::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn checksum_mismatch(
        path: impl Into<PathBuf>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        ResolveError::ChecksumMismatch {
            path: path.into(),
            expected: expected.into(),
            actual: actual.into(),
            hint: hint.into(),
        }
    }
}
