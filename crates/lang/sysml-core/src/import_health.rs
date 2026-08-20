//! Import health diagnostics.
//!
//! Model-level checks on import elements: unknown namespaces, circular chains,
//! duplicate imports, recursive on non-namespaces, and empty wildcard imports.

use std::collections::{HashMap, HashSet};

use crate::element_ordering::{primary_span, sort_elements_by_source_order};
use crate::{ElementId, ElementKind, ModelGraph};
use sysml_span::Diagnostic;
#[cfg(test)]
use sysml_span::Span;

/// Diagnose import health issues across all import elements in a graph.
///
/// Returns diagnostics for:
/// - IM001: import references namespace unresolved in current workspace context (info)
/// - IM002: recursive import on non-namespace element
/// - IM003: duplicate import in same scope
/// - IM004: circular import chain
/// - IM005: wildcard import that imports 0 visible members
/// - IM006: import references unknown standard library namespace (error)
pub fn import_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    import_health_diagnostics_with_library(graph, None)
}

/// Diagnose import health issues with optional library visibility.
///
/// When a library graph is provided, namespace existence checks treat
/// library namespaces as valid import targets.
pub fn import_health_diagnostics_with_library(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
) -> Vec<Diagnostic> {
    import_health_diagnostics_with_context(graph, library, None)
}

