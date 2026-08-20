//! Workspace-level project discovery for the LSP server.
//!
//! Discovers project structure by checking (in order):
//! 1. `.workspace.json` / `.project.json` (KerML interchange format)
//! 2. `sysml.toml` (Cargo-style manifest)
//! 3. Implicit project (any directory with `.sysml` files)
//!
//! This ensures cross-file imports work regardless of which manifest
//! format the user chose (or if they chose none at all).

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Instant;

use sysml_manifest::{Dependency, SysmlManifest};

/// Service-side replacement for the LSP-only `telemetry_control` module.
///
/// Only the predicates this file uses are exposed; rate-limited tracing
/// becomes plain `tracing::*` emission. The dependency-trace env var is
/// read on every call (cheap, OS-cached).
mod telemetry_control {
    pub(super) fn dependency_trace_enabled() -> bool {
        std::env::var("SYSML_DEPENDENCY_TRACE").is_ok()
            || std::env::var("SYSML_LSP_DEPENDENCY_TRACE").is_ok()
    }
}

/// Service-side replacement for the LSP-only `telemetry_events` module.
///
/// Each event becomes a plain `tracing` emission with identical field
/// shape; counter increments and rate limiting drop. Subscribers in the
/// LSP transport still observe the events.
mod telemetry_events {
    pub(super) fn dependency_hydration_failure(
        manifest_root: &str,
        dependency_name: &str,
        source_kind: &str,
        reason: &str,
    ) {
        tracing::warn!(
            event = "lsp.dependency.hydration_failure",
            manifest_root,
            dependency_name,
            source_kind,
            reason,
            "dependency hydration telemetry"
        );
    }

    pub(super) fn workspace_discovery_mode(
        root: &str,
        mode: &str,
        project_count: usize,
        include_stdlib: bool,
    ) {
        tracing::info!(
            event = "lsp.workspace.discovery_mode",
            root,
            mode,
            project_count,
            include_stdlib,
            "workspace discovery telemetry"
        );
    }

    pub(super) fn workspace_member_expansion(
        workspace_root: &str,
        declared_members: usize,
        loaded_members: usize,
        missing_members: usize,
    ) {
        tracing::info!(
            event = "lsp.workspace.member_expansion",
            workspace_root,
            declared_members,
            loaded_members,
            missing_members,
            "workspace member expansion telemetry"
        );
    }
}
use sysml_project::{
    discover_project, DiscoveryResult, Project, ProjectHandle, ProjectInfo, ProjectRoot, WorkspaceInfo,
};
use sysml_resolve::{PackageSource, ResolveError, ResolvedPackage};

/// Result of workspace discovery.
#[derive(Debug)]
pub struct WorkspaceDiscovery {
    /// Discovered projects (may be empty if no manifests found).
    pub projects: Vec<Project>,
    /// Whether stdlib should be loaded.
    pub include_stdlib: bool,
    /// Human-readable description of what was discovered (for UX messages).
    pub discovery_description: String,
    /// Stable discovery mode identifier for diagnostics/telemetry.
    pub discovery_mode: &'static str,
}

/// The first user-project ID (stdlib gets 0..9).
const FIRST_USER_PROJECT_ID: u32 = 10;

#[derive(Debug, Clone)]
pub struct DependencyResolutionFailure {
    pub dependency_name: String,
    pub source_kind: &'static str,
    pub reason: &'static str,
    pub message: String,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct DependencyResolutionOutcome {
    pub dependency_name: String,
    pub source_kind: &'static str,
    pub hydrated_packages: Vec<ResolvedPackage>,
    pub failure: Option<DependencyResolutionFailure>,
}

#[derive(Debug, Clone)]
pub struct DependencyHydrationReport {
    pub outcomes: Vec<DependencyResolutionOutcome>,
    pub hydrated_packages: Vec<ResolvedPackage>,
    pub failures: Vec<DependencyResolutionFailure>,
}

pub fn dependency_source_kind(dep: &Dependency) -> &'static str {
    match dep {
        Dependency::Registry(_) => "registry",
        Dependency::Detailed(d) => {
            let mut selected = 0usize;
            let mut source_kind = "invalid";
            if d.path.is_some() {
                selected += 1;
                source_kind = "path";
            }
            if d.git.is_some() {
                selected += 1;
                source_kind = "git";
            }
            if d.kpar.is_some() {
                selected += 1;
                source_kind = "kpar";
            }
            if d.version.is_some() {
                selected += 1;
                source_kind = "registry";
            }
            if selected == 1 {
                source_kind
            } else {
                "invalid"
            }
        }
    }
}

fn resolved_source_kind(source: &PackageSource) -> &'static str {
    match source {
        PackageSource::Path(_) => "path",
        PackageSource::Git { .. } => "git",
        PackageSource::Kpar { .. } => "kpar",
        PackageSource::Registry { .. } => "registry",
        PackageSource::Stdlib => "stdlib",
    }
}

