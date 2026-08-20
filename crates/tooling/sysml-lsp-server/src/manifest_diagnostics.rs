//! Manifest diagnostics for `sysml.toml` files.
//!
//! Parses manifest content and produces LSP diagnostics for:
//! - TOML syntax errors
//! - Invalid semver versions
//! - Git dependencies without a pinned ref
//! - Conflicting dependency sources
//! - Unknown SysML edition values
//! - Invalid SPDX license identifiers
//! - Empty dependency sections

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

use std::collections::HashMap;
use std::path::Path;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

use sysml_service::project_discovery;
use crate::telemetry_events;
use crate::utils::parse_uri;
use sysml_manifest::{Dependency, StdlibConfig, SysmlManifest};
use sysml_resolve::{self, PackageSource};

/// Allowed SPDX license identifiers (small allowlist).
const KNOWN_SPDX: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "GPL-2.0-only",
    "GPL-3.0-only",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "MPL-2.0",
    "LGPL-2.1-only",
    "LGPL-3.0-only",
    "Unlicense",
    "AGPL-3.0-only",
    "0BSD",
];

/// Known SysML edition values.
const KNOWN_EDITIONS: &[&str] = &["2024", "2025"];

const TOP_LEVEL_KEYS: &[&str] = &["project", "package", "stdlib", "dependencies", "workspace"];
const PROJECT_KEYS: &[&str] = &[
    "name",
    "version",
    "description",
    "license",
    "sysml-edition",
    "authors",
];
const PACKAGE_KEYS: &[&str] = &["iri"];
const STDLIB_KEYS: &[&str] = &["include_only", "exclude"];
const WORKSPACE_KEYS: &[&str] = &["members", "exclude", "default-members", "project"];
const WORKSPACE_PROJECT_KEYS: &[&str] = &["sysml-edition", "license", "version"];
const DEP_DETAIL_KEYS: &[&str] = &["path", "git", "tag", "branch", "rev", "kpar"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDependencyChecks {
    Enabled,
    Disabled,
}

/// Validate manifest content and return LSP diagnostics.
#[cfg(test)]
pub fn validate_manifest(content: &str) -> Vec<Diagnostic> {
    validate_manifest_with_context(content, None)
}

/// Validate manifest content and return LSP diagnostics with optional URI context.
pub fn validate_manifest_with_context(
    content: &str,
    document_uri: Option<&str>,
) -> Vec<Diagnostic> {
    validate_manifest_with_context_mode(content, document_uri, RuntimeDependencyChecks::Enabled)
}

/// Validate manifest content for live editing paths.
///
/// This mode skips runtime dependency hydration checks so keystroke-level
/// diagnostics remain responsive. Runtime dependency failures are still
/// available on open and explicit refresh flows.
pub fn validate_manifest_for_live_edit(
    content: &str,
    document_uri: Option<&str>,
) -> Vec<Diagnostic> {
    validate_manifest_with_context_mode(content, document_uri, RuntimeDependencyChecks::Disabled)
}

fn validate_manifest_with_context_mode(
    content: &str,
    document_uri: Option<&str>,
    runtime_dependency_checks: RuntimeDependencyChecks,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let manifest_dir = document_uri
        .and_then(parse_uri)
        .and_then(|url| url.to_file_path().ok())
        .and_then(|path| path.parent().map(|p| p.to_path_buf()));

    if let Ok(value) = toml::from_str::<toml::Value>(content) {
        check_unknown_keys(&mut diagnostics, content, &value);
    }

    match toml::from_str::<SysmlManifest>(content) {
        Err(e) => {
            let message = format!("failed to parse sysml.toml: {}", e.message());
            telemetry_events::manifest_parse_error(document_uri.unwrap_or("<unknown>"), &message);
            let range = find_error_range(content, &message);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M000".to_owned())),
                message,
                ..Default::default()
            });
        }
        Ok(manifest) => {
            // Check semver validity
            if !is_valid_semver(&manifest.project.version) {
                let range = find_key_value_range(content, "version");
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M010".to_owned())),
                    message: format!("invalid semver version: '{}'", manifest.project.version),
                    ..Default::default()
                });
            }

            // Check empty project name
            if manifest.project.name.is_empty() {
                let range = find_key_value_range(content, "name");
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M011".to_owned())),
                    message: "project name must not be empty".to_owned(),
                    ..Default::default()
                });
            }

            // Check sysml-edition
            if !KNOWN_EDITIONS.contains(&manifest.project.sysml_edition.as_str()) {
                let range = find_key_value_range(content, "sysml-edition");
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M012".to_owned())),
                    message: format!(
                        "unknown sysml-edition '{}'; expected one of: {}",
                        manifest.project.sysml_edition,
                        KNOWN_EDITIONS.join(", ")
                    ),
                    ..Default::default()
                });
            }

            // Check license
            if let Some(ref license) = manifest.project.license {
                let license_str: &str = license;
                if !KNOWN_SPDX.contains(&license_str) {
                    let range = find_key_value_range(content, "license");
                    diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("sysml-manifest".to_owned()),
                        code: Some(NumberOrString::String("M013".to_owned())),
                        message: format!("unknown SPDX license identifier: '{}'", license),
                        ..Default::default()
                    });
                }
            }

            // Check dependencies
            let runtime_outcomes = if runtime_dependency_checks == RuntimeDependencyChecks::Enabled
            {
                manifest_dir.as_deref().map(|root| {
                    project_discovery::resolve_manifest_dependencies(&manifest, root, false)
                })
            } else {
                None
            };
            let runtime_outcomes_by_name: HashMap<
                &str,
                &project_discovery::DependencyResolutionOutcome,
            > = runtime_outcomes
                .as_ref()
                .map(|outcomes| {
                    outcomes
                        .iter()
                        .map(|outcome| (outcome.dependency_name.as_str(), outcome))
                        .collect()
                })
                .unwrap_or_default();

            for (name, dep) in &manifest.dependencies {
                check_dependency(
                    &mut diagnostics,
                    content,
                    name,
                    dep,
                    manifest_dir.as_deref(),
                );
                if let Some(root) = manifest_dir.as_deref() {
                    check_dependency_runtime(
                        &mut diagnostics,
                        content,
                        name,
                        dep,
                        runtime_outcomes_by_name.get(name.as_str()).copied(),
                        root,
                    );
                }
            }

            // Check for empty dependencies section
            if content.contains("[dependencies]") && manifest.dependencies.is_empty() {
                let range = find_section_range(content, "[dependencies]");
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::HINT),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M014".to_owned())),
                    message: "dependencies section is empty".to_owned(),
                    ..Default::default()
                });
            }

            check_workspace_semantics(
                &mut diagnostics,
                content,
                &manifest,
                manifest_dir.as_deref(),
            );
            check_stdlib_semantics(&mut diagnostics, content, &manifest);
        }
    }

    diagnostics
}