/// Diagnose import health issues with full workspace + library context.
///
/// When a workspace graph is provided, namespace existence and member
/// checks also look in the workspace-merged graph. This prevents false
/// positives for cross-file imports (IM001) and wildcard imports of
/// cross-file namespaces (IM005).
pub fn import_health_diagnostics_with_context(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    workspace: Option<&ModelGraph>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let import_kinds = [
        ElementKind::NamespaceImport,
        ElementKind::MembershipImport,
        ElementKind::Import, // batch parser uses generic Import with isNamespace/isRecursive props
    ];

    let mut imports: Vec<_> = graph
        .elements
        .values()
        .filter(|e| import_kinds.contains(&e.kind))
        .collect();
    sort_elements_by_source_order(&mut imports);

    // Track imports per scope for IM003 (duplicate detection)
    let mut scope_imports: HashMap<Option<ElementId>, Vec<(String, ElementId)>> = HashMap::new();

    // Track namespace references for IM004 (circular detection)
    let mut import_edges: Vec<(String, String)> = Vec::new();

    for elem in &imports {
        // Prefer unresolved_importedNamespace (full path like "ISQ::length")
        // over the resolved short name, so IM003 duplicate detection compares
        // the full import path rather than just the top-level namespace.
        let imported_ns = elem
            .get_prop("unresolved_importedNamespace")
            .and_then(|v| v.as_str())
            .or_else(|| elem.get_prop("importedNamespace").and_then(|v| v.as_str()))
            .or_else(|| elem.get_prop("importedReference").and_then(|v| v.as_str()));

        let Some(ns_name) = imported_ns else {
            continue;
        };

        // IM001: Unknown namespace
        let ns_exists = namespace_exists(graph, ns_name)
            || library
                .map(|lib_graph| namespace_exists(lib_graph, ns_name))
                .unwrap_or(false)
            || workspace
                .map(|ws_graph| namespace_exists(ws_graph, ns_name))
                .unwrap_or(false);
        if !ns_exists {
            // Standard library packages should be errors — they're expected
            // to be available.  Non-library imports are likely cross-file
            // references that single-file analysis can't resolve.
            let top_ns = ns_name.split("::").next().unwrap_or(ns_name);
            let is_standard_library = matches!(
                top_ns,
                "ScalarValues"
                    | "ISQ"
                    | "SI"
                    | "Collections"
                    | "Performances"
                    | "Parts"
                    | "Connections"
                    | "Ports"
                    | "Interfaces"
                    | "Items"
                    | "Actions"
                    | "States"
                    | "Constraints"
                    | "Requirements"
                    | "Calculations"
                    | "Occurrences"
                    | "Objects"
                    | "Transfers"
                    | "Allocations"
                    | "AnalysisCases"
                    | "UseCases"
                    | "VerificationCases"
                    | "Views"
                    | "Metadata"
                    | "Quantities"
                    | "MeasurementReferences"
                    | "ControlFunctions"
                    | "DataFunctions"
                    | "BaseFunctions"
                    | "NumericalFunctions"
                    | "TrigFunctions"
                    | "SequenceFunctions"
                    | "VectorFunctions"
                    | "Links"
                    | "Metaobjects"
                    | "KerML"
            );
            if is_standard_library {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "import references unknown standard library namespace '{}'",
                        ns_name
                    ))
                    .with_code("IM006")
                    .with_tier(sysml_span::DiagnosticTier::ImportHealth)
                    .with_span(primary_span(elem))
                    .with_note("valid namespaces: ScalarValues, ISQ, SI, Collections, Performances, Parts, Connections, Ports, Interfaces, Items, Actions, States, Constraints, Requirements, Calculations, ..."),
                );
            } else {
                let mut diag = Diagnostic::info(format!(
                    "import references namespace '{}' (unresolved in current workspace context)",
                    ns_name
                ))
                .with_code("IM001")
                .with_tier(sysml_span::DiagnosticTier::ImportHealth)
                .with_span(primary_span(elem));

                diag = if workspace.is_some() {
                    diag.with_note(
                        "checked current file, workspace project files, and loaded standard library; check spelling/case or `[workspace].members`",
                    )
                } else {
                    diag.with_note(
                        "checked current file and loaded standard library only; open the project/workspace folder so cross-file imports can be indexed",
                    )
                };

                diagnostics.push(diag);
            }
        }

        // IM002: Recursive import on non-namespace element
        let is_recursive = elem
            .get_prop("isRecursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_recursive && ns_exists {
            let is_namespace_in = |g: &ModelGraph| {
                g.elements.values().any(|e| {
                    e.name.as_deref() == Some(ns_name)
                        && matches!(
                            e.kind,
                            ElementKind::Package
                                | ElementKind::Namespace
                                | ElementKind::LibraryPackage
                        )
                })
            };
            let target_is_namespace = is_namespace_in(graph)
                || workspace.map(is_namespace_in).unwrap_or(false)
                || library.map(is_namespace_in).unwrap_or(false);
            if !target_is_namespace {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "recursive import on non-namespace element '{}'",
                        ns_name
                    ))
                    .with_code("IM002")
                    .with_tier(sysml_span::DiagnosticTier::ImportHealth)
                    .with_span(primary_span(elem)),
                );
            }
        }

        // Track for IM003: duplicate detection
        scope_imports
            .entry(elem.owner.clone())
            .or_default()
            .push((ns_name.to_owned(), elem.id.clone()));

        // Track for IM004: circular detection
        if let Some(owner) = &elem.owner {
            if let Some(owner_elem) = graph.get_element(owner) {
                if let Some(owner_name) = &owner_elem.name {
                    import_edges.push((owner_name.clone(), ns_name.to_owned()));
                }
            }
        }

        // IM005: Wildcard import that imports 0 visible members
        // NamespaceImport is a wildcard import (imports all members of the target).
        // The batch parser creates Import with isNamespace=true instead.
        let is_namespace_import = elem.kind == ElementKind::NamespaceImport
            || (elem.kind == ElementKind::Import
                && elem
                    .get_prop("isNamespace")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false));
        if is_namespace_import && ns_exists {
            let has_members_in = |g: &ModelGraph| {
                g.elements.values().any(|e| {
                    e.name.as_deref() == Some(ns_name)
                        && g.children_of(&e.id).any(|child| child.name.is_some())
                })
            };
            let target_has_members = has_members_in(graph)
                || workspace.map(has_members_in).unwrap_or(false)
                || library.map(has_members_in).unwrap_or(false);
            if !target_has_members {
                diagnostics.push(
                    Diagnostic::info(format!("unused import '{}'", ns_name))
                        .with_code("IM005")
                        .with_tier(sysml_span::DiagnosticTier::ImportHealth)
                        .with_span(primary_span(elem))
                        .with_tag(sysml_span::DiagnosticTag::Unnecessary),
                );
            }
        }
    }

    // IM003: Duplicate imports in the same scope
    for (scope, scope_import_list) in &scope_imports {
        let mut seen: HashMap<&str, &ElementId> = HashMap::new();
        for (ns_name, import_id) in scope_import_list {
            if let Some(&_first_id) = seen.get(ns_name.as_str()) {
                let scope_name = scope
                    .as_ref()
                    .and_then(|id| graph.get_element(id))
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| "<root>".to_owned());
                if let Some(elem) = graph.get_element(import_id) {
                    diagnostics.push(
                        Diagnostic::info(format!(
                            "duplicate import of namespace '{}' in scope '{}'",
                            ns_name, scope_name
                        ))
                        .with_code("IM003")
                        .with_tier(sysml_span::DiagnosticTier::ImportHealth)
                        .with_span(primary_span(elem)),
                    );
                }
            } else {
                seen.insert(ns_name.as_str(), import_id);
            }
        }
    }

    // IM004: Circular import chain detection
    if let Some(cycle) = detect_cycle(&import_edges) {
        diagnostics.push(
            Diagnostic::error(format!(
                "circular import chain detected: {}",
                cycle.join(" -> ")
            ))
            .with_code("IM004")
            .with_tier(sysml_span::DiagnosticTier::ImportHealth),
        );
    }

    diagnostics
}

