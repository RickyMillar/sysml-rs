//! Inlay hint handler.

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
use std::path::{Path, PathBuf};

use semver::Version;
use sysml_manifest::{Dependency, SysmlManifest};
use sysml_resolve::PackageSource;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{InlayHintParams, InlayHint, InlayHintLabel, InlayHintKind, InlayHintTooltip, Range, Position};

use crate::evaluation;
use crate::hover::find_element_type;
use sysml_service::project_discovery;
use crate::utils::{offset_to_position, parse_uri, position_to_offset};
use crate::SysmlLanguageServer;

pub(crate) async fn inlay_hint(
    server: &SysmlLanguageServer,
    params: InlayHintParams,
) -> Result<Option<Vec<InlayHint>>> {
    if !server.features.read().await.inlay_hints {
        return Ok(None);
    }

    let uri = params.text_document.uri.to_string();
    let range = params.range;

    if uri.ends_with(sysml_manifest::MANIFEST_FILENAME) {
        return Ok(Some(manifest_dependency_hints(server, &uri, &range).await));
    }

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    let range_start = position_to_offset(&range.start, &doc.content);
    let range_end = position_to_offset(&range.end, &doc.content);

    let mut hints = Vec::new();

    for element in doc.graph.elements.values() {
        // Only show inlay hints for usages without explicit type annotations
        if !element.kind.is_usage() {
            continue;
        }

        let span = match element.spans.first() {
            Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
            _ => continue,
        };

        // Skip if no name (unnamed usages)
        let Some(_name) = &element.name else {
            continue;
        };

        // Check if element has an explicit typing in the source
        // (look for `:` or `typed by` in the element's text)
        // Guard: spans may be stale if parse cache lags behind content
        let Some(text) = crate::utils::safe_slice(&doc.content, span.start, span.end) else {
            continue;
        };
        if text.contains(':') || text.contains("typed by") {
            continue;
        }

        // Find the resolved type
        if let Some(type_name) = find_element_type(element, &doc.graph) {
            // Place hint after the element name
            let name_end =
                (span.start + element.name.as_ref().map_or(0, |n| n.len())).min(doc.content.len());
            let hint_pos = offset_to_position(name_end, &doc.content);

            hints.push(InlayHint {
                position: hint_pos,
                label: InlayHintLabel::String(format!(": {}", type_name)),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(format!(
                    "Inferred type: {}",
                    type_name
                ))),
                padding_left: Some(false),
                padding_right: Some(true),
                data: None,
            });
        }
    }

    // Specialization hints for definitions
    for element in doc.graph.elements.values() {
        if !element.kind.is_definition() {
            continue;
        }

        let span = match element.spans.first() {
            Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
            _ => continue,
        };

        // Skip if no name
        let Some(_name) = &element.name else {
            continue;
        };

        // Skip if already has explicit specialization in source
        // Guard: spans may be stale if parse cache lags behind content
        let Some(text) = crate::utils::safe_slice(&doc.content, span.start, span.end) else {
            continue;
        };
        if text.contains(":>") || text.contains("specializes") {
            continue;
        }

        // Find supertypes via Specialize relationships
        let supertypes: Vec<String> = doc
            .graph
            .outgoing(&element.id)
            .filter(|r| r.kind == sysml_core::RelationshipKind::Specialize)
            .filter_map(|r| doc.graph.get_element(&r.target))
            .filter_map(|e| e.name.clone())
            .collect();

        if !supertypes.is_empty() {
            let name_end = span.start + element.name.as_ref().map_or(0, |n| n.len());
            let hint_pos = offset_to_position(name_end, &doc.content);

            hints.push(InlayHint {
                position: hint_pos,
                label: InlayHintLabel::String(format!(":> {}", supertypes.join(", "))),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(format!(
                    "Specializes: {}",
                    supertypes.join(", ")
                ))),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            });
        }
    }

    // Value inlay hints for expressions (calculations, attributes with expressions)
    for element in doc.graph.elements.values() {
        let is_evaluable = matches!(
            element.kind,
            sysml_core::ElementKind::CalculationUsage | sysml_core::ElementKind::AttributeUsage
        );
        if !is_evaluable {
            continue;
        }

        let span = match element.spans.first() {
            Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
            _ => continue,
        };

        if let Some(display) = evaluation::try_evaluate_value(element, &doc.graph) {
            let hint_pos = offset_to_position(span.end.min(doc.content.len()), &doc.content);
            // Stash the structured expression AST in `data` so the VS Code
            // client can render math (KaTeX) on hover-over-hint without a
            // round-trip to the server (Phase 6B.4).
            let data = sysml_service::expression_ast::project_owner(element, &doc.graph)
                .and_then(|r| r.ast)
                .and_then(|ast| serde_json::to_value(ast).ok());
            hints.push(InlayHint {
                position: hint_pos,
                label: InlayHintLabel::String(format!("= {}", display)),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String("Evaluated expression".to_owned())),
                padding_left: Some(true),
                padding_right: Some(false),
                data,
            });
        }
    }

    // Multiplicity hints for usages without explicit `[` in source
    for element in doc.graph.elements.values() {
        if !element.kind.is_usage() {
            continue;
        }
        let span = match element.spans.first() {
            Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
            _ => continue,
        };
        let Some(name) = &element.name else {
            continue;
        };
        let Some(text) = crate::utils::safe_slice(&doc.content, span.start, span.end) else {
            continue;
        };
        if text.contains('[') {
            continue;
        }
        // Try formatted multiplicity string, then fall back to lower/upper ints
        let mult_display = element
            .props
            .get("multiplicity")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .or_else(|| {
                let lower = element.get_prop("multiplicity_lower")?.as_int()?;
                let upper_str = element
                    .get_prop("multiplicity_upper")
                    .map(|v| match v.as_int() {
                        Some(u) => u.to_string(),
                        None => "*".to_owned(),
                    })
                    .unwrap_or_else(|| "*".to_owned());
                if upper_str == lower.to_string() {
                    Some(format!("{}", lower))
                } else {
                    Some(format!("{}..{}", lower, upper_str))
                }
            });
        if let Some(mult) = mult_display {
            let name_end = (span.start + name.len()).min(doc.content.len());
            let hint_pos = offset_to_position(name_end, &doc.content);
            hints.push(InlayHint {
                position: hint_pos,
                label: InlayHintLabel::String(format!("[{}]", mult)),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(format!(
                    "Multiplicity: [{}]",
                    mult
                ))),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            });
        }
    }

    // Constraint status hints: [pass] / [FAIL] for evaluable constraints
    {
        let mut constraint_count = 0usize;
        for element in doc.graph.elements.values() {
            if constraint_count >= 20 {
                break;
            }
            if !matches!(
                element.kind,
                sysml_core::ElementKind::ConstraintUsage
                    | sysml_core::ElementKind::AssertConstraintUsage
            ) {
                continue;
            }
            let span = match element.spans.first() {
                Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
                _ => continue,
            };
            if let Some(display) = evaluation::try_evaluate_value(element, &doc.graph) {
                let passed = matches!(display.as_str(), "true" | "1");
                let label = if passed { "[pass]" } else { "[FAIL]" };
                let hint_pos = offset_to_position(span.end.min(doc.content.len()), &doc.content);
                hints.push(InlayHint {
                    position: hint_pos,
                    label: InlayHintLabel::String(label.to_owned()),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(format!(
                        "Constraint evaluates to: {}",
                        display
                    ))),
                    padding_left: Some(true),
                    padding_right: Some(false),
                    data: None,
                });
                constraint_count += 1;
            }
        }
    }

    // Import count hints: (N elements) for import statements
    for element in doc.graph.elements.values() {
        if !matches!(
            element.kind,
            sysml_core::ElementKind::NamespaceImport | sysml_core::ElementKind::MembershipImport
        ) {
            continue;
        }
        let span = match element.spans.first() {
            Some(s) if s.file == uri && s.start >= range_start && s.start < range_end => s,
            _ => continue,
        };
        if let Some(ns_name) = element
            .props
            .get("importedNamespace")
            .and_then(|v| v.as_str())
        {
            // Count resolved members by looking up the namespace in the graph
            let member_count = doc
                .graph
                .elements
                .values()
                .filter(|e| {
                    e.qname
                        .as_ref()
                        .map(|q| {
                            let qs = q.to_string();
                            qs.starts_with(ns_name)
                                && qs.len() > ns_name.len()
                                && qs.as_bytes().get(ns_name.len()) == Some(&b':')
                        })
                        .unwrap_or(false)
                })
                .count();
            if member_count > 0 {
                let hint_pos = offset_to_position(span.end.min(doc.content.len()), &doc.content);
                hints.push(InlayHint {
                    position: hint_pos,
                    label: InlayHintLabel::String(format!(
                        "({} element{})",
                        member_count,
                        if member_count == 1 { "" } else { "s" }
                    )),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(format!(
                        "Import resolves {} member{} from {}",
                        member_count,
                        if member_count == 1 { "" } else { "s" },
                        ns_name
                    ))),
                    padding_left: Some(true),
                    padding_right: Some(false),
                    data: None,
                });
            }
        }
    }

    Ok(Some(hints))
}

