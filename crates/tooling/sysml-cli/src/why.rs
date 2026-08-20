//! `sysml why` — explain why a dependency is present.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::json;
use sysml_manifest::load_manifest;
use sysml_resolve::resolve;

use crate::common::CliError;

pub fn run(package_name: &str, quiet: bool, json_output: bool) -> Result<(), CliError> {
    let (_manifest_path, manifest, manifest_dir) = crate::common::load_root_manifest_from_cwd()?;
    let graph = resolve(&manifest, &manifest_dir)
        .map_err(|e| CliError::user(format!("dependency resolution failed: {e}")))?;

    let root = manifest.project.name.clone();
    let edges = build_edges(&manifest, &graph);
    let path = find_path(&root, package_name, &edges).ok_or_else(|| {
        CliError::user(format!(
            "dependency '{package_name}' is not in the resolved graph"
        ))
    })?;

    if json_output {
        println!(
            "{}",
            json!({
                "target": package_name,
                "path": path,
            })
        );
        return Ok(());
    }

    if !quiet {
        println!("{}", path.join(" -> "));
    }

    Ok(())
}

fn build_edges(
    root_manifest: &sysml_manifest::SysmlManifest,
    graph: &sysml_resolve::ResolvedGraph,
) -> HashMap<String, Vec<String>> {
    let package_names: HashSet<String> = graph.packages.iter().map(|p| p.name.clone()).collect();
    let mut edges = HashMap::new();

    let mut root_children: Vec<String> = root_manifest
        .dependencies
        .keys()
        .filter(|name| package_names.contains(*name))
        .cloned()
        .collect();
    root_children.sort();
    edges.insert(root_manifest.project.name.clone(), root_children);

    for pkg in &graph.packages {
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

    edges
}

fn find_path(
    root: &str,
    target: &str,
    edges: &HashMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    if root == target {
        return Some(vec![root.to_owned()]);
    }

    let mut queue = VecDeque::new();
    let mut prev: HashMap<String, String> = HashMap::new();
    let mut visited: HashSet<String> = HashSet::new();

    visited.insert(root.to_owned());
    queue.push_back(root.to_owned());

    while let Some(node) = queue.pop_front() {
        if let Some(children) = edges.get(&node) {
            for child in children {
                if visited.contains(child) {
                    continue;
                }
                visited.insert(child.clone());
                prev.insert(child.clone(), node.clone());
                if child == target {
                    let mut path = vec![target.to_owned()];
                    let mut cur = target.to_owned();
                    while let Some(parent) = prev.get(&cur) {
                        path.push(parent.clone());
                        if parent == root {
                            break;
                        }
                        cur = parent.clone();
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(child.clone());
            }
        }
    }

    None
}