fn resolve_error_reason_and_action(
    source_kind: &str,
    error: &ResolveError,
) -> (&'static str, String) {
    match error {
        ResolveError::MissingDependency { .. } => (
            "missing_dependency",
            "Verify the dependency path or source URL exists and is reachable.".to_owned(),
        ),
        ResolveError::UnsupportedSource { dep_type, .. } => {
            if source_kind == "registry" {
                let action = if dep_type == "registry-sysand-unconfigured" {
                    "Configure Sysand registry index via `.sysml/registries/sysand/index.json` in the workspace root, or set `SYSML_REGISTRY_SYSAND_INDEX`.".to_owned()
                } else if dep_type == "registry-version-range" {
                    "Use an exact registry version or a semver range (for example `^1.2` or `~1.2`).".to_owned()
                } else if dep_type.starts_with("registry-backend-") {
                    "Use `registry = \"sysand\"` or configure a supported registry backend.".to_owned()
                } else {
                    "Resolve registry configuration issues and retry dependency hydration.".to_owned()
                };
                ("unsupported_source", action)
            } else {
                (
                    "unsupported_source",
                    "Use a supported dependency source or update to a resolver version with provider support.".to_owned(),
                )
            }
        }
        ResolveError::ChecksumMismatch { .. } => (
            "checksum_mismatch",
            "Clear dependency cache (`sysml cache clean --all`) and run `sysml lock` again.".to_owned(),
        ),
        ResolveError::Cycle { .. } => (
            "dependency_cycle",
            "Break the dependency cycle declared in manifests before retrying.".to_owned(),
        ),
        ResolveError::Manifest(_) => (
            "manifest_error",
            "Fix the dependency manifest (`sysml.toml`) reported in this error.".to_owned(),
        ),
        ResolveError::Io { .. } => {
            if source_kind == "registry" {
                let message = error.to_string();
                if message.contains("no compatible release") {
                    (
                        "no_compatible_release",
                        "Update the dependency requirement or publish a compatible release to the registry index.".to_owned(),
                    )
                } else if message.contains("malformed release version") {
                    (
                        "registry_index_invalid",
                        "Fix malformed version entries in the registry index JSON and retry.".to_owned(),
                    )
                } else {
                    (
                        "io_error",
                        "Check filesystem/network access for this dependency source and retry.".to_owned(),
                    )
                }
            } else {
                (
                    "io_error",
                    "Check filesystem/network access for this dependency source and retry.".to_owned(),
                )
            }
        }
    }
}

fn emit_hydration_failure(
    manifest_root: &Path,
    failure: &DependencyResolutionFailure,
    emit_telemetry: bool,
) {
    if !emit_telemetry {
        return;
    }
    telemetry_events::dependency_hydration_failure(
        &manifest_root.display().to_string(),
        &failure.dependency_name,
        failure.source_kind,
        failure.reason,
    );
}

pub fn resolve_manifest_dependencies(
    manifest: &SysmlManifest,
    manifest_root: &Path,
    emit_telemetry: bool,
) -> Vec<DependencyResolutionOutcome> {
    let dep_trace = telemetry_control::dependency_trace_enabled();
    let started_at = Instant::now();
    if dep_trace {
        tracing::info!(
            manifest_root = %manifest_root.display(),
            dependency_count = manifest.dependencies.len(),
            "dependency trace: resolving manifest dependencies"
        );
    }

    let mut outcomes = Vec::with_capacity(manifest.dependencies.len());
    for (dep_name, dep) in &manifest.dependencies {
        let source_kind = dependency_source_kind(dep);
        if source_kind == "invalid" {
            let failure = DependencyResolutionFailure {
                dependency_name: dep_name.clone(),
                source_kind: "invalid",
                reason: "invalid_spec",
                message: "Dependency must select exactly one source: path, git, kpar, or registry.".to_owned(),
                action: "Fix the dependency entry in `sysml.toml` to use exactly one source.".to_owned(),
            };
            emit_hydration_failure(manifest_root, &failure, emit_telemetry);
            if dep_trace {
                tracing::info!(
                    manifest_root = %manifest_root.display(),
                    dependency = %dep_name,
                    source_kind,
                    reason = failure.reason,
                    "dependency trace: dependency resolution failed (invalid spec)"
                );
            }
            outcomes.push(DependencyResolutionOutcome {
                dependency_name: dep_name.clone(),
                source_kind: "invalid",
                hydrated_packages: Vec::new(),
                failure: Some(failure),
            });
            continue;
        }

        let mut scoped_manifest = manifest.clone();
        let mut scoped_deps = BTreeMap::new();
        scoped_deps.insert(dep_name.clone(), dep.clone());
        scoped_manifest.dependencies = scoped_deps;

        match sysml_resolve::resolve(&scoped_manifest, manifest_root) {
            Ok(graph) => {
                if dep_trace {
                    tracing::info!(
                        manifest_root = %manifest_root.display(),
                        dependency = %dep_name,
                        source_kind,
                        hydrated_package_count = graph.packages.len(),
                        "dependency trace: dependency resolution hydrated package(s)"
                    );
                }
                outcomes.push(DependencyResolutionOutcome {
                    dependency_name: dep_name.clone(),
                    source_kind,
                    hydrated_packages: graph.packages,
                    failure: None,
                });
            }
            Err(error) => {
                let (reason, action) = resolve_error_reason_and_action(source_kind, &error);
                let failure = DependencyResolutionFailure {
                    dependency_name: dep_name.clone(),
                    source_kind,
                    reason,
                    message: error.to_string(),
                    action,
                };
                emit_hydration_failure(manifest_root, &failure, emit_telemetry);
                if dep_trace {
                    tracing::info!(
                        manifest_root = %manifest_root.display(),
                        dependency = %dep_name,
                        source_kind,
                        reason = failure.reason,
                        error = %failure.message,
                        "dependency trace: dependency resolution failed"
                    );
                }
                outcomes.push(DependencyResolutionOutcome {
                    dependency_name: dep_name.clone(),
                    source_kind,
                    hydrated_packages: Vec::new(),
                    failure: Some(failure),
                });
            }
        }
    }
    if dep_trace {
        tracing::info!(
            manifest_root = %manifest_root.display(),
            dependency_count = outcomes.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "dependency trace: manifest dependency resolution complete"
        );
    }
    outcomes
}

