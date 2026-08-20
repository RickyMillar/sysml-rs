//! `sysml remove` — Remove a dependency from `sysml.toml`.

use std::path::Path;

use sysml_manifest::{find_manifest, save_manifest, LockFile, MANIFEST_FILENAME};

use crate::common::CliError;

/// Run the `remove` command.
pub fn run(name: &str) -> Result<(), CliError> {
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

    if manifest.remove_dependency(name).is_none() {
        return Err(CliError::user(format!(
            "dependency '{name}' not found in {MANIFEST_FILENAME}"
        )));
    }

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

    println!("Removed '{name}' from {}", manifest_path.display());
    Ok(())
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
