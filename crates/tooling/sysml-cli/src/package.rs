//! `sysml package` — Build a `.kpar` archive.

use std::fs;
use std::path::{Path, PathBuf};

use sysml_project::kpar::{write_kpar, KparBuilder};
use sysml_manifest::{find_manifest, MANIFEST_FILENAME};

use crate::common::CliError;

/// Run the `package` command.
pub fn run(manifest_path: Option<&Path>, output_dir: Option<&Path>) -> Result<(), CliError> {
    // Find and load manifest
    let (path, manifest) = if let Some(path) = manifest_path {
        let m = sysml_manifest::load_manifest(path)
            .map_err(|e| CliError::user(format!("failed to load {}: {e}", path.display())))?;
        (path.to_path_buf(), m)
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| CliError::internal(format!("failed to get current directory: {e}")))?;
        find_manifest(&cwd)
            .map_err(|e| CliError::user(format!("failed to find manifest: {e}")))?
            .ok_or_else(|| {
                CliError::user(format!(
                    "no {MANIFEST_FILENAME} found (searched from {})",
                    cwd.display()
                ))
            })?
    };

    let project_dir = path
        .parent()
        .ok_or_else(|| CliError::internal("manifest has no parent directory"))?;

    // Determine source directory — prefer src/ if it exists, fallback to project root
    let source_dir = if project_dir.join("src").is_dir() {
        project_dir.join("src")
    } else {
        project_dir.to_path_buf()
    };

    // Build KPAR archive
    let builder = KparBuilder::new(manifest.clone(), &source_dir);
    let archive = builder
        .build()
        .map_err(|e| CliError::internal(format!("failed to build KPAR archive: {e}")))?;

    // Determine output path
    let output = determine_output_path(
        output_dir,
        project_dir,
        &manifest.project.name,
        &manifest.project.version,
    )?;

    // Ensure output directory exists
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CliError::internal(format!(
                "failed to create output directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    // Write the archive
    write_kpar(&output, &archive)
        .map_err(|e| CliError::internal(format!("failed to write KPAR archive: {e}")))?;

    let file_count = archive.source_files.len();
    let file_size = fs::metadata(&output)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".to_owned());

    println!(
        "Packaged {} ({} files, {})",
        manifest.project.name, file_count, file_size
    );
    println!("  {}", output.display());

    Ok(())
}

fn determine_output_path(
    output_dir: Option<&Path>,
    project_dir: &Path,
    name: &str,
    version: &str,
) -> Result<PathBuf, CliError> {
    let filename = format!("{name}-{version}.kpar");

    if let Some(dir) = output_dir {
        Ok(dir.join(filename))
    } else {
        Ok(project_dir.join("target").join("package").join(filename))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
