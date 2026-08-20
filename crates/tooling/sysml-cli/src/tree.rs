//! `sysml tree` — show dependency relationships.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use sysml_manifest::load_manifest;
use sysml_resolve::{resolve, PackageSource, ResolvedGraph};

use crate::common::CliError;

pub fn run(quiet: bool, json_output: bool) -> Result<(), CliError> {
    let (_manifest_path, manifest, manifest_dir) = crate::common::load_root_manifest_from_cwd()?;
    let graph = resolve(&manifest, &manifest_dir)
        .map_err(|e| CliError::user(format!("dependency resolution failed: {e}")))?;

    let model = build_dep_model(&manifest, &graph);

    if json_output {
        let edges = model
            .edges
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect::<serde_json::Map<String, serde_json::Value>>();
        let packages = model
            .package_meta
            .iter()
            .map(|(name, meta)| {
                json!({
                    "name": name,
                    "version": meta.version,
                    "source": meta.source,
                    "requested_requirement": meta.requested_requirement,
                    "resolved_version": meta.resolved_version,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            json!({
                "root": model.root,
                "edges": edges,
                "packages": packages,
            })
        );
        return Ok(());
    }

    if quiet {
        return Ok(());
    }

    println!("{}", model.root);
    let mut stack = HashSet::new();
    render_children(&model.root, "", &model, &mut stack);

    Ok(())
}

#[derive(Clone)]
struct PackageMeta {
    version: String,
    source: String,
    requested_requirement: Option<String>,
    resolved_version: Option<String>,
}

struct DepModel {
    root: String,
    edges: HashMap<String, Vec<String>>,
    package_meta: HashMap<String, PackageMeta>,
}

fn build_dep_model(
    root_manifest: &sysml_manifest::SysmlManifest,
    graph: &ResolvedGraph,
) -> DepModel {
    let root = root_manifest.project.name.clone();
    let package_names: HashSet<String> = graph.packages.iter().map(|p| p.name.clone()).collect();

    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    let mut package_meta = HashMap::new();

    let mut root_children: Vec<String> = root_manifest
        .dependencies
        .keys()
        .filter(|name| package_names.contains(*name))
        .cloned()
        .collect();
    root_children.sort();
    edges.insert(root.clone(), root_children);

    for pkg in &graph.packages {
        package_meta
            .entry(pkg.name.clone())
            .or_insert_with(|| PackageMeta {
                version: pkg.version.clone(),
                source: pkg.source.to_lock_source(),
                requested_requirement: match &pkg.source {
                    PackageSource::Registry { requested, .. } => Some(requested.clone()),
                    _ => None,
                },
                resolved_version: match &pkg.source {
                    PackageSource::Registry { version, .. } => Some(version.clone()),
                    _ => None,
                },
            });

        let manifest_path = pkg.source_dir.join("sysml.toml");
        let deps = if manifest_path.exists() {
            match load_manifest(&manifest_path) {
                Ok(dep_manifest) => {
                    let mut names: Vec<String> = dep_manifest
                        .dependencies
                        .keys()
                        .filter(|name| package_names.contains(*name))
                        .cloned()
                        .collect();
                    names.sort();
                    names
                }
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        edges.insert(pkg.name.clone(), deps);
    }

    DepModel {
        root,
        edges,
        package_meta,
    }
}

fn render_children(node: &str, prefix: &str, model: &DepModel, stack: &mut HashSet<String>) {
    let Some(children) = model.edges.get(node) else {
        return;
    };

    for (idx, child) in children.iter().enumerate() {
        let is_last = idx + 1 == children.len();
        let branch = if is_last { "└──" } else { "├──" };
        let label = package_label(child, &model.package_meta);
        println!("{prefix}{branch} {label}");

        if !stack.insert(child.clone()) {
            continue;
        }

        let next_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };
        render_children(child, &next_prefix, model, stack);
        stack.remove(child);
    }
}

fn package_label(name: &str, package_meta: &HashMap<String, PackageMeta>) -> String {
    if let Some(meta) = package_meta.get(name) {
        format!("{name} {} ({})", meta.version, meta.source)
    } else {
        name.to_owned()
    }
}
