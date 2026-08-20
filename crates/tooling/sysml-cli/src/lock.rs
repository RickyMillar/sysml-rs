//! `sysml lock` — Resolve dependencies and update `sysml.lock`.

use serde_json::json;
use sysml_manifest::LOCK_FILENAME;
use sysml_resolve::{generate_lock, is_lock_up_to_date, resolve};

use crate::common::CliError;

pub fn run_with_options(force: bool, quiet: bool, json_output: bool) -> Result<(), CliError> {
    let (_manifest_path, manifest, manifest_dir) = crate::common::load_root_manifest_from_cwd()?;

    let graph = resolve(&manifest, &manifest_dir)
        .map_err(|e| CliError::user(format!("dependency resolution failed: {e}")))?;

    let new_lock = generate_lock(&graph);
    let lock_path = manifest_dir.join(LOCK_FILENAME);

    // Check if lock file needs updating
    if !force && lock_path.exists() {
        let existing_lock = sysml_manifest::load_lock(&lock_path)
            .map_err(|e| CliError::internal(format!("failed to read existing lock file: {e}")))?;

        if is_lock_up_to_date(&graph, &existing_lock) {
            if json_output {
                println!(
                    "{}",
                    json!({
                        "status": "up_to_date",
                        "packages": new_lock.packages.len(),
                    })
                );
            } else if !quiet {
                println!(
                    "Lock file is up to date ({} packages)",
                    new_lock.packages.len()
                );
            }
            return Ok(());
        }
    }

    sysml_manifest::save_lock(&lock_path, &new_lock)
        .map_err(|e| CliError::internal(format!("failed to save lock file: {e}")))?;

    if json_output {
        let packages: Vec<_> = new_lock
            .packages
            .iter()
            .map(|p| {
                let (resolved_version, source_detail) = parse_registry_source(&p.source)
                    .map(|(backend, package, version)| {
                        (
                            Some(version.clone()),
                            json!({
                                "backend": backend,
                                "package": package,
                                "requested_requirement": p.requested,
                                "resolved_version": version,
                            }),
                        )
                    })
                    .unwrap_or((None, serde_json::Value::Null));
                json!({
                    "name": p.name,
                    "version": p.version,
                    "source": p.source,
                    "checksum": p.checksum,
                    "requested_requirement": p.requested,
                    "resolved_version": resolved_version,
                    "source_detail": source_detail,
                })
            })
            .collect();
        println!(
            "{}",
            json!({
                "status": "updated",
                "packages": packages,
                "lock_path": lock_path,
            })
        );
    } else if !quiet {
        println!(
            "Resolved {} packages, wrote {}",
            new_lock.packages.len(),
            lock_path.display()
        );

        for pkg in &new_lock.packages {
            println!("  {} {} ({})", pkg.name, pkg.version, pkg.source);
        }
    }

    Ok(())
}

fn parse_registry_source(source: &str) -> Option<(String, String, String)> {
    let rest = source.strip_prefix("registry:")?;
    let (backend, package_and_version) = rest.split_once(':')?;
    let (package, version) = package_and_version.rsplit_once('@')?;
    Some((
        backend.to_owned(),
        package.to_owned(),
        version.to_owned(),
    ))
}