fn check_stdlib_semantics(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    manifest: &SysmlManifest,
) {
    let stdlib = manifest.effective_stdlib();
    let known = StdlibConfig::known_library_names();

    check_stdlib_list_entries(
        diagnostics,
        content,
        "include_only",
        &stdlib.include_only,
        known,
        false,
    );
    check_stdlib_list_entries(
        diagnostics,
        content,
        "exclude",
        &stdlib.exclude,
        known,
        true,
    );

    let include: std::collections::HashSet<&str> =
        stdlib.include_only.iter().map(String::as_str).collect();
    let exclude: std::collections::HashSet<&str> = stdlib
        .exclude
        .iter()
        .filter_map(|entry| {
            if entry == "*" || entry.eq_ignore_ascii_case("all") {
                None
            } else {
                Some(entry.as_str())
            }
        })
        .collect();

    for overlap in include.intersection(&exclude) {
        diagnostics.push(Diagnostic {
            range: find_key_value_range(content, "exclude"),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("sysml-manifest".to_owned()),
            code: Some(NumberOrString::String("M052".to_owned())),
            data: Some(serde_json::json!({
                "kind": "stdlib_include_exclude_overlap",
                "library": overlap,
            })),
            message: format!(
                "stdlib library '{}' appears in both include_only and exclude; it will be excluded",
                overlap
            ),
            ..Default::default()
        });
    }
}

fn check_stdlib_list_entries(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    key: &str,
    entries: &[String],
    known: &[&str],
    allow_all_wildcard: bool,
) {
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        if !seen.insert(entry.as_str()) {
            diagnostics.push(Diagnostic {
                range: find_key_value_range(content, key),
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M051".to_owned())),
                data: Some(serde_json::json!({
                    "kind": "duplicate_stdlib_entry",
                    "key": key,
                    "value": entry,
                })),
                message: format!("duplicate '{}' entry in [stdlib].{}", entry, key),
                ..Default::default()
            });
            continue;
        }

        if allow_all_wildcard && (entry == "*" || entry.eq_ignore_ascii_case("all")) {
            continue;
        }

        if known.contains(&entry.as_str()) {
            continue;
        }

        let suggestion = nearest_known_key(entry, known).map(str::to_string);
        let message = match &suggestion {
            Some(s) => format!(
                "unknown stdlib library '{}' in [stdlib].{}; did you mean '{}'?",
                entry, key, s
            ),
            None => format!("unknown stdlib library '{}' in [stdlib].{}", entry, key),
        };

        diagnostics.push(Diagnostic {
            range: find_key_value_range(content, key),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("sysml-manifest".to_owned()),
            code: Some(NumberOrString::String("M050".to_owned())),
            data: Some(serde_json::json!({
                "kind": "unknown_stdlib_library",
                "key": key,
                "value": entry,
                "suggestion": suggestion,
            })),
            message,
            ..Default::default()
        });
    }
}