pub fn hydrate_manifest_dependencies(
    manifest: &SysmlManifest,
    manifest_root: &Path,
    emit_telemetry: bool,
) -> DependencyHydrationReport {
    let outcomes = resolve_manifest_dependencies(manifest, manifest_root, emit_telemetry);
    let mut unique_packages = BTreeMap::new();
    let mut failures = Vec::new();

    for outcome in &outcomes {
        for package in &outcome.hydrated_packages {
            let key = format!("{}|{}", package.name, package.source.to_lock_source());
            unique_packages
                .entry(key)
                .or_insert_with(|| package.clone());
        }
        if let Some(failure) = &outcome.failure {
            failures.push(failure.clone());
        }
    }

    DependencyHydrationReport {
        outcomes,
        hydrated_packages: unique_packages.into_values().collect(),
        failures,
    }
}

fn normalize_workspace_members(ws: &sysml_manifest::WorkspaceConfig) -> Vec<String> {
    let excluded: HashSet<String> = ws.exclude.iter().cloned().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for member in &ws.members {
        if excluded.contains(member) {
            continue;
        }
        if seen.insert(member.clone()) {
            out.push(member.clone());
        }
    }
    out
}

fn load_dependency_projects(
    manifest: &SysmlManifest,
    manifest_root: &Path,
    include_stdlib: bool,
    emit_telemetry: bool,
) -> Vec<Project> {
    let dep_trace = telemetry_control::dependency_trace_enabled();
    let report = hydrate_manifest_dependencies(manifest, manifest_root, emit_telemetry);
    let mut projects = Vec::new();
    let mut next_offset: u32 = 5_000;
    for pkg in report.hydrated_packages {
        let pkg_source_kind = resolved_source_kind(&pkg.source);
        let dep_manifest_path = pkg.source_dir.join(sysml_manifest::MANIFEST_FILENAME);
        match sysml_manifest::load_manifest(&dep_manifest_path) {
            Ok(dep_manifest) => {
                let pid = ProjectHandle(FIRST_USER_PROJECT_ID + next_offset);
                next_offset += 1;
                let mut project = project_from_manifest(pid, &dep_manifest, &pkg.source_dir);
                project.info.description = project
                    .info
                    .description
                    .clone()
                    .or_else(|| Some(format!("Dependency package ({})", pkg.name)));
                tracing::info!(
                    name = %pkg.name,
                    version = %pkg.version,
                    source_kind = %pkg_source_kind,
                    root = %pkg.source_dir.display(),
                    include_stdlib,
                    "hydrated dependency project"
                );
                if dep_trace {
                    let canonical_root = pkg
                        .source_dir
                        .canonicalize()
                        .unwrap_or_else(|_| pkg.source_dir.clone());
                    tracing::info!(
                        dependency = %pkg.name,
                        source_kind = %pkg_source_kind,
                        root_raw = %pkg.source_dir.display(),
                        root_canonical = %canonical_root.display(),
                        "dependency trace: dependency project registered"
                    );
                }
                projects.push(project);
            }
            Err(e) => {
                if emit_telemetry {
                    telemetry_events::dependency_hydration_failure(
                        &manifest_root.display().to_string(),
                        &pkg.name,
                        pkg_source_kind,
                        "manifest_load_failed",
                    );
                }
                tracing::warn!(
                    dependency = %pkg.name,
                    source_kind = %pkg_source_kind,
                    manifest = %dep_manifest_path.display(),
                    error = %e,
                    "failed to load dependency manifest"
                );
            }
        }
    }

    projects
}

/// Create a [`Project`] from a [`SysmlManifest`] and its directory.
fn project_from_manifest(id: ProjectHandle, manifest: &SysmlManifest, dir: &Path) -> Project {
    Project {
        id,
        info: ProjectInfo {
            name: manifest.project.name.clone(),
            description: manifest.project.description.clone(),
            version: manifest.project.version.clone(),
            topic: Vec::new(),
            usage: Vec::new(),
        },
        meta: None,
        root: ProjectRoot::Directory(dir.to_path_buf()),
    }
}

