//! `.kpar` archive support for SysML v2 projects.
//!
//! Provides two APIs:
//! - [`KparReader`]: Streaming reader for `.kpar` archives (from sysml-project)
//! - [`KparArchive`] + [`read_kpar`] + [`write_kpar`]: In-memory archive representation
//!   (formerly the standalone `sysml-kpar` crate)

mod reader;

mod archive_error;
mod archive_read;
mod archive_write;
pub mod schema;

pub use reader::KparReader;

// Re-exports from the former sysml-kpar crate
pub use archive_error::KparError;
pub use archive_read::read_kpar;
pub use archive_write::{write_kpar, KparBuilder};
// Re-export schema types for backward compatibility with former sysml-kpar API.
// Note: `schema::ProjectInfo` is a kpar-specific type, distinct from
// `crate::ProjectInfo` (the manifest project info type).
pub use schema::{ProjectInfo, ProjectMetadata, UsageEntry};

/// In-memory representation of a `.kpar` archive.
#[derive(Debug, Clone)]
pub struct KparArchive {
    /// The root directory name inside the archive.
    pub root_dir: String,

    /// Parsed `.project.json` contents.
    pub project_info: schema::ProjectInfo,

    /// Parsed `.meta.json` contents.
    pub metadata: schema::ProjectMetadata,

    /// Source files: (relative_path, contents).
    /// Paths are relative to the root directory.
    pub source_files: Vec<(String, Vec<u8>)>,
}

impl KparArchive {
    /// Get the project name.
    pub fn name(&self) -> &str {
        &self.project_info.name
    }

    /// Get the project version.
    pub fn version(&self) -> &str {
        &self.project_info.version
    }

    /// Get the list of source file paths (relative to root dir).
    pub fn source_file_names(&self) -> Vec<&str> {
        self.source_files
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }
}
