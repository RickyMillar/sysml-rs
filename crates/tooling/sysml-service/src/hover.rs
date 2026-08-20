//! Hover content generation — element signature, supertypes, doc comments,
//! physics classification, evaluated values.
//!
//! Replaces the bulk of LSP `hover.rs`. The LSP shell keeps:
//!   - import-segment hover (depends on tree-sitter Tree access),
//!   - external library disk-source loading (async tokio::fs),
//!   - the keyword fallback (CST-based, depends on `salsa_tree`),
//! and now consumes `compute_hover` plus the re-exported pure helpers
//! (`build_hover_content`, `extract_doc_comment`, `keyword_documentation`,
//! `append_package_members_preview`, `element_kind_to_hover_label`).
//!
//! Position columns follow the LSP convention: UTF-16 code units, 0-indexed
//! line + character. The service has no `tower-lsp` dependency; the fields
//! are plain `u32`s.

use std::sync::{Arc, Mutex};

use sysml_core::physics::classify::classify_port_definition;
use sysml_core::physics::domain::{PhysicsDomainRegistry, VariableRole};
use sysml_core::{is_package_kind, Element, ElementKind, ModelGraph, RelationshipKind};
use sysml_ide_db::{AnalysisHost, Cancelled};

use crate::evaluation::try_evaluate_value;
use crate::expression_ast::{pretty_print, project_owner};
use crate::goto_definition::{find_element_type, resolve_goto_target};
use crate::position::{offset_to_line_col, position_to_offset};

/// Spans for elements that have no source file (e.g. synthetic library defs).
const SYNTHETIC_FILE: &str = "<synthetic>";

/// Rendered hover information for `(uri, line, col)`.
///
/// Columns follow LSP conventions (UTF-16 code units, 0-indexed). The
/// transport (LSP, MCP, REST) wraps this in its native shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HoverInfo {
    pub markdown: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
}