/// Create an implicit [`Project`] when no manifest is found.
///
/// All `.sysml` files under `root` will be treated as belonging to this
/// project, enabling cross-file imports without any configuration.
fn implicit_project(id: ProjectHandle, root: &Path) -> Project {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed").to_owned();
    Project {
        id,
        info: ProjectInfo {
            name,
            description: None,
            version: "0.0.0".to_owned(),
            topic: Vec::new(),
            usage: Vec::new(),
        },
        meta: None,
        root: ProjectRoot::Directory(root.to_path_buf()),
    }
}

/// Discover projects in an LSP workspace root.
///
/// Handles five cases (checked in order):
/// 1. `.workspace.json` found → load all listed projects
/// 2. `.project.json` found → load single project
/// 3. `sysml.toml` with `[workspace]` → load workspace members
/// 4. `sysml.toml` (single project) → load project from manifest
/// 5. No manifest at all → create implicit project from workspace root
///
/// In all cases, `include_stdlib` is set to `true` so standard library
/// types are always available for resolution.
pub fn discover_lsp_workspace(
    root: &Path,
    include_stdlib_param: bool,
) -> sysml_project::Result<WorkspaceDiscovery> {
    discover_lsp_workspace_impl(root, include_stdlib_param, true)
}

/// Discover projects without emitting telemetry counters/events.
pub fn discover_lsp_workspace_silent(
    root: &Path,
    include_stdlib_param: bool,
) -> sysml_project::Result<WorkspaceDiscovery> {
    discover_lsp_workspace_impl(root, include_stdlib_param, false)
}

fn discover_lsp_workspace_impl(
    root: &Path,
    _include_stdlib: bool,
    emit_telemetry: bool,
) -> sysml_project::Result<WorkspaceDiscovery> {
    // Always enable stdlib — disabling it was a bug that caused false
    // positive diagnostics for standard library types.
    let include_stdlib = true;

    match discover_project(root) {
        DiscoveryResult::Workspace(ws_dir) => {
            let ws_path = ws_dir.join(".workspace.json");
            let ws_info = WorkspaceInfo::from_path(&ws_path)?;
            let mut projects = Vec::new();

            for (i, ws_proj) in ws_info.projects.iter().enumerate() {
                let proj_dir = ws_dir.join(&ws_proj.path);
                if !proj_dir.join(".project.json").exists() {
                    tracing::warn!(
                        path = %ws_proj.path,
                        "workspace lists project but .project.json not found, skipping"
                    );
                    continue;
                }
                let pid = ProjectHandle(FIRST_USER_PROJECT_ID + i as u32);
                match Project::from_directory(pid, &proj_dir) {
                    Ok(project) => projects.push(project),
                    Err(e) => {
                        tracing::warn!(
                            path = %proj_dir.display(),
                            error = %e,
                            "failed to load project listed in workspace, skipping"
                        );
                    }
                }
            }

            let desc = format!(
                "Workspace with {} project(s) (.workspace.json)",
                projects.len()
            );
            if emit_telemetry {
                telemetry_events::workspace_discovery_mode(
                    &ws_dir.display().to_string(),
                    "workspace_json",
                    projects.len(),
                    include_stdlib,
                );
            }
            Ok(WorkspaceDiscovery {
                projects,
                include_stdlib,
                discovery_description: desc,
                discovery_mode: "workspace_json",
            })
        }
        DiscoveryResult::Project(proj_dir) => {
            let pid = ProjectHandle(FIRST_USER_PROJECT_ID);
            let project = Project::from_directory(pid, &proj_dir)?;
            let desc = format!("Project '{}' (.project.json)", project.info.name);
            if emit_telemetry {
                telemetry_events::workspace_discovery_mode(
                    &proj_dir.display().to_string(),
                    "project_json",
                    1,
                    include_stdlib,
                );
            }
            Ok(WorkspaceDiscovery {
                projects: vec![project],
                include_stdlib,
                discovery_description: desc,
                discovery_mode: "project_json",
            })
        }
        DiscoveryResult::NotFound => {
            // Try sysml.toml before giving up
            discover_from_sysml_toml(root, include_stdlib, emit_telemetry)
        }
    }
}