/// Check a single dependency for issues.
fn check_dependency(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    name: &str,
    dep: &Dependency,
    manifest_dir: Option<&std::path::Path>,
) {
    if let Dependency::Detailed(d) = dep {
        // Git dep without pinned ref
        if d.git.is_some() && d.tag.is_none() && d.branch.is_none() && d.rev.is_none() {
            let range = find_dep_name_range(content, name);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M020".to_owned())),
                data: Some(serde_json::json!({
                    "kind": "unpinned_git_dependency",
                    "dependency": name,
                })),
                message: format!(
                    "git dependency '{}' has no tag, branch, or rev; builds may not be reproducible",
                    name
                ),
                ..Default::default()
            });
        }

        // Conflicting sources: path + git
        if d.path.is_some() && d.git.is_some() {
            let range = find_dep_name_range(content, name);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M021".to_owned())),
                message: format!(
                    "dependency '{}' has both 'path' and 'git' sources; only one should be specified",
                    name
                ),
                ..Default::default()
            });
        }

        // Conflicting sources: path + kpar
        if d.path.is_some() && d.kpar.is_some() {
            let range = find_dep_name_range(content, name);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M022".to_owned())),
                message: format!(
                    "dependency '{}' has both 'path' and 'kpar' sources; only one should be specified",
                    name
                ),
                ..Default::default()
            });
        }

        if let (Some(path), Some(root)) = (&d.path, manifest_dir) {
            let dep_path = root.join(path);
            if !dep_path.exists() {
                let range = find_dep_name_range(content, name);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M023".to_owned())),
                    data: Some(serde_json::json!({
                        "kind": "missing_dependency_path",
                        "dependency": name,
                        "path": path,
                    })),
                    message: format!(
                        "path dependency '{}' points to '{}', but that path does not exist in this workspace",
                        name, path
                    ),
                    ..Default::default()
                });
            }
        }
    }
}

fn check_dependency_runtime(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    name: &str,
    dep: &Dependency,
    outcome: Option<&project_discovery::DependencyResolutionOutcome>,
    manifest_root: &Path,
) {
    let Some(outcome) = outcome else {
        return;
    };

    if let Some(failure) = &outcome.failure {
        let severity = if failure.reason == "unsupported_source" {
            DiagnosticSeverity::WARNING
        } else {
            DiagnosticSeverity::ERROR
        };
        let concise_message = concise_dependency_failure_message(&failure.message);
        diagnostics.push(Diagnostic {
            range: find_dep_name_range(content, name),
            severity: Some(severity),
            source: Some("sysml-manifest".to_owned()),
            code: Some(NumberOrString::String("M040".to_owned())),
            data: Some(serde_json::json!({
                "kind": "dependency_resolution_failure",
                "dependency": name,
                "source": failure.source_kind,
                "reason": failure.reason,
                "action": failure.action,
            })),
            message: format!(
                "dependency '{}' ({}) failed to resolve: {}",
                name, failure.source_kind, concise_message
            ),
            ..Default::default()
        });
        return;
    }

    if let Some(hint) = dependency_update_hint(name, dep, outcome, manifest_root) {
        diagnostics.push(Diagnostic {
            range: find_dep_name_range(content, name),
            severity: Some(DiagnosticSeverity::HINT),
            source: Some("sysml-manifest".to_owned()),
            code: Some(NumberOrString::String("M041".to_owned())),
            data: Some(serde_json::json!({
                "kind": "dependency_update_available",
                "dependency": name,
                "source": "registry",
                "backend": hint.backend,
                "package": hint.package,
                "requested_requirement": hint.requested_requirement,
                "resolved_version": hint.resolved_version,
                "latest_version": hint.latest_version,
                "action": hint.action,
            })),
            message: hint.message,
            ..Default::default()
        });
    }
}

fn concise_dependency_failure_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return "unknown dependency resolution error".to_owned();
    }

    let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();
    if first_line.len() <= 220 {
        return first_line.to_owned();
    }

    let mut truncated = first_line.chars().take(217).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[derive(Debug, Clone)]
struct DependencyUpdateHint {
    backend: String,
    package: String,
    requested_requirement: String,
    resolved_version: String,
    latest_version: String,
    action: String,
    message: String,
}

fn dependency_update_hint(
    dep_name: &str,
    _dep: &Dependency,
    outcome: &project_discovery::DependencyResolutionOutcome,
    manifest_root: &Path,
) -> Option<DependencyUpdateHint> {
    let mut direct_registry: Option<(&str, &str, &str, &str)> = None;
    for pkg in &outcome.hydrated_packages {
        if let PackageSource::Registry {
            backend,
            package,
            requested,
            version,
        } = &pkg.source
        {
            if package == dep_name {
                direct_registry = Some((
                    backend.as_str(),
                    package.as_str(),
                    requested.as_str(),
                    version.as_str(),
                ));
                break;
            }
        }
    }

    let (backend, package, requested_requirement, resolved_version) = direct_registry?;
    if !allow_offline_registry_update_check(backend, manifest_root) {
        return None;
    }

    let latest = match sysml_resolve::resolve_latest_registry_release_metadata(
        backend,
        package,
        manifest_root,
    ) {
        Ok(latest) => latest,
        Err(_) => return None,
    };

    let current_version = semver::Version::parse(resolved_version).ok()?;
    let latest_version = semver::Version::parse(&latest.resolved_version).ok()?;
    if latest_version <= current_version {
        return None;
    }

    let action = if semver::Version::parse(requested_requirement).is_ok() {
        format!(
            "Update '{}' from '{}' to '{}' in `sysml.toml`, then run `sysml update`.",
            dep_name, resolved_version, latest.resolved_version
        )
    } else {
        format!(
            "Widen the version requirement '{}' for '{}' if you want '{}', then run `sysml update`.",
            requested_requirement, dep_name, latest.resolved_version
        )
    };

    Some(DependencyUpdateHint {
        backend: backend.to_owned(),
        package: package.to_owned(),
        requested_requirement: requested_requirement.to_owned(),
        resolved_version: resolved_version.to_owned(),
        latest_version: latest.resolved_version.clone(),
        action: action.clone(),
        message: format!(
            "registry dependency '{}' resolved to {} (requested '{}'); newer release {} is available. {}",
            dep_name, resolved_version, requested_requirement, latest.resolved_version, action
        ),
    })
}