/// Compute hover for the cursor at `(uri, line, col)`.
///
/// Resolves the element under the cursor, follows relationships, and (for
/// typed usages) jumps to the type definition via a workspace walk. Returns
/// `None` when the cursor is not over a model element — the LSP shell can
/// then fall through to its keyword-fallback or import-segment paths.
pub fn compute_hover(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    line: u32,
    col: u32,
) -> Option<HoverInfo> {
    // Phase 1 — locate the raw element + content under one analysis snapshot.
    // We collect everything we need and then drop the snapshot before any
    // cross-file walk so concurrent edits aren't blocked.
    struct Phase1 {
        in_file_content: String,
        offset: usize,
        raw_element: Element,
        is_def: bool,
        // Resolved-via-relationship element (still in current file's graph).
        resolved_element: Element,
        // Snapshot of the file's graph (used if no cross-file lookup needed).
        in_file_graph: ModelGraph,
        // Type-name for cross-file def-search, if the raw element is a relationship.
        type_name_for_lookup: Option<String>,
    }

    let physics_registry: Arc<PhysicsDomainRegistry>;
    let phase1: Phase1 = {
        // Resolve (SourceFile, project, snapshot) under a SMALL guard, then
        // drop the guard before running any salsa query — a query under the
        // guard serializes every other host user (precedent:
        // `compute_full_diagnostics`).
        let (analysis, sf, project_id) = {
            let guard = host.lock().unwrap();
            let file_id = guard.file_id(uri)?;
            let sf = guard.source_file(file_id)?;
            let project_id = guard.files().project_id(file_id);
            (guard.analysis(), sf, project_id)
        };
        physics_registry = sysml_ide_db::file_physics_registry(analysis.db(), sf).arc();

        let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let offset = position_to_offset(line, col, &content);
            let position_map = analysis.position_map(sf);
            let (element_id, is_def) = position_map.element_at(offset)?;
            let element_id = element_id.clone();
            // Workspace-resolved graph: cross-file types must resolve here
            // (same rule as goto-def; same accessor as diagnostics).
            let resolved_model = analysis.resolve_file_best(sf, project_id);
            let graph = resolved_model.graph();
            let raw = graph.get_element(&element_id)?.clone();
            let resolved = resolve_goto_target(&raw, graph).clone();

            let type_name_for_lookup = if raw.kind.is_relationship() {
                find_element_type(&resolved, graph).or_else(|| {
                    raw.props
                        .get("unresolved_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned())
                })
            } else {
                None
            };

            Some(Phase1 {
                in_file_content: content,
                offset,
                raw_element: raw,
                is_def,
                resolved_element: resolved,
                in_file_graph: graph.clone(),
                type_name_for_lookup,
            })
        }));

        drop(analysis);
        match result {
            Ok(Some(p)) => p,
            _ => return None,
        }
    };

    // Phase 2 — if the raw element was a relationship and we have a
    // type-name, walk the host for a matching definition (cross-file
    // type lookup, mirroring LSP's old `workspace_snapshot.find_by_name`
    // logic).
    let mut hover_element: Element = phase1.resolved_element.clone();
    let mut hover_graph: ModelGraph = phase1.in_file_graph.clone();
    let mut hover_source: String = phase1.in_file_content.clone();

    if let Some(type_name) = &phase1.type_name_for_lookup {
        if let Some((def, def_graph, def_source)) =
            find_definition_across_host(host, uri, type_name)
        {
            hover_element = def;
            hover_graph = def_graph;
            hover_source = def_source;
        }
    }

    // Phase 3 — render markdown.
    let mut markdown = build_hover_content(
        &hover_element,
        &hover_graph,
        &hover_source,
        false,
        phase1.is_def,
        &physics_registry,
    );
    append_package_members_preview(&mut markdown, &hover_element, &hover_graph);

    // Hover range = the raw element's name span (or first span) within the
    // current file. Matches the LSP-side range computation.
    let range_span = phase1
        .raw_element
        .name_span
        .as_ref()
        .filter(|s| s.file == uri && s.start <= phase1.offset && phase1.offset < s.end)
        .or_else(|| {
            phase1
                .raw_element
                .spans
                .iter()
                .find(|s| s.file == uri && s.start <= phase1.offset && phase1.offset < s.end)
        });
    let (line_start, col_start, line_end, col_end) = match range_span {
        Some(span) => {
            let (ls, cs) = offset_to_line_col(span.start, &phase1.in_file_content);
            let (le, ce) = offset_to_line_col(span.end, &phase1.in_file_content);
            (ls, cs, le, ce)
        }
        None => {
            // Fall back to a zero-width range at the cursor offset.
            let (ls, cs) = offset_to_line_col(phase1.offset, &phase1.in_file_content);
            (ls, cs, ls, cs)
        }
    };

    Some(HoverInfo {
        markdown,
        line_start,
        col_start,
        line_end,
        col_end,
    })
}

/// Walk every host file searching for a definition-kind element with the
/// given name. Returns the element, its graph, and that file's content
/// (all owned). Mirrors the LSP-side `workspace_snapshot.find_by_name +
/// salsa_doc(...)` pattern but reads everything from salsa.
fn find_definition_across_host(
    host: &Mutex<AnalysisHost>,
    skip_uri: &str,
    name: &str,
) -> Option<(Element, ModelGraph, String)> {
    // Enumerate (uri, SourceFile) under a SMALL guard, then drop it before
    // the per-file parse loop — parsing every file under the guard
    // serializes every other host user.
    let (analysis, files) = {
        let guard = host.lock().unwrap();
        let files: Vec<sysml_ide_db::SourceFile> = guard
            .files()
            .file_ids()
            .filter(|&file_id| guard.files().uri(file_id).is_some_and(|u| u != skip_uri))
            .filter_map(|file_id| guard.source_file(file_id))
            .collect();
        (guard.analysis(), files)
    };

    for sf in files {

        let parsed = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            analysis.parse_file(sf)
        })) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let graph = parsed.graph();
        let Some(def) = graph
            .elements
            .values()
            .find(|e| e.kind.is_definition() && e.name.as_deref() == Some(name))
        else {
            continue;
        };
        let def_clone = def.clone();
        let graph_clone = graph.clone();
        let content = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            analysis.file_text(sf).to_owned()
        })) {
            Ok(c) => c,
            Err(_) => continue,
        };
        return Some((def_clone, graph_clone, content));
    }
    None
}