/// Attempt discovery via `sysml.toml` manifest, falling back to implicit project.
fn discover_from_sysml_toml(
    root: &Path,
    include_stdlib: bool,
    emit_telemetry: bool,
) -> sysml_project::Result<WorkspaceDiscovery> {
    match sysml_manifest::find_manifest(root) {
        Ok(Some((manifest_path, manifest))) => {
            let proj_dir = manifest_path.parent().unwrap_or(root).to_path_buf();

            if manifest.is_workspace() {
                // sysml.toml workspace: discover member projects
                discover_sysml_workspace(&proj_dir, &manifest, include_stdlib, emit_telemetry)
            } else {
                // Single sysml.toml project
                let pid = ProjectHandle(FIRST_USER_PROJECT_ID);
                let project = project_from_manifest(pid, &manifest, &proj_dir);
                let mut projects = vec![project];
                projects.extend(load_dependency_projects(
                    &manifest,
                    &proj_dir,
                    include_stdlib,
                    emit_telemetry,
                ));
                let desc = format!(
                    "Project '{}' (sysml.toml, {} dependency project(s))",
                    manifest.project.name,
                    projects.len().saturating_sub(1)
                );
                tracing::info!(
                    name = %manifest.project.name,
                    version = %manifest.project.version,
                    root = %proj_dir.display(),
                    "discovered SysML project from sysml.toml"
                );
                if emit_telemetry {
                    telemetry_events::workspace_discovery_mode(
                        &proj_dir.display().to_string(),
                        "sysml_project",
                        projects.len(),
                        include_stdlib,
                    );
                }
                Ok(WorkspaceDiscovery {
                    projects,
                    include_stdlib,
                    discovery_description: desc,
                    discovery_mode: "sysml_project",
                })
            }
        }
        Ok(None) => {
            // No manifest at all — create an implicit project so cross-file
            // imports still work (like TypeScript without tsconfig.json).
            let pid = ProjectHandle(FIRST_USER_PROJECT_ID);
            let project = implicit_project(pid, root);
            let desc = format!(
                "Implicit project '{}' (no manifest found)",
                project.info.name
            );
            tracing::info!(
                root = %root.display(),
                "no project manifest found, creating implicit project for cross-file resolution"
            );
            if emit_telemetry {
                telemetry_events::workspace_discovery_mode(
                    &root.display().to_string(),
                    "implicit",
                    1,
                    include_stdlib,
                );
            }
            Ok(WorkspaceDiscovery {
                projects: vec![project],
                include_stdlib,
                discovery_description: desc,
                discovery_mode: "implicit",
            })
        }
        Err(e) => {
            tracing::warn!(
                root = %root.display(),
                error = %e,
                "failed to search for sysml.toml, falling back to implicit project"
            );
            // Don't fail discovery — fall back to implicit project
            let pid = ProjectHandle(FIRST_USER_PROJECT_ID);
            let project = implicit_project(pid, root);
            let desc = format!(
                "Implicit project '{}' (manifest search failed: {})",
                project.info.name, e
            );
            if emit_telemetry {
                telemetry_events::workspace_discovery_mode(
                    &root.display().to_string(),
                    "implicit_manifest_search_failed",
                    1,
                    include_stdlib,
                );
            }
            Ok(WorkspaceDiscovery {
                projects: vec![project],
                include_stdlib,
                discovery_description: desc,
                discovery_mode: "implicit_manifest_search_failed",
            })
        }
    }
}