fn allow_offline_registry_update_check(backend: &str, manifest_root: &Path) -> bool {
    if backend != "sysand" {
        return false;
    }

    match std::env::var("SYSML_REGISTRY_SYSAND_INDEX") {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                manifest_root
                    .join(".sysml/registries/sysand/index.json")
                    .exists()
            } else {
                !trimmed.starts_with("http://") && !trimmed.starts_with("https://")
            }
        }
        Err(_) => manifest_root
            .join(".sysml/registries/sysand/index.json")
            .exists(),
    }
}

fn check_unknown_keys(diagnostics: &mut Vec<Diagnostic>, content: &str, value: &toml::Value) {
    let Some(root) = value.as_table() else {
        return;
    };

    for key in root.keys() {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            push_unknown_key_diagnostic(diagnostics, content, None, key, TOP_LEVEL_KEYS);
        }
    }

    if let Some(project) = root.get("project").and_then(|v| v.as_table()) {
        check_table_unknown_keys(diagnostics, content, Some("project"), project, PROJECT_KEYS);
    }
    if let Some(package) = root.get("package").and_then(|v| v.as_table()) {
        check_table_unknown_keys(diagnostics, content, Some("package"), package, PACKAGE_KEYS);
    }
    if let Some(stdlib) = root.get("stdlib").and_then(|v| v.as_table()) {
        check_table_unknown_keys(diagnostics, content, Some("stdlib"), stdlib, STDLIB_KEYS);
    }
    if let Some(workspace) = root.get("workspace").and_then(|v| v.as_table()) {
        check_table_unknown_keys(
            diagnostics,
            content,
            Some("workspace"),
            workspace,
            WORKSPACE_KEYS,
        );
        if let Some(workspace_project) = workspace.get("project").and_then(|v| v.as_table()) {
            check_table_unknown_keys(
                diagnostics,
                content,
                Some("workspace.project"),
                workspace_project,
                WORKSPACE_PROJECT_KEYS,
            );
        }
    }
    if let Some(deps) = root.get("dependencies").and_then(|v| v.as_table()) {
        for (dep_name, dep_value) in deps {
            if let Some(dep_table) = dep_value.as_table() {
                for dep_key in dep_table.keys() {
                    if DEP_DETAIL_KEYS.contains(&dep_key.as_str()) {
                        continue;
                    }
                    let range = find_dep_name_range(content, dep_name);
                    let suggestion =
                        nearest_known_key(dep_key, DEP_DETAIL_KEYS).map(str::to_string);
                    let message = match &suggestion {
                        Some(s) => format!(
                            "unknown dependency key '{}' in '{}'; did you mean '{}'?",
                            dep_key, dep_name, s
                        ),
                        None => format!("unknown dependency key '{}' in '{}'", dep_key, dep_name),
                    };
                    diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("sysml-manifest".to_owned()),
                        code: Some(NumberOrString::String("M001".to_owned())),
                        data: Some(serde_json::json!({
                            "kind": "unknown_key",
                            "section": "dependencies.<name>",
                            "key": dep_key,
                            "suggestion": suggestion.clone(),
                        })),
                        message,
                        ..Default::default()
                    });
                }
            }
        }
    }
}

fn check_table_unknown_keys(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    section: Option<&str>,
    table: &toml::map::Map<String, toml::Value>,
    allowed: &[&str],
) {
    for key in table.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        push_unknown_key_diagnostic(diagnostics, content, section, key, allowed);
    }
}

fn push_unknown_key_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    section: Option<&str>,
    key: &str,
    allowed: &[&str],
) {
    let range = find_key_range_in_section(content, section, key);
    let suggestion = nearest_known_key(key, allowed).map(str::to_string);
    let message = match &suggestion {
        Some(s) => format!("unknown key '{}'; did you mean '{}'?", key, s),
        None => format!("unknown key '{}'", key),
    };
    diagnostics.push(Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::WARNING),
        source: Some("sysml-manifest".to_owned()),
        code: Some(NumberOrString::String("M001".to_owned())),
        data: Some(serde_json::json!({
            "kind": "unknown_key",
            "section": section.unwrap_or("<top-level>"),
            "key": key,
            "suggestion": suggestion,
        })),
        message,
        ..Default::default()
    });
}