/// Build hover content for an element.
///
/// Produces a markdown string showing:
/// - A SysML code block with the element signature (kind, name, type, multiplicity)
/// - Source file location (reference sites only)
/// - Supertype information for definitions (reference sites only)
/// - Doc comments if present before the element (reference sites only)
///
/// When `is_definition_site` is true, shows minimal info (signature + qualified name)
/// since the user is already looking at the definition. Reference sites show full details.
#[tracing::instrument(level = "debug", skip(element, graph, source))]
pub fn build_hover_content(
    element: &Element,
    graph: &ModelGraph,
    source: &str,
    is_nearest: bool,
    is_definition_site: bool,
    physics_registry: &PhysicsDomainRegistry,
) -> String {
    let name = element.name.as_deref().unwrap_or("<unnamed>");
    let kind_label = element_kind_to_hover_label(&element.kind);

    let mut signature = String::new();
    signature.push_str(kind_label);
    signature.push(' ');
    signature.push_str(name);

    let type_name = find_element_type(element, graph);
    if let Some(ref tn) = type_name {
        signature.push_str(" : ");
        signature.push_str(tn);
    }

    if let Some(mult) = element.props.get("multiplicity").and_then(|v| v.as_str()) {
        signature.push_str(&format!(" [{}]", mult));
    } else if let Some(lower) = element
        .get_prop("multiplicity_lower")
        .and_then(|v| v.as_int())
    {
        let upper_str = element
            .get_prop("multiplicity_upper")
            .map(|v| match v.as_int() {
                Some(u) => u.to_string(),
                None => "*".to_owned(),
            })
            .unwrap_or_else(|| "*".to_owned());
        let display = if upper_str == lower.to_string() {
            format!("{}", lower)
        } else {
            format!("{}..{}", lower, upper_str)
        };
        signature.push_str(&format!(" [{}]", display));
    }

    let mut content = String::new();

    if is_nearest {
        content.push_str("*(nearest match)*\n\n");
    }

    content.push_str("```sysml\n");
    content.push_str(&signature);
    content.push_str("\n```\n");

    // Qualified name if different from simple name.
    if let Some(qstr) = element.qname.as_ref().map(|q| q.to_string()).or_else(|| {
        graph
            .build_qualified_name(&element.id)
            .map(|q| q.to_string())
    }) {
        if qstr != name {
            content.push_str(&format!("\n`{}`\n", qstr));
        }
    }

    if let Some(span) = element.spans.first() {
        if span.file.to_lowercase().contains("library") {
            content.push_str("\n*(from standard library)*\n");
        }
    }

    if !is_definition_site {
        if let Some(span) = element.spans.first() {
            if !span.file.is_empty() && span.file != SYNTHETIC_FILE {
                let display_path = if let Some(path_part) = span.file.strip_prefix("file://") {
                    std::path::Path::new(path_part)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or(path_part)
                } else {
                    std::path::Path::new(&span.file)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or(&span.file)
                };
                let line_num = source
                    .get(..span.start)
                    .map(|before| before.matches('\n').count() + 1)
                    .unwrap_or(1);
                content.push_str(&format!("\nDefined in `{}:{}`\n", display_path, line_num));
            }
        }

        if element.kind.is_definition() {
            let mut chain: Vec<String> = Vec::new();
            let mut current_ids: Vec<_> = graph
                .outgoing(&element.id)
                .filter(|rel| rel.kind == RelationshipKind::Specialize)
                .map(|rel| rel.target.clone())
                .collect();

            for _depth in 0..3 {
                if current_ids.is_empty() {
                    break;
                }
                let mut next_ids = Vec::new();
                for id in &current_ids {
                    if let Some(e) = graph.get_element(id) {
                        if let Some(name) = &e.name {
                            chain.push(name.clone());
                        }
                        for rel in graph.outgoing(id) {
                            if rel.kind == RelationshipKind::Specialize {
                                next_ids.push(rel.target.clone());
                            }
                        }
                    }
                }
                current_ids = next_ids;
            }

            if !chain.is_empty() {
                content.push_str(&format!(
                    "\nType hierarchy: `{}`\n",
                    chain.join("` \u{2192} `")
                ));
            }
        }

        if element.kind.is_definition() {
            let mut inherited_count = 0usize;
            let supertypes: Vec<_> = graph
                .outgoing(&element.id)
                .filter(|rel| rel.kind == RelationshipKind::Specialize)
                .map(|rel| rel.target.clone())
                .collect();
            for supertype_id in &supertypes {
                inherited_count += graph.owned_members(supertype_id).count();
            }
            if inherited_count > 0 {
                content.push_str(&format!(
                    "\n*(+ {} inherited member{})*\n",
                    inherited_count,
                    if inherited_count == 1 { "" } else { "s" }
                ));
            }
        }

        if element.kind == ElementKind::PortUsage {
            if let Some(dir) = element
                .get_prop("effectiveDirection")
                .or_else(|| element.get_prop("direction"))
                .and_then(|v| v.as_str())
            {
                content.push_str(&format!("\n**Direction**: `{}`\n", dir));
            }
            if let Some(def) = element
                .get_prop("portDefinition")
                .and_then(|v| v.as_str())
            {
                content.push_str(&format!("\n**Port Definition**: `{}`\n", def));
            }
            if element
                .get_prop("isConjugated")
                .and_then(|v| v.as_bool())
                == Some(true)
            {
                content.push_str("\n**Conjugated** (direction reversed)\n");
            }
        }

        if let Some(section) = try_physics_hover(element, graph, physics_registry) {
            content.push_str(&format!("\n{}\n", section));
        }

        if let Some(value) = try_evaluate_value(element, graph) {
            content.push_str(&format!("\n**Value**: `{}`\n", value));
        }

        // Structured expression projection: pretty-print + JSON for KaTeX clients.
        if let Some(result) = project_owner(element, graph) {
            if let Some(ref ast) = result.ast {
                let plain = pretty_print(ast);
                content.push_str(&format!("\n**Expression**: `{}`\n", plain));
                if let Ok(json) = serde_json::to_string(ast) {
                    content.push_str("\n```sysml-expression-ast\n");
                    content.push_str(&json);
                    content.push_str("\n```\n");
                }
            }
        }

        if let Some(doc) = extract_doc_comment(element, source) {
            content.push_str(&format!("\n---\n\n{}\n", doc));
        }
    }

    content
}