/// Discover workspace members from a `sysml.toml` with `[workspace]`.
fn discover_sysml_workspace(
    ws_dir: &Path,
    manifest: &SysmlManifest,
    include_stdlib: bool,
    emit_telemetry: bool,
) -> sysml_project::Result<WorkspaceDiscovery> {
    let ws_config = manifest.workspace.as_ref().expect("checked is_workspace");
    let normalized_members = normalize_workspace_members(ws_config);
    let excluded_members = ws_config
        .members
        .len()
        .saturating_sub(normalized_members.len());
    let mut projects = Vec::new();
    let mut missing_members = 0usize;
    let mut dependency_projects =
        load_dependency_projects(manifest, ws_dir, include_stdlib, emit_telemetry);

    for (i, member_path) in normalized_members.iter().enumerate() {
        let member_dir = ws_dir.join(member_path);
        let member_manifest_path = member_dir.join(sysml_manifest::MANIFEST_FILENAME);

        if !member_dir.exists() {
            missing_members += 1;
            tracing::warn!(
                member = %member_path,
                "workspace member directory not found, skipping"
            );
            continue;
        }

        let pid = ProjectHandle(FIRST_USER_PROJECT_ID + i as u32);

        if member_manifest_path.is_file() {
            // Member has its own sysml.toml
            match sysml_manifest::load_manifest(&member_manifest_path) {
                Ok(member_manifest) => {
                    let project = project_from_manifest(pid, &member_manifest, &member_dir);
                    tracing::info!(
                        name = %member_manifest.project.name,
                        member = %member_path,
                        "discovered workspace member from sysml.toml"
                    );
                    projects.push(project);
                    dependency_projects.extend(load_dependency_projects(
                        &member_manifest,
                        &member_dir,
                        include_stdlib,
                        emit_telemetry,
                    ));
                }
                Err(e) => {
                    tracing::warn!(
                        member = %member_path,
                        error = %e,
                        "failed to load workspace member manifest, skipping"
                    );
                }
            }
        } else {
            // Member directory exists but has no manifest — create implicit project
            let project = implicit_project(pid, &member_dir);
            tracing::info!(
                member = %member_path,
                "workspace member has no sysml.toml, using implicit project"
            );
            projects.push(project);
        }
    }

    // Add hydrated dependency projects after members and deduplicate by root dir.
    projects.extend(dependency_projects);
    let mut seen_roots = HashSet::new();
    projects.retain(|project| {
        let key = match &project.root {
            ProjectRoot::Directory(dir) => dir
                .canonicalize()
                .unwrap_or_else(|_| dir.clone())
                .display()
                .to_string(),
            _ => format!("in-memory:{}", project.id.0),
        };
        seen_roots.insert(key)
    });

    let desc = format!(
        "Workspace '{}' with {} member(s), {} excluded, {} dependency project(s) (sysml.toml)",
        manifest.project.name,
        normalized_members.len().saturating_sub(missing_members),
        excluded_members,
        projects
            .len()
            .saturating_sub(normalized_members.len().saturating_sub(missing_members))
    );
    tracing::info!(
        name = %manifest.project.name,
        members = projects.len(),
        root = %ws_dir.display(),
        "discovered SysML workspace from sysml.toml"
    );
    if emit_telemetry {
        telemetry_events::workspace_discovery_mode(
            &ws_dir.display().to_string(),
            "sysml_workspace",
            projects.len(),
            include_stdlib,
        );
        telemetry_events::workspace_member_expansion(
            &ws_dir.display().to_string(),
            normalized_members.len(),
            normalized_members.len().saturating_sub(missing_members),
            missing_members,
        );
    }
    Ok(WorkspaceDiscovery {
        projects,
        include_stdlib,
        discovery_description: desc,
        discovery_mode: "sysml_workspace",
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use directories::{BaseDirs, ProjectDirs};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
    use tempfile::TempDir;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {}: {e}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed in {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_git_fixture(root: &Path, name: &str, version: &str) -> (String, String) {
        let repo_dir = root.join(format!("{name}-repo"));
        std::fs::create_dir_all(&repo_dir).unwrap();
        git(&repo_dir, &["init", "--initial-branch", "main"]);
        git(&repo_dir, &["config", "user.email", "tests@sysml.rs"]);
        git(&repo_dir, &["config", "user.name", "SysML LSP Tests"]);
        std::fs::write(
            repo_dir.join("sysml.toml"),
            format!(
                r#"
[project]
name = "{name}"
version = "{version}"
"#
            ),
        )
        .unwrap();
        std::fs::write(repo_dir.join("dep.sysml"), "package Dep { part def X; }\n").unwrap();
        git(&repo_dir, &["add", "sysml.toml", "dep.sysml"]);
        git(&repo_dir, &["commit", "-m", "initial"]);
        let commit = git(&repo_dir, &["rev-parse", "HEAD"]);
        (format!("file://{}", repo_dir.display()), commit)
    }

    fn create_kpar_archive(root: &Path, name: &str, version: &str) -> PathBuf {
        let mut metadata = ProjectMetadata::new();
        metadata.add_index_entry("Root", "Root.sysml");
        let archive = KparArchive {
            root_dir: name.to_string(),
            project_info: ProjectInfo {
                name: name.to_string(),
                version: version.to_string(),
                description: Some("project discovery test archive".to_string()),
                license: Some("MIT".to_string()),
                usage: Vec::new(),
            },
            metadata,
            source_files: vec![(
                "Root.sysml".to_string(),
                b"package Root { part def X; }\n".to_vec(),
            )],
        };
        let archive_path = root.join(format!("{name}.kpar"));
        write_kpar(&archive_path, &archive).unwrap();
        archive_path
    }

    fn write_sysand_index(root: &Path, package: &str, version: &str, artifact_path: &Path) {
        let index_dir = root.join(".sysml/registries/sysand");
        std::fs::create_dir_all(&index_dir).unwrap();
        let checksum = format!("sha256:{}", sha256_hex_file(artifact_path));
        std::fs::write(
            index_dir.join("index.json"),
            format!(
                "{{\"packages\":{{\"{package}\":{{\"{version}\":{{\"artifact\":\"{}\",\"checksum\":\"{checksum}\"}}}}}}}}",
                artifact_path.display()
            ),
        )
        .unwrap();
    }

    fn sha256_hex_file(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn clean_registry_cache_for_request(backend: &str, package: &str, requirement: &str) {
        let cache_dir = registry_cache_dir_for_request(backend, package, requirement);
        let _ = std::fs::remove_dir_all(cache_dir);
    }

    fn registry_cache_dir_for_request(backend: &str, package: &str, requirement: &str) -> PathBuf {
        let request_key = format!("{backend}:{package}@{requirement}");
        cache_root()
            .join("dependencies")
            .join("registry")
            .join(backend)
            .join(source_hash(&request_key))
    }

    fn cache_root() -> PathBuf {
        if let Some(project_dirs) = ProjectDirs::from("", "", "sysml-rs") {
            return project_dirs.cache_dir().to_path_buf();
        }

        if let Some(base_dirs) = BaseDirs::new() {
            return base_dirs.cache_dir().join("sysml-rs");
        }

        PathBuf::from("/tmp/sysml-rs-cache")
    }

    fn source_hash(source: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(source.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn discover_empty_workspace_creates_implicit_project() {
        let dir = TempDir::new().unwrap();
        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        // Now creates an implicit project so cross-file resolution works
        assert_eq!(result.projects.len(), 1);
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains("Implicit project"));
    }

    #[test]
    fn discover_single_project_json() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".project.json"),
            r#"{"name":"TestProject","version":"1.0.0"}"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "TestProject");
        assert_eq!(result.projects[0].id, ProjectHandle(FIRST_USER_PROJECT_ID));
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains(".project.json"));
    }

    #[test]
    fn discover_workspace_json_with_projects() {
        let dir = TempDir::new().unwrap();

        // Create workspace.json
        std::fs::write(
            dir.path().join(".workspace.json"),
            r#"{"projects":[{"path":"proj-a","iris":[]},{"path":"proj-b","iris":[]}]}"#,
        )
        .unwrap();

        // Create project directories
        let proj_a = dir.path().join("proj-a");
        std::fs::create_dir(&proj_a).unwrap();
        std::fs::write(
            proj_a.join(".project.json"),
            r#"{"name":"ProjA","version":"1.0.0"}"#,
        )
        .unwrap();

        let proj_b = dir.path().join("proj-b");
        std::fs::create_dir(&proj_b).unwrap();
        std::fs::write(
            proj_b.join(".project.json"),
            r#"{"name":"ProjB","version":"2.0.0"}"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 2);
        assert_eq!(result.projects[0].info.name, "ProjA");
        assert_eq!(result.projects[1].info.name, "ProjB");
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains(".workspace.json"));
    }

    #[test]
    fn discover_workspace_skips_missing_projects() {
        let dir = TempDir::new().unwrap();

        std::fs::write(
            dir.path().join(".workspace.json"),
            r#"{"projects":[{"path":"exists","iris":[]},{"path":"missing","iris":[]}]}"#,
        )
        .unwrap();

        let exists = dir.path().join("exists");
        std::fs::create_dir(&exists).unwrap();
        std::fs::write(
            exists.join(".project.json"),
            r#"{"name":"Exists","version":"1.0.0"}"#,
        )
        .unwrap();

        // "missing" directory doesn't exist
        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "Exists");
    }

    #[test]
    fn discover_sysml_toml_single_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "coffee-machine"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "coffee-machine");
        assert_eq!(result.projects[0].id, ProjectHandle(FIRST_USER_PROJECT_ID));
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains("sysml.toml"));
        assert!(result.discovery_description.contains("coffee-machine"));
    }

    #[test]
    fn discover_sysml_toml_workspace() {
        let dir = TempDir::new().unwrap();

        // Root workspace manifest
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "my-workspace"
version = "0.1.0"

[workspace]
members = ["lib-a", "lib-b"]
"#,
        )
        .unwrap();

        // Member with its own manifest
        let lib_a = dir.path().join("lib-a");
        std::fs::create_dir(&lib_a).unwrap();
        std::fs::write(
            lib_a.join("sysml.toml"),
            r#"
[project]
name = "lib-a"
version = "0.1.0"
"#,
        )
        .unwrap();

        // Member without manifest (implicit)
        let lib_b = dir.path().join("lib-b");
        std::fs::create_dir(&lib_b).unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 2);
        assert_eq!(result.projects[0].info.name, "lib-a");
        // lib-b has no manifest, so it gets an implicit project named after the dir
        assert_eq!(result.projects[1].info.name, "lib-b");
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains("Workspace"));
    }

    #[test]
    fn discover_sysml_workspace_applies_exclude() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "my-workspace"