fn check_workspace_semantics(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    manifest: &SysmlManifest,
    manifest_dir: Option<&std::path::Path>,
) {
    let Some(workspace) = manifest.workspace.as_ref() else {
        return;
    };

    // Duplicate entry detection in members/exclude/default-members.
    check_duplicate_workspace_entries(diagnostics, content, "members", &workspace.members, "M030");
    check_duplicate_workspace_entries(diagnostics, content, "exclude", &workspace.exclude, "M031");
    check_duplicate_workspace_entries(
        diagnostics,
        content,
        "default-members",
        &workspace.default_members,
        "M032",
    );

    let members_set = workspace
        .members
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let exclude_set = workspace
        .exclude
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    // Overlap: member also excluded.
    for overlap in members_set.intersection(&exclude_set) {
        diagnostics.push(Diagnostic {
            range: find_key_value_range(content, "exclude"),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("sysml-manifest".to_owned()),
            code: Some(NumberOrString::String("M033".to_owned())),
            data: Some(serde_json::json!({
                "kind": "workspace_member_excluded_overlap",
                "path": overlap,
            })),
            message: format!(
                "workspace member '{}' is also listed in [workspace].exclude",
                overlap
            ),
            ..Default::default()
        });
    }

    // default-members must exist in members.
    for default_member in &workspace.default_members {
        if !members_set.contains(default_member) {
            diagnostics.push(Diagnostic {
                range: find_key_value_range(content, "default-members"),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String("M034".to_owned())),
                data: Some(serde_json::json!({
                    "kind": "default_member_not_in_members",
                    "path": default_member,
                })),
                message: format!(
                    "default member '{}' is not present in [workspace].members",
                    default_member
                ),
                ..Default::default()
            });
        }
    }

    // Missing workspace member paths on disk.
    if let Some(root) = manifest_dir {
        for member in &workspace.members {
            let member_path = root.join(member);
            if !member_path.exists() {
                diagnostics.push(Diagnostic {
                    range: find_key_value_range(content, "members"),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("sysml-manifest".to_owned()),
                    code: Some(NumberOrString::String("M035".to_owned())),
                    data: Some(serde_json::json!({
                        "kind": "missing_workspace_member_path",
                        "path": member,
                    })),
                    message: format!(
                        "workspace member '{}' does not exist relative to this workspace root",
                        member
                    ),
                    ..Default::default()
                });
            }
        }
    }
}

fn check_duplicate_workspace_entries(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    key: &str,
    values: &[String],
    code: &str,
) {
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            diagnostics.push(Diagnostic {
                range: find_key_value_range(content, key),
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("sysml-manifest".to_owned()),
                code: Some(NumberOrString::String(code.to_owned())),
                data: Some(serde_json::json!({
                    "kind": "duplicate_workspace_entry",
                    "key": key,
                    "path": value,
                })),
                message: format!("duplicate entry '{}' in [workspace].{}", value, key),
                ..Default::default()
            });
        }
    }
}

fn find_key_range_in_section(content: &str, section: Option<&str>, key: &str) -> Range {
    let target_header = section.map(|s| format!("[{}]", s));
    let mut in_section = section.is_none();

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = target_header
                .as_ref()
                .map(|h| trimmed == h)
                .unwrap_or(false);
            continue;
        }

        if !in_section {
            continue;
        }
        if let Some(stripped) = trimmed.strip_prefix(key) {
            let rest = stripped.trim_start();
            if rest.starts_with('=') {
                return Range {
                    start: Position::new(line_idx as u32, 0),
                    end: Position::new(line_idx as u32, line.len() as u32),
                };
            }
        }
    }

    find_key_value_range(content, key)
}

fn nearest_known_key<'a>(key: &str, allowed: &'a [&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for candidate in allowed {
        let dist = edit_distance(key, candidate);
        match best {
            Some((_, best_dist)) if dist >= best_dist => {}
            _ => best = Some((candidate, dist)),
        }
    }
    best.and_then(|(candidate, distance)| if distance <= 3 { Some(candidate) } else { None })
}

fn edit_distance(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.chars().count();
    }
    if b.is_empty() {
        return a.chars().count();
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0; b_chars.len() + 1];

    for (i, ac) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, bc) in b_chars.iter().enumerate() {
            let cost = if ac == bc { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

/// Simple semver check: major.minor.patch with optional pre-release/build metadata.
fn is_valid_semver(version: &str) -> bool {
    semver::Version::parse(version).is_ok()
}

// ---- Position-finding helpers ----

/// Find the range for a TOML parse error. Tries to extract line info from the
/// error message, falling back to the first line.
fn find_error_range(content: &str, error_message: &str) -> Range {
    // toml errors often contain patterns like "at line N column M"
    if let Some(line) = extract_error_line(error_message) {
        let line_idx = line.saturating_sub(1);
        let line_text = content.lines().nth(line_idx as usize).unwrap_or("");
        return Range {
            start: Position::new(line_idx, 0),
            end: Position::new(line_idx, line_text.len() as u32),
        };
    }
    // Fallback: highlight the first line
    let first_line_len = content.lines().next().map_or(0, |l| l.len());
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, first_line_len as u32),
    }
}