async fn manifest_dependency_hints(
    server: &SysmlLanguageServer,
    uri: &str,
    range: &Range,
) -> Vec<InlayHint> {
    let Some(doc) = server.salsa_parsed_doc(uri).await else {
        return Vec::new();
    };
    let content = doc.content;

    let Some(manifest_root) = manifest_root_from_uri(uri) else {
        return Vec::new();
    };
    let Ok(manifest) = toml::from_str::<SysmlManifest>(&content) else {
        return Vec::new();
    };

    let outcomes =
        project_discovery::resolve_manifest_dependencies(&manifest, &manifest_root, false);
    let outcomes_by_dep: HashMap<&str, &project_discovery::DependencyResolutionOutcome> = outcomes
        .iter()
        .map(|outcome| (outcome.dependency_name.as_str(), outcome))
        .collect();

    let mut hints = Vec::new();
    for (dep_name, dep_spec) in &manifest.dependencies {
        let Some(position) = find_dependency_line_end_position(&content, dep_name) else {
            continue;
        };
        if !position_in_range(position, range) {
            continue;
        }
        let Some(outcome) = outcomes_by_dep.get(dep_name.as_str()) else {
            continue;
        };

        let (label, tooltip) = if let Some(failure) = &outcome.failure {
            (
                "resolve error".to_owned(),
                format!(
                    "{} dependency '{}' failed: {}. {}",
                    failure.source_kind, dep_name, failure.message, failure.action
                ),
            )
        } else if outcome.source_kind == "registry" {
            registry_dependency_hint(dep_name, dep_spec, outcome, &manifest_root)
        } else {
            (
                "resolved".to_owned(),
                format!("{} dependency '{}' resolved", outcome.source_kind, dep_name),
            )
        };

        hints.push(InlayHint {
            position,
            label: InlayHintLabel::String(label),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: Some(InlayHintTooltip::String(tooltip)),
            padding_left: Some(true),
            padding_right: Some(false),
            data: None,
        });
    }

    hints
}

