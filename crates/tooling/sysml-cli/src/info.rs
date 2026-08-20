//! `sysml info` — Show project information.

use std::path::Path;

use sysml_manifest::{find_manifest, MANIFEST_FILENAME};

use crate::common::CliError;

/// Run the `info` command.
pub fn run(manifest_path: Option<&Path>, json: bool) -> Result<(), CliError> {
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

    if json {
        print_json(&path, &manifest)?;
    } else {
        print_human(&path, &manifest);
    }

    Ok(())
}

fn print_human(path: &Path, manifest: &sysml_manifest::SysmlManifest) {
    println!("Project: {}", manifest.project.name);
    println!("Version: {}", manifest.project.version);
    if let Some(desc) = &manifest.project.description {
        println!("Description: {desc}");
    }
    if let Some(license) = &manifest.project.license {
        println!("License: {license}");
    }
    println!("SysML Edition: {}", manifest.project.sysml_edition);
    println!("IRI: {}", manifest.effective_iri());
    println!("Manifest: {}", path.display());

    if !manifest.project.authors.is_empty() {
        println!("Authors:");
        for author in &manifest.project.authors {
            println!("  - {author}");
        }
    }

    let stdlib = manifest.effective_stdlib();
    let enabled = stdlib.enabled_libraries();
    if !enabled.is_empty() {
        println!("Standard Libraries:");
        for lib in enabled {
            println!("  - {lib}");
        }
    }

    if !manifest.dependencies.is_empty() {
        println!("Dependencies:");
        for (name, dep) in &manifest.dependencies {
            let source = format_dep_source(dep);
            println!("  {name} ({source})");
        }
    }

    if let Some(ws) = &manifest.workspace {
        println!("Workspace Members:");
        for member in &ws.members {
            println!("  - {member}");
        }
    }
}

fn print_json(path: &Path, manifest: &sysml_manifest::SysmlManifest) -> Result<(), CliError> {
    let info = serde_json::json!({
        "name": manifest.project.name,
        "version": manifest.project.version,
        "description": manifest.project.description,
        "license": manifest.project.license,
        "sysml_edition": manifest.project.sysml_edition,
        "iri": manifest.effective_iri(),
        "manifest_path": path.to_string_lossy(),
        "authors": manifest.project.authors,
        "stdlib": manifest.effective_stdlib().enabled_libraries(),
        "dependencies": manifest.dependencies.keys().collect::<Vec<_>>(),
        "is_workspace": manifest.is_workspace(),
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&info)
            .map_err(|e| CliError::internal(format!("JSON serialization failed: {e}")))?
    );

    Ok(())
}

fn format_dep_source(dep: &sysml_manifest::Dependency) -> String {
    match dep {
        sysml_manifest::Dependency::Registry(version) => format!("registry: {version}"),
        sysml_manifest::Dependency::Detailed(d) => {
            if let Some(path) = &d.path {
                format!("path: {path}")
            } else if let Some(git) = &d.git {
                let ref_info = d
                    .tag
                    .as_ref()
                    .map(|t| format!(" tag={t}"))
                    .or_else(|| d.branch.as_ref().map(|b| format!(" branch={b}")))
                    .or_else(|| d.rev.as_ref().map(|r| format!(" rev={r}")))
                    .unwrap_or_default();
                format!("git: {git}{ref_info}")
            } else if let Some(kpar) = &d.kpar {
                format!("kpar: {kpar}")
            } else {
                "unknown".to_owned()
            }
        }
    }
}
