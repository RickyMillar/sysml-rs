//! `sysml fetch` — Resolve and materialize dependencies into cache without writing lockfile.

use serde_json::json;
use sysml_resolve::{resolve, PackageSource};

use crate::common::CliError;

pub fn run(quiet: bool, json_output: bool) -> Result<(), CliError> {
    let (_manifest_path, manifest, manifest_dir) = crate::common::load_root_manifest_from_cwd()?;
    let graph = resolve(&manifest, &manifest_dir)
        .map_err(|e| CliError::user(format!("dependency resolution failed: {e}")))?;

    if json_output {
        let packages: Vec<_> = graph
            .packages
            .iter()
            .map(|pkg| {
                let (requested_requirement, resolved_version, source_detail) = match &pkg.source {
                    PackageSource::Registry {
                        backend,
                        package,
                        requested,
                        version,
                    } => (
                        Some(requested.clone()),
                        Some(version.clone()),
                        json!({
                            "backend": backend,
                            "package": package,
                            "requested_requirement": requested,
                            "resolved_version": version,
                        }),
                    ),
                    _ => (None, None, serde_json::Value::Null),
                };
                json!({
                    "name": pkg.name,
                    "version": pkg.version,
                    "source": pkg.source.to_lock_source(),
                    "requested_requirement": requested_requirement,
                    "resolved_version": resolved_version,
                    "source_detail": source_detail,
                })
            })
            .collect();
        println!(
            "{}",
            json!({
                "status": "fetched",
                "packages": packages,
            })
        );
    } else if !quiet {
        println!("Fetched {} packages into cache", graph.packages.len());
        for pkg in &graph.packages {
            println!(
                "  {} {} ({})",
                pkg.name,
                pkg.version,
                pkg.source.to_lock_source()
            );
        }
    }

    Ok(())
}
