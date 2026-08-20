//! SysML v2 project manifest support.
//!
//! Implements KerML Clause 10 (Model Interchange Projects) providing:
//! - `.project.json` / `.meta.json` parsing ([`ProjectInfo`], [`ProjectMeta`])
//! - `.workspace.json` multi-project workspaces ([`WorkspaceInfo`])
//! - Project/workspace discovery by walking up the file tree ([`discover`])
//! - SHA-256 checksum verification ([`checksum`])
//! - Embedded standard-library registry ([`StdlibRegistry`])
//! - `.kpar` archive reading and writing (feature `kpar`, [`KparReader`], [`kpar::KparArchive`])
//! - Lock file support (feature `lock`, [`KparLockFile`])

mod checksum;
mod discover;
pub mod discovery;
mod error;
mod info;
mod meta;
mod project;
mod stdlib;
mod workspace;

#[cfg(feature = "kpar")]
pub mod kpar;
#[cfg(feature = "lock")]
mod lock;

pub use checksum::{compute_checksum, verify_checksum, ChecksumAlgorithm};
pub use discover::{discover_project, discover_workspace, DiscoveryResult};
pub use error::Error;
pub use info::{ProjectInfo, ProjectUsage};
pub use meta::{FileChecksum, ProjectMeta, SymbolEntry, SymbolIndex};
pub use project::{Project, ProjectHandle, ProjectRoot};
pub use stdlib::StdlibRegistry;
pub use workspace::{WorkspaceInfo, WorkspaceProject};

#[cfg(feature = "kpar")]
pub use kpar::KparReader;
#[cfg(feature = "kpar")]
pub use kpar::{read_kpar, write_kpar, KparArchive, KparBuilder, KparError};
#[cfg(feature = "lock")]
pub use lock::{KparLockFile, LockedProject, ProjectSource};

/// Result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;
