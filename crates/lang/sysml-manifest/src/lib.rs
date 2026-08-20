//! SysML project manifest (`sysml.toml`) and lock file (`sysml.lock`) support.
//!
//! This crate provides:
//! - Parsing and serialization of `sysml.toml` project manifests
//! - Parsing and serialization of `sysml.lock` lock files
//! - Project and workspace discovery (walk-up-dirs)
//! - Dependency specification types

mod dependency;
mod discovery;
mod error;
mod lock;
mod manifest;
pub mod path_walk;
mod stdlib;

pub use dependency::{Dependency, GitRef};
pub use discovery::{find_manifest, find_workspace};
pub use error::ManifestError;
pub use path_walk::walk_up;
pub use lock::{load_lock, save_lock, LockFile, LockedPackage};
pub use manifest::{
    load_manifest, save_manifest, PackageConfig, ProjectConfig, SysmlManifest, WorkspaceConfig,
};
pub use stdlib::StdlibConfig;

/// Manifest filename.
pub const MANIFEST_FILENAME: &str = "sysml.toml";

/// Lock file filename.
pub const LOCK_FILENAME: &str = "sysml.lock";