fn namespace_exists(graph: &ModelGraph, ns_name: &str) -> bool {
    graph.elements.values().any(|e| {
        let direct_name = e.name.as_deref() == Some(ns_name);
        let qualified_name = e
            .qname
            .as_ref()
            .map(|q| q.to_string() == ns_name)
            .unwrap_or(false);

        (direct_name || qualified_name)
            && matches!(
                e.kind,
                ElementKind::Package
                    | ElementKind::Namespace
                    | ElementKind::LibraryPackage
                    | ElementKind::PartDefinition
                    | ElementKind::PartUsage
                    | ElementKind::ActionDefinition
                    | ElementKind::StateDefinition
                    | ElementKind::RequirementDefinition
                    | ElementKind::VerificationCaseDefinition
                    | ElementKind::ConnectionDefinition
                    | ElementKind::InterfaceDefinition
                    | ElementKind::AllocationDefinition
                    | ElementKind::AttributeDefinition
                    | ElementKind::EnumerationDefinition
                    | ElementKind::ConstraintDefinition
                    | ElementKind::PortDefinition
                    | ElementKind::ItemDefinition
                    | ElementKind::OccurrenceDefinition
                    | ElementKind::FlowDefinition
                    | ElementKind::UseCaseDefinition
                    | ElementKind::AnalysisCaseDefinition
                    | ElementKind::CaseDefinition
                    | ElementKind::ViewDefinition
                    | ElementKind::ViewpointDefinition
                    | ElementKind::RenderingDefinition
                    | ElementKind::MetadataDefinition
            )
    })
}