/// Try to extract a line number from a TOML error message.
fn extract_error_line(message: &str) -> Option<u32> {
    // Pattern: "at line N"
    let idx = message.find("at line ")?;
    let after = &message[idx + 8..];
    let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Find the range of a `key = value` line in the TOML source.
fn find_key_value_range(content: &str, key: &str) -> Range {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix(key) {
            let rest = stripped.trim_start();
            if rest.starts_with('=') {
                return Range {
                    start: Position::new(line_idx as u32, 0),
                    end: Position::new(line_idx as u32, line.len() as u32),
                };
            }
        }
    }
    // Fallback
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, 0),
    }
}

/// Find the range of a `[section]` header line.
fn find_section_range(content: &str, section: &str) -> Range {
    for (line_idx, line) in content.lines().enumerate() {
        if line.trim() == section {
            return Range {
                start: Position::new(line_idx as u32, 0),
                end: Position::new(line_idx as u32, line.len() as u32),
            };
        }
    }
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, 0),
    }
}

/// Find the range of a dependency name within `[dependencies]`.
/// Looks for `name = ...` lines after the `[dependencies]` header.
fn find_dep_name_range(content: &str, dep_name: &str) -> Range {
    let mut in_deps = false;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_deps = true;
            continue;
        }
        if in_deps && trimmed.starts_with('[') {
            // New section — stop looking
            break;
        }
        if in_deps && trimmed.starts_with(dep_name) {
            let rest = trimmed[dep_name.len()..].trim_start();
            if rest.starts_with('=') {
                return Range {
                    start: Position::new(line_idx as u32, 0),
                    end: Position::new(line_idx as u32, line.len() as u32),
                };
            }
        }
    }
    // Also check inline table syntax: [dependencies.name]
    let section_header = format!("[dependencies.{}]", dep_name);
    for (line_idx, line) in content.lines().enumerate() {
        if line.trim() == section_header {
            return Range {
                start: Position::new(line_idx as u32, 0),
                end: Position::new(line_idx as u32, line.len() as u32),
            };
        }
    }
    // Fallback
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, 0),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sha2::{Digest, Sha256};
    use sysml_project::kpar::{write_kpar, KparArchive, ProjectInfo, ProjectMetadata};
    use tempfile::TempDir;
    use tower_lsp::lsp_types::Url;

    fn write_manifest_and_uri(root: &Path, content: &str) -> String {
        let manifest_path = root.join("sysml.toml");
        fs::write(&manifest_path, content).expect("manifest should be written");
        Url::from_file_path(&manifest_path)
            .expect("manifest path should map to file URI")
            .to_string()
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
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
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_git_fixture(root: &Path, name: &str, version: &str) -> (String, String) {
        let repo_dir = root.join(name);
        fs::create_dir_all(&repo_dir).expect("git fixture dir should be created");
        git(&repo_dir, &["init", "--initial-branch", "main"]);
        git(
            &repo_dir,
            &["config", "user.email", "sysml-tests@example.com"],
        );
        git(&repo_dir, &["config", "user.name", "SysML Tests"]);
        fs::write(
            repo_dir.join("sysml.toml"),
            format!("[project]\nname = \"{name}\"\nversion = \"{version}\"\n"),
        )
        .expect("git fixture manifest should be written");
        git(&repo_dir, &["add", "sysml.toml"]);
        git(&repo_dir, &["commit", "-m", "initial"]);
        let commit = git(&repo_dir, &["rev-parse", "HEAD"]);
        (format!("file://{}", repo_dir.display()), commit)
    }

    fn write_fixture_kpar(path: &Path, name: &str, version: &str) {
        let mut metadata = ProjectMetadata::new();
        metadata.created = Some("2026-03-04T00:00:00Z".to_string());
        metadata.add_index_entry("Root", "Root.sysml");

        let archive = KparArchive {
            root_dir: name.to_string(),
            project_info: ProjectInfo {
                name: name.to_string(),
                version: version.to_string(),
                description: Some("manifest diagnostic fixture".to_string()),
                license: Some("MIT".to_string()),
                usage: Vec::new(),
            },
            metadata,
            source_files: vec![(
                "Root.sysml".to_string(),
                b"package Root {\n  part def Unit;\n}\n".to_vec(),
            )],
        };

        write_kpar(path, &archive).expect("fixture kpar should be written");
    }

    fn sha256_hex_file(path: &Path) -> String {
        let bytes = fs::read(path).expect("file should be readable");
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn write_sysand_index_with_releases(root: &Path, package: &str, releases: &[(&str, PathBuf)]) {
        let index_dir = root.join(".sysml/registries/sysand");
        let artifacts_dir = index_dir.join("artifacts");
        fs::create_dir_all(&artifacts_dir).expect("sysand artifacts dir should be created");

        let mut entries = Vec::new();
        for (version, source_artifact) in releases {
            let filename = format!("{package}-{version}.kpar");
            let target = artifacts_dir.join(&filename);
            fs::copy(source_artifact, &target).expect("artifact should be copied");
            let checksum = format!("sha256:{}", sha256_hex_file(&target));
            entries.push(format!(
                "\"{version}\":{{\"artifact\":\"artifacts/{filename}\",\"checksum\":\"{checksum}\"}}"
            ));
        }

        fs::write(
            index_dir.join("index.json"),
            format!(
                "{{\"packages\":{{\"{package}\":{{{}}}}}}}",
                entries.join(",")
            ),
        )
        .expect("sysand index should be written");
    }

    #[test]
    fn test_valid_manifest_no_diagnostics() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"
"#;
        let diagnostics = validate_manifest(content);
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_invalid_toml_produces_error() {
        let content = r#"
[project
name = "broken"
"#;
        let diagnostics = validate_manifest(content);
        assert!(!diagnostics.is_empty(), "expected at least one diagnostic");
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn test_invalid_semver_produces_error() {
        let content = r#"
[project]
name = "my-project"
version = "not-a-version"
sysml-edition = "2025"
"#;
        let diagnostics = validate_manifest(content);
        let semver_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("invalid semver"));
        assert!(
            semver_diag.is_some(),
            "expected semver error diagnostic, got: {:?}",
            diagnostics
        );
        assert_eq!(
            semver_diag.unwrap().severity,
            Some(DiagnosticSeverity::ERROR)
        );
    }

    #[test]
    fn test_git_dep_without_ref_produces_warning() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
some-lib = { git = "https://github.com/org/repo" }
"#;
        let diagnostics = validate_manifest(content);
        let git_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("no tag, branch, or rev"));
        assert!(
            git_diag.is_some(),
            "expected git ref warning, got: {:?}",
            diagnostics
        );
        assert_eq!(
            git_diag.unwrap().severity,
            Some(DiagnosticSeverity::WARNING)
        );
    }

    #[test]
    fn test_conflicting_dep_sources() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
dual-src = { path = "../local", git = "https://github.com/org/repo", tag = "v1" }
"#;
        let diagnostics = validate_manifest(content);
        let conflict_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("both 'path' and 'git'"));
        assert!(
            conflict_diag.is_some(),
            "expected conflicting sources warning, got: {:?}",
            diagnostics
        );
        assert_eq!(
            conflict_diag.unwrap().severity,
            Some(DiagnosticSeverity::WARNING)
        );
    }

    #[test]
    fn test_unknown_edition_produces_warning() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2099"