/// Build the physics domain classification hover section for ports.
///
/// Returns `None` for non-port elements or when classification yields no
/// domain.
fn try_physics_hover(
    element: &Element,
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
) -> Option<String> {
    let def_name = match element.kind {
        ElementKind::PortDefinition => element.name.clone(),
        ElementKind::PortUsage => element
            .get_prop("portDefinition")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                graph.children_of(&element.id).find_map(|child| {
                    if child.kind == ElementKind::FeatureTyping {
                        child
                            .get_prop("unresolved_type")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    } else {
                        None
                    }
                })
            }),
        _ => return None,
    }?;

    let classification = classify_port_definition(&def_name, graph, registry);
    let domain = classification.domain?;

    let mut section = format!("**Physics**: {} domain\n", domain);

    for feature in &classification.features {
        let role_label = match feature.role {
            VariableRole::Effort => "effort (shared variable)",
            VariableRole::Flow => "flow (conserved variable)",
            _ => continue,
        };
        section.push_str(&format!("- `{}` — {}\n", feature.name, role_label));
    }

    let confidence_label = match classification.confidence {
        sysml_core::physics::ClassificationConfidence::Declared => {
            "declared (@Signal/@PowerPort metadata)"
        }
        sysml_core::physics::ClassificationConfidence::ISQTyped => "ISQ-typed",
        sysml_core::physics::ClassificationConfidence::NameHeuristic => "name heuristic",
        sysml_core::physics::ClassificationConfidence::Unknown => "unknown",
    };
    section.push_str(&format!("- *Classification*: {}\n", confidence_label));

    Some(section)
}