version = "0.1.0"

[workspace]
members = ["lib-a", "lib-b"]
exclude = ["lib-b"]
"#,
        )
        .unwrap();

        let lib_a = dir.path().join("lib-a");
        std::fs::create_dir(&lib_a).unwrap();
        std::fs::write(
            lib_a.join("sysml.toml"),
            r#"
[project]
name = "lib-a"
version = "0.1.0"
"#,
        )
        .unwrap();

        let lib_b = dir.path().join("lib-b");
        std::fs::create_dir(&lib_b).unwrap();
        std::fs::write(
            lib_b.join("sysml.toml"),
            r#"
[project]
name = "lib-b"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "lib-a");
    }

    #[test]
    fn discover_sysml_project_hydrates_path_dependencies() {
        let dir = TempDir::new().unwrap();
        let dep_dir = dir.path().join("dep-lib");
        std::fs::create_dir(&dep_dir).unwrap();

        std::fs::write(
            dep_dir.join("sysml.toml"),
            r#"
[project]
name = "dep-lib"
version = "0.2.0"
"#,
        )
        .unwrap();

        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-lib = { path = "./dep-lib" }
"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert!(
            result
                .projects
                .iter()
                .any(|p| p.info.name == "root-project"),
            "expected root project in discovery output"
        );
        assert!(
            result.projects.iter().any(|p| p.info.name == "dep-lib"),
            "expected hydrated dependency project in discovery output"
        );
    }

    #[test]
    fn discover_sysml_project_hydrates_git_and_kpar_dependencies() {
        if !git_available() {
            eprintln!("skipping git+kpar hydration test: git binary unavailable");
            return;
        }

        let dir = TempDir::new().unwrap();
        let (git_url, commit) = create_git_fixture(dir.path(), "dep-git", "1.1.0");
        create_kpar_archive(dir.path(), "dep-kpar", "2.0.0");

        std::fs::write(
            dir.path().join("sysml.toml"),
            format!(
                r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-git = {{ git = "{git_url}", rev = "{commit}" }}
dep-kpar = {{ kpar = "./dep-kpar.kpar" }}
"#
            ),
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert!(
            result
                .projects
                .iter()
                .any(|project| project.info.name == "root-project"),
            "expected root project to be discovered"
        );
        assert!(
            result
                .projects
                .iter()
                .any(|project| project.info.name == "dep-git"),
            "expected git dependency project to be hydrated"
        );
        assert!(
            result
                .projects
                .iter()
                .any(|project| project.info.name == "dep-kpar"),
            "expected kpar dependency project to be hydrated"
        );
    }

    #[test]
    fn discover_sysml_project_keeps_working_when_some_dependencies_fail() {
        let dir = TempDir::new().unwrap();
        let dep_dir = dir.path().join("dep-lib");
        std::fs::create_dir(&dep_dir).unwrap();
        std::fs::write(
            dep_dir.join("sysml.toml"),
            r#"
[project]
name = "dep-lib"
version = "0.2.0"
"#,
        )
        .unwrap();

        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
dep-lib = { path = "./dep-lib" }
missing-lib = { path = "./missing-lib" }
registry-lib = "1.0.0"
"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert!(
            result
                .projects
                .iter()
                .any(|project| project.info.name == "root-project"),
            "expected root project to be discovered even with failing dependencies"
        );
        assert!(
            result
                .projects
                .iter()
                .any(|project| project.info.name == "dep-lib"),
            "expected valid dependency project to be hydrated"
        );
    }

    #[test]
    fn resolve_manifest_dependencies_reports_source_aware_failures() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
