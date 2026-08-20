//! SysML project dependency resolution.
//!
//! Resolves the dependency graph from a `sysml.toml` manifest,
//! fetching remote dependencies and caching them locally.
//!
//! # Current scope
//!
//! - Path dependency resolution (recursive, with cycle detection)
//! - Git dependency resolution (`rev`, `tag`, `branch`) with local cache
//! - KPAR dependency resolution for local paths and `file://` URLs
//! - Sysand-backed registry dependency resolution (exact + semver ranges)
//! - Lock file generation and change detection
//!
//! Additional registry backends are planned.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use sysml_manifest::load_manifest;
//! use sysml_resolve::{resolve, source_paths};
//!
//! let manifest_dir = Path::new("/my/project");
//! let manifest = load_manifest(&manifest_dir.join("sysml.toml")).unwrap();
//! let graph = resolve(&manifest, manifest_dir).unwrap();
//! let files = source_paths(&graph);
//! println!("Found {} .sysml source files", files.len());
//! ```

mod error;
mod graph;
mod lock;
mod providers;
mod registry;
mod resolver;

pub use error::ResolveError;
pub use graph::{PackageSource, ResolvedGraph, ResolvedPackage};
pub use lock::{generate_lock, is_lock_up_to_date};
pub use registry::{
    resolve_latest_registry_release_metadata, resolve_registry_release_metadata,
    RegistryReleaseMetadata,
};
pub use resolver::{resolve, source_paths};