"#;
        let diagnostics = validate_manifest(content);
        let edition_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("unknown sysml-edition"));
        assert!(
            edition_diag.is_some(),
            "expected edition warning, got: {:?}",
            diagnostics
        );
        assert_eq!(
            edition_diag.unwrap().severity,
            Some(DiagnosticSeverity::WARNING)
        );
    }

    #[test]
    fn test_invalid_license_produces_warning() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
license = "WTFPL"
sysml-edition = "2025"
"#;
        let diagnostics = validate_manifest(content);
        let license_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("unknown SPDX"));
        assert!(
            license_diag.is_some(),
            "expected license warning, got: {:?}",
            diagnostics
        );
        assert_eq!(
            license_diag.unwrap().severity,
            Some(DiagnosticSeverity::WARNING)
        );
    }

    #[test]
    fn test_empty_dependencies_hint() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
"#;
        let diagnostics = validate_manifest(content);
        let empty_diag = diagnostics.iter().find(|d| d.message.contains("empty"));
        assert!(
            empty_diag.is_some(),
            "expected empty deps hint, got: {:?}",
            diagnostics
        );
        assert_eq!(empty_diag.unwrap().severity, Some(DiagnosticSeverity::HINT));
    }

    #[test]
    fn test_path_kpar_conflict() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
conflict = { path = "../local", kpar = "https://example.com/lib.kpar" }
"#;
        let diagnostics = validate_manifest(content);
        let conflict_diag = diagnostics
            .iter()
            .find(|d| d.message.contains("both 'path' and 'kpar'"));
        assert!(
            conflict_diag.is_some(),
            "expected path+kpar conflict warning, got: {:?}",
            diagnostics
        );
    }

    #[test]
    fn test_git_dep_with_tag_no_warning() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
pinned = { git = "https://github.com/org/repo", tag = "v1.0.0" }
"#;
        let diagnostics = validate_manifest(content);
        let git_diag = diagnostics.iter().find(|d| d.message.contains("no tag"));
        assert!(git_diag.is_none(), "should not warn for git dep with tag");
    }

    #[test]
    fn test_unknown_key_with_suggestion() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"