missing-lib = { path = "./missing-lib" }
registry-lib = "1.0.0"
"#,
        )
        .unwrap();

        let manifest = sysml_manifest::load_manifest(&dir.path().join("sysml.toml")).unwrap();
        let outcomes = resolve_manifest_dependencies(&manifest, dir.path(), false);

        let missing = outcomes
            .iter()
            .find(|outcome| outcome.dependency_name == "missing-lib")
            .and_then(|outcome| outcome.failure.as_ref())
            .expect("missing-lib should report failure");
        assert_eq!(missing.source_kind, "path");
        assert_eq!(missing.reason, "missing_dependency");
        assert!(
            missing.action.contains("path or source URL"),
            "missing dependency action should be actionable: {}",
            missing.action
        );

        let registry = outcomes
            .iter()
            .find(|outcome| outcome.dependency_name == "registry-lib")
            .and_then(|outcome| outcome.failure.as_ref())
            .expect("registry-lib should report failure");
        assert_eq!(registry.source_kind, "registry");
        assert_eq!(registry.reason, "unsupported_source");
        assert!(
            registry.action.contains("SYSML_REGISTRY_SYSAND_INDEX")
                || registry.action.contains(".sysml/registries/sysand"),
            "registry failure action should mention Sysand index configuration: {}",
            registry.action
        );
    }

    #[test]
    fn resolve_manifest_dependencies_hydrates_registry_dependency_when_index_present() {
        let dir = TempDir::new().unwrap();
        let package = "registry-hydrate-ok";
        let version = "9.8.7";
        let archive = create_kpar_archive(dir.path(), package, version);
        write_sysand_index(dir.path(), package, version, &archive);
        clean_registry_cache_for_request("sysand", package, version);

        std::fs::write(
            dir.path().join("sysml.toml"),
            format!(
                r#"
[project]
name = "root-project"
version = "0.1.0"

[dependencies]
{package} = "{version}"
"#
            ),
        )
        .unwrap();

        let manifest = sysml_manifest::load_manifest(&dir.path().join("sysml.toml")).unwrap();
        let outcomes = resolve_manifest_dependencies(&manifest, dir.path(), false);
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome.dependency_name == package)
            .expect("registry dependency should have outcome");
        assert!(
            outcome.failure.is_none(),
            "registry dependency should hydrate successfully when index exists: {:?}",
            outcome.failure
        );
        assert_eq!(outcome.hydrated_packages.len(), 1);
        assert!(matches!(
            outcome.hydrated_packages[0].source,
            PackageSource::Registry { .. }
        ));

        clean_registry_cache_for_request("sysand", package, version);
    }

    #[test]
    fn implicit_project_from_bare_directory() {
        let dir = TempDir::new().unwrap();
        // Just .sysml files, no manifest at all
        std::fs::write(dir.path().join("test.sysml"), "package Test;").unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        // Should create an implicit project (not empty!)
        assert_eq!(result.projects.len(), 1);
        assert!(result.include_stdlib);
        assert!(result.discovery_description.contains("Implicit"));
    }

    #[test]
    fn stdlib_always_enabled() {
        let dir = TempDir::new().unwrap();
        // Even when caller passes false, stdlib should still be enabled
        let result = discover_lsp_workspace(dir.path(), false).unwrap();
        assert!(result.include_stdlib);
    }

    #[test]
    fn sysml_toml_takes_precedence_over_bare() {
        let dir = TempDir::new().unwrap();
        // sysml.toml present → should use it, not create implicit project
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "explicit-project"
version = "1.0.0"
"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("model.sysml"), "package Model;").unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "explicit-project");
        assert!(result.discovery_description.contains("sysml.toml"));
    }

    #[test]
    fn project_json_takes_precedence_over_sysml_toml() {
        let dir = TempDir::new().unwrap();
        // Both .project.json and sysml.toml present → .project.json wins
        std::fs::write(
            dir.path().join(".project.json"),
            r#"{"name":"FromJson","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("sysml.toml"),
            r#"
[project]
name = "from-toml"
version = "1.0.0"
"#,
        )
        .unwrap();

        let result = discover_lsp_workspace(dir.path(), true).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].info.name, "FromJson");
    }
}