/// Extract a doc comment preceding an element from the source text.
pub fn extract_doc_comment(element: &Element, source: &str) -> Option<String> {
    let span = element.spans.first()?;
    if span.start == 0 || span.file == SYNTHETIC_FILE {
        return None;
    }

    // `get(..)` avoids panics when spans are stale or land mid-codepoint.
    let before = source.get(..span.start)?;
    let trimmed = before.trim_end();

    if trimmed.ends_with("*/") {
        let comment_end = trimmed.len();
        if let Some(start_idx) = trimmed.rfind("/**") {
            let comment = &trimmed[start_idx + 3..comment_end - 2];
            let cleaned: Vec<&str> = comment
                .lines()
                .map(|line| line.trim().trim_start_matches('*').trim())
                .filter(|line| !line.is_empty())
                .collect();
            if !cleaned.is_empty() {
                return Some(cleaned.join("\n"));
            }
        }
    }

    let lines: Vec<&str> = before.lines().collect();
    let mut doc_lines = Vec::new();
    for line in lines.iter().rev() {
        let trimmed_line = line.trim();
        if trimmed_line.starts_with("//") {
            let comment = trimmed_line.trim_start_matches('/').trim();
            doc_lines.push(comment);
        } else if trimmed_line.is_empty() {
            if !doc_lines.is_empty() {
                break;
            }
        } else {
            break;
        }
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Append a one-line preview of package member names to a hover content
/// buffer. No-op for non-package elements or empty packages.
pub fn append_package_members_preview(
    content: &mut String,
    element: &Element,
    graph: &ModelGraph,
) {
    if !is_package_kind(element.kind.clone()) {
        return;
    }

    let mut members: Vec<String> = graph
        .owned_members(&element.id)
        .filter_map(|member| member.name.clone())
        .collect();
    if members.is_empty() {
        return;
    }
    members.sort();
    members.dedup();

    let preview: Vec<String> = members.iter().take(8).cloned().collect();
    let suffix = if members.len() > preview.len() {
        ", ..."
    } else {
        ""
    };
    content.push_str(&format!("\nContains: {}{}\n", preview.join(", "), suffix));
}

/// Map an `ElementKind` to a human-readable hover label.
pub fn element_kind_to_hover_label(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::Package => "package",
        ElementKind::LibraryPackage => "library package",
        ElementKind::PartDefinition => "part def",
        ElementKind::PartUsage => "part",
        ElementKind::AttributeDefinition => "attribute def",
        ElementKind::AttributeUsage => "attribute",
        ElementKind::ActionDefinition => "action def",
        ElementKind::ActionUsage => "action",
        ElementKind::StateDefinition => "state def",
        ElementKind::StateUsage => "state",
        ElementKind::PortDefinition => "port def",
        ElementKind::PortUsage => "port",
        ElementKind::ConnectionDefinition => "connection def",
        ElementKind::ConnectionUsage => "connection",
        ElementKind::InterfaceDefinition => "interface def",
        ElementKind::InterfaceUsage => "interface",
        ElementKind::ItemDefinition => "item def",
        ElementKind::ItemUsage => "item",
        ElementKind::EnumerationDefinition => "enum def",
        ElementKind::EnumerationUsage => "enum",
        ElementKind::RequirementDefinition => "requirement def",
        ElementKind::RequirementUsage => "requirement",
        ElementKind::ConstraintDefinition => "constraint def",
        ElementKind::ConstraintUsage => "constraint",
        ElementKind::DataType => "datatype",
        ElementKind::AllocationDefinition => "allocation def",
        ElementKind::AllocationUsage => "allocation",
        ElementKind::OccurrenceDefinition => "occurrence def",
        ElementKind::OccurrenceUsage => "occurrence",
        ElementKind::CalculationDefinition => "calc def",
        ElementKind::CalculationUsage => "calc",
        ElementKind::ConcernDefinition => "concern def",
        ElementKind::ConcernUsage => "concern",
        ElementKind::FlowUsage => "flow",
        ElementKind::TransitionUsage => "transition",
        ElementKind::SendActionUsage => "send action",
        ElementKind::AcceptActionUsage => "accept action",
        ElementKind::PerformActionUsage => "perform action",
        ElementKind::AssignmentActionUsage => "assign action",
        ElementKind::IfActionUsage => "if action",
        ElementKind::WhileLoopActionUsage => "while action",
        ElementKind::ForLoopActionUsage => "for action",
        ElementKind::AnalysisCaseDefinition => "analysis case def",
        ElementKind::AnalysisCaseUsage => "analysis case",
        ElementKind::UseCaseDefinition => "use case def",
        ElementKind::UseCaseUsage => "use case",
        ElementKind::VerificationCaseDefinition => "verification case def",
        ElementKind::VerificationCaseUsage => "verification case",
        ElementKind::ViewDefinition => "view def",
        ElementKind::ViewUsage => "view",
        ElementKind::RenderingDefinition => "rendering def",
        ElementKind::RenderingUsage => "rendering",
        ElementKind::MetadataDefinition => "metadata def",
        ElementKind::MetadataUsage => "metadata",
        _ if kind.is_definition() => "def",
        _ if kind.is_usage() => "usage",
        _ => "element",
    }
}

/// Provide documentation for SysML keywords and syntax nodes (used by the
/// LSP keyword fallback when no model element is present).
pub fn keyword_documentation(kind: &str) -> Option<&'static str> {
    match kind {
        "part" => Some("**part** — Declares a part usage, a composite structural element"),
        "part_def" | "part def" => {
            Some("**part def** — Defines a part definition (structural type)")
        }
        "def" => Some("**def** — Marks a definition (type declaration)"),
        "attribute" => Some("**attribute** — Declares an attribute (value property)"),
        "action" => Some("**action** — Declares an action usage (behavioral element)"),
        "state" => Some("**state** — Declares a state usage (state machine element)"),
        "port" => Some("**port** — Declares a port usage (interaction point)"),
        "connection" => Some("**connection** — Declares a connection between ports"),
        "interface" => Some("**interface** — Declares an interface definition or usage"),
        "item" => Some("**item** — Declares an item usage (discrete element)"),
        "package" => Some("**package** — Declares a namespace package"),
        "import" => Some("**import** — Imports elements from another namespace"),
        "specializes" | ":>" => Some("**specializes** (`:>`) — Inherits from a supertype"),
        "redefines" | ":>>" => Some("**redefines** (`:>>`) — Redefines an inherited feature"),
        "subsets" => Some("**subsets** — Subsets a feature of the supertype"),
        "references" | "::>" => Some("**references** (`::>`) — References another element"),
        "requirement" => Some("**requirement** — Declares a requirement definition or usage"),
        "constraint" => Some("**constraint** — Declares a constraint expression"),
        "enum" => Some("**enum** — Declares an enumeration definition or usage"),
        "allocation" => Some("**allocation** — Declares an allocation relationship"),
        "occurrence" => Some("**occurrence** — Declares an occurrence definition or usage"),
        "abstract" => Some("**abstract** — Marks an element as abstract (cannot be instantiated)"),
        "in" => Some("**in** — Marks a feature as an input parameter"),
        "out" => Some("**out** — Marks a feature as an output parameter"),
        "inout" => Some("**inout** — Marks a feature as both input and output"),
        "entry" => Some("**entry** — Entry action of a state"),
        "exit" => Some("**exit** — Exit action of a state"),
        "do" => Some("**do** — Do activity within a state"),
        "transition" => Some("**transition** — Declares a state transition"),
        "if" => Some("**if** — Conditional guard or decision"),
        "then" => Some("**then** — Action after a guard is true"),
        "else" => Some("**else** — Alternative branch"),
        "first" => Some("**first** — Marks the first state in a state machine"),
        "flow" => Some("**flow** — Declares a flow connection"),
        "bind" => Some("**bind** — Declares a binding connector"),
        "satisfy" => Some("**satisfy** — Satisfies a requirement"),
        "verify" => Some("**verify** — Verifies a requirement"),
        "calc" => Some("**calc** — Declares a calculation definition or usage"),
        "case" => Some("**case** — Declares a use case or analysis case"),
        "concern" => Some("**concern** — Declares a stakeholder concern"),
        "doc" => Some("**doc** — Declares a documentation comment element"),
        "comment" => Some("**comment** — A comment node in the source"),
        "typing" | ":" => Some("**:** — Typed by (feature typing relationship)"),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

    fn element_by_name(source: &str, name: &str) -> Element {
        let parser = TreeSitterParser::new();
        let tree = parser
            .parse_tree(source)
            .expect("tree-sitter should parse test source");
        let result = build_model_graph(&tree, source, "file:///test.sysml");
        result
            .graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .expect("named element should exist")
            .clone()
    }

    fn graph_for_source(source: &str) -> ModelGraph {
        let parser = TreeSitterParser::new();
        let tree = parser
            .parse_tree(source)
            .expect("tree-sitter should parse test source");
        build_model_graph(&tree, source, "file:///test.sysml").graph
    }

    #[test]
    fn extract_doc_comment_handles_unicode_comment() {
        let source = "package Test {\n  // docs with arrow \u{2192}\n  part def Vehicle {}\n}\n";
        let element = element_by_name(source, "Vehicle");
        let doc = extract_doc_comment(&element, source);
        assert_eq!(doc.as_deref(), Some("docs with arrow \u{2192}"));
    }

    #[test]
    fn hover_reference_site_shows_source_location() {
        let source = "package Vehicles {\n  part def Vehicle {}\n}\n";
        let graph = graph_for_source(source);
        let element = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("Vehicle"))
            .expect("Vehicle should exist");

        let registry = PhysicsDomainRegistry::new();
        let content = build_hover_content(element, &graph, source, false, false, &registry);
        assert!(
            content.contains("Defined in `test.sysml:"),
            "hover should show source location for reference sites, got: {}",
            content
        );
    }

    #[test]
    fn hover_definition_site_omits_source_location() {
        let source = "part def Vehicle {}\n";
        let graph = graph_for_source(source);
        let element = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("Vehicle"))
            .expect("Vehicle should exist");

        let registry = PhysicsDomainRegistry::new();
        let content = build_hover_content(element, &graph, source, false, true, &registry);
        assert!(
            !content.contains("Defined in"),
            "hover should NOT show source location at definition site, got: {}",
            content
        );
    }

    #[test]
    fn extract_doc_comment_non_char_boundary_span_is_safe() {
        let source = "package Test {\n  // docs with arrow \u{2192}\n  part def Vehicle {}\n}\n";
        let mut element = element_by_name(source, "Vehicle");
        let arrow_idx = source
            .find('\u{2192}')
            .expect("unicode arrow should be present");
        element
            .spans
            .first_mut()
            .expect("element should have span")
            .start = arrow_idx + 1; // inside multibyte UTF-8 sequence

        assert_eq!(extract_doc_comment(&element, source), None);
    }
}
