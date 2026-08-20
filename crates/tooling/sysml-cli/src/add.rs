//! `sysml add` — Add a dependency to `sysml.toml`.

use std::path::Path;

use sysml_manifest::{find_manifest, save_manifest, Dependency, LockFile, MANIFEST_FILENAME};

use crate::common::CliError;

/// Dependency source specification from CLI flags.
pub struct DepSource {
    pub path: Option<String>,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    pub kpar: Option<String>,
}

/// Run the `add` command.
pub fn run(name: &str, source: &DepSource) -> Result<(), CliError> {
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::internal(format!("failed to get current directory: {e}")))?;

    let (manifest_path, mut manifest) = find_manifest(&cwd)
        .map_err(|e| CliError::user(format!("failed to find manifest: {e}")))?
        .ok_or_else(|| {
            CliError::user(format!(
                "no {MANIFEST_FILENAME} found (searched from {})",
                cwd.display()
            ))
        })?;

    let dep = build_dependency(source)?;

    if manifest.dependencies.contains_key(name) {
        eprintln!("Updating existing dependency '{name}'");
    } else {
        eprintln!("Adding dependency '{name}'");
    }

    manifest.add_dependency(name, dep);

    let manifest_dir = manifest_path.parent().ok_or_else(|| {
        CliError::internal(format!(
            "failed to determine manifest directory for {}",
            manifest_path.display()
        ))
    })?;
    let lock = resolve_lock_file(&manifest, manifest_dir)?;

    save_manifest(&manifest_path, &manifest)
        .map_err(|e| CliError::internal(format!("failed to save {MANIFEST_FILENAME}: {e}")))?;
    save_lock_file(&lock, manifest_dir)?;

    println!("Added '{name}' to {}", manifest_path.display());
    Ok(())
}

fn build_dependency(source: &DepSource) -> Result<Dependency, CliError> {
    if let Some(path) = &source.path {
        Ok(Dependency::path(path))
    } else if let Some(git) = &source.git {
        if let Some(tag) = &source.tag {
            Ok(Dependency::git_tag(git, tag))
        } else if let Some(branch) = &source.branch {
            Ok(Dependency::git_branch(git, branch))
        } else if let Some(rev) = &source.rev {
            Ok(Dependency::git_rev(git, rev))
        } else {
            // Git with default branch — use branch "main" as convention
            Ok(Dependency::git_branch(git, "main"))
        }
    } else if let Some(kpar) = &source.kpar {
        Ok(Dependency::kpar(kpar))
    } else {
        Err(CliError::user(
            "must specify one of: --path, --git, or --kpar",
        ))
    }
}

fn resolve_lock_file(
    manifest: &sysml_manifest::SysmlManifest,
    manifest_dir: &Path,
) -> Result<LockFile, CliError> {
    let graph = sysml_resolve::resolve(manifest, manifest_dir)
        .map_err(|e| CliError::user(format!("dependency resolution failed: {e}")))?;
    Ok(sysml_resolve::generate_lock(&graph))
}

fn save_lock_file(lock: &LockFile, manifest_dir: &Path) -> Result<(), CliError> {
    let lock_path = manifest_dir.join(sysml_manifest::LOCK_FILENAME);
    sysml_manifest::save_lock(&lock_path, lock)
        .map_err(|e| CliError::internal(format!("failed to save lock file: {e}")))
}
