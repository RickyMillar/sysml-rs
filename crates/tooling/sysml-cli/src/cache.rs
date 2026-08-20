//! `sysml cache` command group.

use std::path::PathBuf;

use clap::Subcommand;
use directories::{BaseDirs, ProjectDirs};
use serde_json::json;

use crate::common::CliError;

#[derive(Subcommand)]
pub enum CacheCommand {
    /// Remove cached dependency artifacts
    Clean {
        /// Also remove other cache files under the cache root
        #[arg(long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Suppress non-error output
        #[arg(long)]
        quiet: bool,
    },
}

#[allow(clippy::needless_pass_by_value)]
pub fn run(command: CacheCommand) -> Result<(), CliError> {
    match command {
        CacheCommand::Clean { all, json, quiet } => clean(all, quiet, json),
    }
}

fn clean(all: bool, quiet: bool, json_output: bool) -> Result<(), CliError> {
    let root = cache_root();
    let target = if all {
        root
    } else {
        root.join("dependencies")
    };

    let existed = target.exists();
    if existed {
        if target.is_dir() {
            std::fs::remove_dir_all(&target).map_err(|e| {
                CliError::internal(format!("failed to remove '{}': {e}", target.display()))
            })?;
        } else {
            std::fs::remove_file(&target).map_err(|e| {
                CliError::internal(format!("failed to remove '{}': {e}", target.display()))
            })?;
        }
    }

    if json_output {
        println!(
            "{}",
            json!({
                "removed": existed,
                "path": target,
                "scope": if all { "all" } else { "dependencies" },
            })
        );
    } else if !quiet {
        if existed {
            println!("Removed cache: {}", target.display());
        } else {
            println!("Cache already clean: {}", target.display());
        }
    }

    Ok(())
}

fn cache_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("SYSML_RS_CACHE_DIR") {
        if !override_dir.trim().is_empty() {
            return PathBuf::from(override_dir);
        }
    }

    if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
        return project_dirs.cache_dir().to_path_buf();
    }

    if let Some(base_dirs) = BaseDirs::new() {
        return base_dirs.cache_dir().join("sysml-rs");
    }

    PathBuf::from("/tmp/sysml-rs-cache")
}