licence = "MIT"
"#;
        let diagnostics = validate_manifest(content);
        let unknown = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M001".to_string())));
        assert!(unknown.is_some(), "expected unknown-key diagnostic");
        assert!(
            unknown.unwrap().message.contains("did you mean 'license'"),
            "expected typo suggestion in diagnostic message"
        );
    }

    #[test]
    fn test_workspace_default_member_not_in_members() {
        let content = r#"
[project]
name = "my-project"
version = "0.1.0"

[workspace]
members = ["a"]
default-members = ["missing"]
"#;
        let diagnostics = validate_manifest(content);
        let diag = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M034".to_string())));
        assert!(
            diag.is_some(),
            "expected default-members validation diagnostic"
        );
    }

    #[test]
    fn test_stdlib_unknown_library_entry_produces_warning() {
        let content = r#"
[project]
name = "stdlib-unknown"
version = "0.1.0"

[stdlib]
include_only = ["systems", "analysiss"]
"#;
        let diagnostics = validate_manifest(content);
        let diag = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M050".to_string())));
        assert!(diag.is_some(), "expected unknown stdlib library diagnostic");
        assert!(
            diag.unwrap().message.contains("did you mean 'analysis'"),
            "expected typo suggestion for unknown stdlib library"
        );
    }

    #[test]
    fn test_stdlib_duplicate_and_overlap_diagnostics() {
        let content = r#"
[project]
name = "stdlib-overlap"
version = "0.1.0"

[stdlib]
include_only = ["systems", "systems", "analysis"]
exclude = ["analysis"]
"#;
        let diagnostics = validate_manifest(content);
        let has_duplicate = diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("M051".to_string())));
        let has_overlap = diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("M052".to_string())));
        assert!(has_duplicate, "expected duplicate stdlib entry diagnostic");
        assert!(has_overlap, "expected include/exclude overlap diagnostic");
    }

    #[test]
    fn test_runtime_dependency_failure_includes_git_source_details() {
        if !git_available() {
            eprintln!("skipping git runtime diagnostic test: git binary unavailable");
            return;
        }

        let dir = TempDir::new().expect("workspace temp dir should be created");
        let (git_url, _commit) = create_git_fixture(dir.path(), "dep-git", "1.0.0");
        let bad_rev = "0000000000000000000000000000000000000000";
        let content = format!(
            r#"
[project]
name = "runtime-git"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
dep-git = {{ git = "{git_url}", rev = "{bad_rev}" }}
"#
        );
        let uri = write_manifest_and_uri(dir.path(), &content);
        let diagnostics = validate_manifest_with_context(&content, Some(&uri));
        let runtime = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M040".to_string())))
            .expect("expected runtime dependency diagnostic");
        assert_eq!(runtime.severity, Some(DiagnosticSeverity::ERROR));
        assert!(
            runtime.message.contains("dep-git") && runtime.message.contains("(git)"),
            "expected git source detail in runtime diagnostic message, got: {}",
            runtime.message
        );
    }

    #[test]
    fn test_runtime_dependency_failure_includes_path_source_details() {
        let dir = TempDir::new().expect("workspace temp dir should be created");
        let content = r#"
[project]
name = "runtime-path"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
dep-path = { path = "./missing-path" }
"#;
        let uri = write_manifest_and_uri(dir.path(), content);
        let diagnostics = validate_manifest_with_context(content, Some(&uri));
        let runtime = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M040".to_string())))
            .expect("expected runtime dependency diagnostic");
        assert!(
            runtime.message.contains("dep-path") && runtime.message.contains("(path)"),
            "expected path source detail in runtime diagnostic message, got: {}",
            runtime.message
        );
    }

    #[test]
    fn test_registry_dependency_update_available_hint() {
        let dir = TempDir::new().expect("workspace temp dir should be created");
        let package = "dep-registry-update";
        let a_100 = dir.path().join("dep-registry-update-1.0.0.kpar");
        let a_120 = dir.path().join("dep-registry-update-1.2.0.kpar");
        write_fixture_kpar(&a_100, package, "1.0.0");
        write_fixture_kpar(&a_120, package, "1.2.0");
        write_sysand_index_with_releases(
            dir.path(),
            package,
            &[("1.0.0", a_100.clone()), ("1.2.0", a_120.clone())],
        );

        let content = format!(
            r#"
[project]
name = "runtime-registry-update"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
{package} = "1.0.0"
"#
        );
        let uri = write_manifest_and_uri(dir.path(), &content);
        let diagnostics = validate_manifest_with_context(&content, Some(&uri));
        let update = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("M041".to_string())))
            .expect("expected update-available hint diagnostic");
        assert_eq!(update.severity, Some(DiagnosticSeverity::HINT));
        assert!(
            update.message.contains("1.0.0") && update.message.contains("1.2.0"),
            "expected requested/resolved and latest versions in update hint, got: {}",
            update.message
        );
    }

    #[test]
    fn test_partial_runtime_dependency_checks_do_not_block_other_diagnostics() {
        let dir = TempDir::new().expect("workspace temp dir should be created");
        let package = "dep-registry-partial";
        let a_100 = dir.path().join("dep-registry-partial-1.0.0.kpar");
        let a_110 = dir.path().join("dep-registry-partial-1.1.0.kpar");
        write_fixture_kpar(&a_100, package, "1.0.0");
        write_fixture_kpar(&a_110, package, "1.1.0");
        write_sysand_index_with_releases(
            dir.path(),
            package,
            &[("1.0.0", a_100.clone()), ("1.1.0", a_110.clone())],
        );

        let content = format!(
            r#"
[project]
name = "runtime-partial"
version = "0.1.0"
sysml-edition = "2025"

[dependencies]
missing-path = {{ path = "./does-not-exist" }}
{package} = "1.0.0"
"#
        );
        let uri = write_manifest_and_uri(dir.path(), &content);
        let diagnostics = validate_manifest_with_context(&content, Some(&uri));
        let has_runtime_failure = diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("M040".to_string())));
        let has_update_hint = diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("M041".to_string())));
        assert!(
            has_runtime_failure,
            "expected runtime dependency failure diagnostic"
        );
        assert!(
            has_update_hint,
            "expected update hint diagnostic even with another failing dependency"
        );
    }
}