fn manifest_root_from_uri(uri: &str) -> Option<PathBuf> {
    parse_uri(uri)?
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
}

fn registry_dependency_hint(
    dep_name: &str,
    dep_spec: &Dependency,
    outcome: &project_discovery::DependencyResolutionOutcome,
    manifest_root: &Path,
) -> (String, String) {
    let Some((backend, package, requested_requirement, resolved_version)) =
        registry_resolution_metadata(dep_name, dep_spec, outcome)
    else {
        return (
            "resolved".to_owned(),
            format!("registry dependency '{}' resolved", dep_name),
        );
    };

    if !allow_offline_registry_update_check(&backend, manifest_root) {
        return (
            format!("resolved {resolved_version}"),
            format!(
                "registry dependency '{}' resolved to {} (requested '{}')",
                dep_name, resolved_version, requested_requirement
            ),
        );
    }

    let latest = match sysml_resolve::resolve_latest_registry_release_metadata(
        &backend,
        &package,
        manifest_root,
    ) {
        Ok(latest) => latest,
        Err(_) => {
            return (
                format!("resolved {resolved_version}"),
                format!(
                    "registry dependency '{}' resolved to {} (requested '{}')",
                    dep_name, resolved_version, requested_requirement
                ),
            );
        }
    };

    match (
        Version::parse(&resolved_version),
        Version::parse(&latest.resolved_version),
    ) {
        (Ok(current), Ok(latest_version)) if latest_version > current => (
            format!("{} available", latest.resolved_version),
            format!(
                "registry dependency '{}' resolved to {} (requested '{}'); newest compatible release is {}",
                dep_name, resolved_version, requested_requirement, latest.resolved_version
            ),
        ),
        (Ok(_), Ok(_)) => (
            "up to date".to_owned(),
            format!(
                "registry dependency '{}' is up to date at {} (requested '{}')",
                dep_name, resolved_version, requested_requirement
            ),
        ),
        _ => (
            format!("resolved {resolved_version}"),
            format!(
                "registry dependency '{}' resolved to {} (requested '{}')",
                dep_name, resolved_version, requested_requirement
            ),
        ),
    }
}

fn registry_resolution_metadata<'a>(
    dep_name: &str,
    dep_spec: &'a Dependency,
    outcome: &'a project_discovery::DependencyResolutionOutcome,
) -> Option<(String, String, String, String)> {
    let mut fallback: Option<(String, String, String, String)> = None;
    for package in &outcome.hydrated_packages {
        if let PackageSource::Registry {
            backend,
            package,
            requested,
            version,
        } = &package.source
        {
            let candidate = (
                backend.clone(),
                package.clone(),
                requested.clone(),
                version.clone(),
            );
            if package == dep_name {
                return Some(candidate);
            }
            fallback = fallback.or(Some(candidate));
        }
    }

    if let Some(meta) = fallback {
        return Some(meta);
    }

    match dep_spec {
        Dependency::Registry(requirement) => Some((
            "sysand".to_owned(),
            dep_name.to_owned(),
            requirement.clone(),
            requirement.clone(),
        )),
        Dependency::Detailed(d) => d.version.as_ref().map(|requirement| {
            (
                "sysand".to_owned(),
                dep_name.to_owned(),
                requirement.clone(),
                requirement.clone(),
            )
        }),
    }
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

fn find_dependency_line_end_position(content: &str, dep_name: &str) -> Option<Position> {
    let mut in_dependencies = false;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if !in_dependencies || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((lhs, _rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim().trim_matches('"');
        if key == dep_name {
            return Some(Position {
                line: line_idx as u32,
                character: line.encode_utf16().count() as u32,
            });
        }
    }
    None
}

fn position_in_range(position: Position, range: &Range) -> bool {
    if position.line < range.start.line || position.line > range.end.line {
        return false;
    }
    if position.line == range.start.line && position.character < range.start.character {
        return false;
    }
    if position.line == range.end.line && position.character > range.end.character {
        return false;
    }
    true
}