/// Detect cycles in directed edges using DFS.
fn detect_cycle(edges: &[(String, String)]) -> Option<Vec<String>> {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut nodes: HashSet<&str> = HashSet::new();
    for (from, to) in edges {
        adjacency
            .entry(from.as_str())
            .or_default()
            .push(to.as_str());
        nodes.insert(from.as_str());
        nodes.insert(to.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut in_stack: HashSet<&str> = HashSet::new();
    let mut path: Vec<&str> = Vec::new();

    for &node in &nodes {
        if !visited.contains(node) {
            if let Some(cycle) = dfs_cycle(node, &adjacency, &mut visited, &mut in_stack, &mut path)
            {
                return Some(cycle);
            }
        }
    }

    None
}

#[allow(clippy::unwrap_used, clippy::indexing_slicing)] // invariant: in_stack membership guarantees position exists
fn dfs_cycle<'a>(
    node: &'a str,
    adjacency: &HashMap<&str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    visited.insert(node);
    in_stack.insert(node);
    path.push(node);

    if let Some(neighbors) = adjacency.get(node) {
        for &next in neighbors {
            if !visited.contains(next) {
                if let Some(cycle) = dfs_cycle(next, adjacency, visited, in_stack, path) {
                    return Some(cycle);
                }
            } else if in_stack.contains(next) {
                // Found a cycle - extract it
                let cycle_start = path.iter().position(|&n| n == next).unwrap();
                let mut cycle: Vec<String> = path[cycle_start..]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                cycle.push((*next).to_owned()); // close the cycle
                return Some(cycle);
            }
        }
    }

    path.pop();
    in_stack.remove(node);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Element;

    #[test]
    fn reports_unknown_namespace() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "NonExistentPackage")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("IM001")
                    && d.message.contains("NonExistentPackage")),
            "expected IM001, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_im001_for_existing_namespace() {
        let mut graph = ModelGraph::new();

        let target = Element::new_with_kind(ElementKind::Package).with_name("ScalarValues");
        let target_id = graph.add_element(target);

        // Add a child so it's not empty (avoids IM005)
        let child = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name("Real")
            .with_owner(target_id);
        graph.add_element(child);

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "ScalarValues")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("IM001")),
            "should not have IM001 for existing namespace, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_recursive_on_non_namespace() {
        let mut graph = ModelGraph::new();

        let target = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        graph.add_element(target);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_prop("importedNamespace", "Vehicle")
            .with_prop("isRecursive", crate::Value::Bool(true))
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("IM002") && d.message.contains("Vehicle")),
            "expected IM002, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_im002_for_recursive_on_namespace() {
        let mut graph = ModelGraph::new();

        let target = Element::new_with_kind(ElementKind::Package).with_name("ScalarValues");
        let target_id = graph.add_element(target);

        let child = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name("Real")
            .with_owner(target_id);
        graph.add_element(child);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_prop("importedNamespace", "ScalarValues")
            .with_prop("isRecursive", crate::Value::Bool(true))
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("IM002")),
            "should not have IM002 for recursive on Package, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_duplicate_import() {
        let mut graph = ModelGraph::new();

        let target = Element::new_with_kind(ElementKind::Package).with_name("ScalarValues");
        let target_id = graph.add_element(target);

        let child = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name("Real")
            .with_owner(target_id);
        graph.add_element(child);

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp1 = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id.clone())
            .with_prop("importedNamespace", "ScalarValues")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp1);

        let imp2 = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "ScalarValues")
            .with_span(Span::new("file:///test.sysml", 11, 20));
        graph.add_element(imp2);

        let diags = import_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("IM003") && d.message.contains("ScalarValues")),
            "expected IM003, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_circular_import_chain() {
        let mut graph = ModelGraph::new();

        let pkg_a = Element::new_with_kind(ElementKind::Package).with_name("A");
        let pkg_a_id = graph.add_element(pkg_a);

        let pkg_b = Element::new_with_kind(ElementKind::Package).with_name("B");
        let pkg_b_id = graph.add_element(pkg_b);

        // A imports B
        let imp1 = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_a_id)
            .with_prop("importedNamespace", "B")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp1);

        // B imports A
        let imp2 = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_b_id)
            .with_prop("importedNamespace", "A")
            .with_span(Span::new("file:///test.sysml", 11, 20));
        graph.add_element(imp2);

        let diags = import_health_diagnostics(&graph);
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("IM004") && d.message.contains("circular")),
            "expected IM004, got: {:?}",
            diags
        );
    }

    #[test]
    fn reports_empty_wildcard_import() {
        let mut graph = ModelGraph::new();

        // Package with no named children
        let target = Element::new_with_kind(ElementKind::Package).with_name("EmptyPkg");
        graph.add_element(target);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_prop("importedNamespace", "EmptyPkg")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        let im005 = diags.iter().find(|d| d.code.as_deref() == Some("IM005"));
        assert!(im005.is_some(), "expected IM005, got: {:?}", diags);
        let im005 = im005.unwrap();
        assert!(
            im005.message.contains("unused import"),
            "IM005 should say 'unused import', got: {}",
            im005.message
        );
        assert!(
            im005.tags.contains(&sysml_span::DiagnosticTag::Unnecessary),
            "IM005 should have Unnecessary tag"
        );
    }

    #[test]
    fn no_im005_for_non_empty_namespace() {
        let mut graph = ModelGraph::new();

        let target = Element::new_with_kind(ElementKind::Package).with_name("ScalarValues");
        let target_id = graph.add_element(target);

        let child = Element::new_with_kind(ElementKind::AttributeDefinition)
            .with_name("Real")
            .with_owner(target_id);
        graph.add_element(child);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_prop("importedNamespace", "ScalarValues")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("IM005")),
            "should not have IM005 for namespace with members, got: {:?}",
            diags
        );
    }

    #[test]
    fn im006_is_error_for_standard_library() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "ScalarValues")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        let im006 = diags.iter().find(|d| d.code.as_deref() == Some("IM006"));
        assert!(im006.is_some(), "expected IM006 for ScalarValues");
        assert!(
            im006.unwrap().is_error(),
            "IM006 for standard library should be error level"
        );
    }

    #[test]
    fn im001_is_info_for_cross_file_import() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "CoffeeMachineTypes")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        let im001 = diags.iter().find(|d| d.code.as_deref() == Some("IM001"));
        assert!(im001.is_some(), "expected IM001 for CoffeeMachineTypes");
        assert!(
            !im001.unwrap().is_error(),
            "IM001 for cross-file import should be info, not error"
        );
    }

    #[test]
    fn im001_without_workspace_context_suggests_indexing_workspace() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "CoffeeMachineTypes")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics_with_context(&graph, None, None);
        let im001 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("IM001"))
            .expect("expected IM001");
        assert!(
            im001
                .message
                .contains("unresolved in current workspace context"),
            "IM001 message should use workspace-context wording: {}",
            im001.message
        );
        assert!(
            im001
                .notes
                .iter()
                .any(|n| n.contains("open the project/workspace folder")),
            "IM001 without workspace context should suggest opening/indexing workspace: {:?}",
            im001.notes
        );
    }

    #[test]
    fn im001_with_workspace_context_mentions_workspace_members() {
        let mut graph = ModelGraph::new();
        let workspace = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "CoffeeMachineTypes")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics_with_context(&graph, None, Some(&workspace));
        let im001 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("IM001"))
            .expect("expected IM001");
        assert!(
            im001
                .notes
                .iter()
                .any(|n| n.contains("[workspace].members")),
            "IM001 with workspace context should mention [workspace].members: {:?}",
            im001.notes
        );
    }

    /// P-RA2 Slice 4: every IM* diagnostic from this module must carry the
    /// ImportHealth tier so the readiness filter can withhold them until the
    /// project file set is indexed.
    #[test]
    fn im_diagnostics_tagged_import_health() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);
        let imp = Element::new_with_kind(ElementKind::NamespaceImport)
            .with_owner(pkg_id)
            .with_prop("importedNamespace", "NonExistentPackage")
            .with_span(Span::new("file:///test.sysml", 0, 10));
        graph.add_element(imp);

        let diags = import_health_diagnostics(&graph);
        let im_diags: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.code
                    .as_deref()
                    .map(|c| c.starts_with("IM"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(!im_diags.is_empty(), "expected at least one IM* diagnostic");
        for d in im_diags {
            assert_eq!(
                d.tier,
                sysml_span::DiagnosticTier::ImportHealth,
                "IM diagnostic must carry ImportHealth tier: {:?}",
                d
            );
        }
    }
}
