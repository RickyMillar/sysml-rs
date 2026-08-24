//! Shared container rendering helpers.
//!
//! Provides reusable functions for container expand/collapse behavior that is
//! common across multiple view generators (General, Geometry, Browser, etc.).
//!
//! Generator-specific behavior (sub-diagram islands, pseudo-states, fixed
//! positioning) stays in each generator. This module handles only the patterns
//! that are identical or near-identical across generators.

use sysml_core::{Element, ElementId, ElementKind, ModelGraph};

const DOC_WRAP_WIDTH: usize = 48;
const DOC_MAX_LINES: usize = 8;

/// Wrap documentation text into lines of ~`DOC_WRAP_WIDTH` chars, word-boundary aware.
/// Truncates after `DOC_MAX_LINES` lines. Returns wrapped lines with `/* */` on first/last.
pub(crate) fn wrap_doc_text(raw: &str) -> Vec<String> {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut remaining = collapsed.as_str();

    while !remaining.is_empty() && lines.len() < DOC_MAX_LINES {
        if remaining.len() <= DOC_WRAP_WIDTH {
            lines.push(remaining.to_owned());
            remaining = "";
        } else {
            // Find a safe byte index that doesn't split a multi-byte UTF-8 char
            let mut safe_width = DOC_WRAP_WIDTH;
            while safe_width > 0 && !remaining.is_char_boundary(safe_width) {
                safe_width -= 1;
            }
            // Find word boundary near wrap width
            let break_at = remaining[..safe_width]
                .rfind(' ')
                .unwrap_or(safe_width);
            let (line, rest) = remaining.split_at(break_at);
            lines.push(line.to_owned());
            remaining = rest.trim_start();
        }
    }

    let truncated = !remaining.is_empty();

    // Format with /* */ wrapping
    if lines.len() == 1 {
        let text = &lines[0];
        if truncated {
            return vec![format!("/* {}... */", text)];
        }
        return vec![format!("/* {} */", text)];
    }

    let last = lines.len() - 1;
    lines.iter().enumerate().map(|(i, line)| {
        if i == 0 {
            format!("/* {}", line)
        } else if i == last {
            if truncated {
                format!("   {}... */", line)
            } else {
                format!("   {} */", line)
            }
        } else {
            format!("   {}", line)
        }
    }).collect()
}

use crate::ir::types::DiagramNode;
use crate::view_text;
use crate::visual_kind::{self as classify, VisualKind};

/// Deterministic, source-ordered children of an element (C13).
///
/// `ModelGraph::children_of` iterates a hash-set index — order is
/// nondeterministic across runs (ElementIds are random per parse). Every
/// composer collection point that produces sibling rows/nodes must go through
/// this helper so member order matches source declaration order.
/// Elements without spans sort last, tie-broken by name then id.
pub(crate) fn ordered_children<'g>(
    graph: &'g ModelGraph,
    id: &ElementId,
) -> Vec<&'g Element> {
    let mut children: Vec<&Element> = graph.children_of(id).collect();
    sysml_core::element_ordering::sort_elements_by_source_order(&mut children);
    children
}

/// C11: whether a child element is a VALUE-feature — an attribute-family
/// usage that is a leaf (no own structure) and therefore renders as a text
/// compartment row (`name : Type = default`), never as a nested child node.
///
/// Contract §D; spec graphical BNF (SysML-graphical-bnf.kgbnf §8.2.3.7):
/// `attributes-compartment-element = el-prefix? UsagePrefix usage-cp` with
/// `usage-cp = usageDeclaration ValuePart?` — compartment items are TEXT.
pub(crate) fn is_value_feature(element: &Element, graph: &ModelGraph) -> bool {
    if !element.kind.is_usage() {
        return false;
    }
    if VisualKind::from_element_kind(&element.kind) != VisualKind::Attribute {
        return false;
    }
    // Leaf check: a structured attribute (own bdd-relevant structural
    // children) keeps node treatment. Annotations don't count as structure.
    !graph.children_of(&element.id).any(|c| {
        !classify::is_membership_kind(&c.kind)
            && !classify::is_import_kind(&c.kind)
            && !matches!(c.kind, ElementKind::Comment | ElementKind::Documentation)
            && classify::is_bdd_relevant(c)
    })
}

/// Extract a display string for an element's value/default (the `= v` tail).
///
/// Prefers the literal `value` prop lifted by the parser (plus its `unit`
/// annotation), falling back to the pretty-printed value-expression AST
/// subtree for non-literal defaults.
pub(crate) fn feature_value_text(element: &Element, graph: &ModelGraph) -> Option<String> {
    if let Some(v) = element.get_prop("value") {
        let raw = v.to_string();
        let mut s = raw.trim_matches('"').to_owned();
        if s.is_empty() {
            return None;
        }
        if let Some(unit) = element.get_prop("unit").and_then(|u| u.as_str()) {
            s = format!("{} [{}]", s, unit);
        }
        return Some(s);
    }
    // Non-literal defaults are kept as an expression AST subtree.
    sysml_core::expression_pretty::pretty_print_owner(element, graph).filter(|s| !s.is_empty())
}

/// Format an element as compartment text for collapsed container rendering.
///
/// Per the SysML v2 graphical BNF, compartment elements are rendered as text:
///   `keyword name : Type`
///
/// When the compartment's `allowed_child_kinds()` has exactly one entry matching
/// this element, the keyword is omitted (the section header already implies it).
/// E.g., in `comp:attributes`, renders `name : Type` instead of `attribute name : Type`.
///
/// Handles special cases: TransitionUsage, EnumerationUsage, Comment/Documentation.
pub(crate) fn compartment_text_for_element(
    element: &Element,
    graph: &ModelGraph,
    compartment: classify::CompartmentKind,
) -> String {
    let name = crate::view_text::element_display_name(element, graph);

    match element.kind {
        // Transitions: "source then target" with optional trigger/guard
        ElementKind::TransitionUsage => {
            let source_name = element
                .get_prop("source")
                .map(|v| v.to_string().trim_matches('"').to_owned())
                .unwrap_or_else(|| "?".to_owned());
            let target_name = element
                .get_prop("target")
                .map(|v| v.to_string().trim_matches('"').to_owned())
                .unwrap_or_else(|| "?".to_owned());
            // Trigger/guard are TransitionFeatureMembership-wrapped children;
            // derive the text from them (one home: transition_feature_text).
            let trigger = graph
                .transition_feature_text(&element.id, "trigger")
                .map(|t| format!(" accept {}", t));
            let guard = graph
                .transition_feature_text(&element.id, "guard")
                .map(|g| format!(" if {}", g));
            format!(
                "{}{}{} then {}",
                source_name,
                trigger.unwrap_or_default(),
                guard.unwrap_or_default(),
                target_name
            )
        }

        // Enumerations: just show the literal name (no keyword clutter)
        ElementKind::EnumerationUsage if element.owner.is_some() => name.to_owned(),

        // Comments/Documentation: wrap text into multiple short lines.
        ElementKind::Comment | ElementKind::Documentation => {
            let raw = element
                .get_prop("body")
                .or_else(|| element.get_prop("documentation"))
                .map(|v| v.to_string().trim_matches('"').to_owned())
                .unwrap_or_else(|| name.to_owned());
            let lines = wrap_doc_text(&raw);
            if lines.is_empty() {
                format!("/* {} */", name)
            } else {
                lines[0].clone()
            }
        }

        // Default: "name : Type" (keyword omitted if section implies it)
        _ => {
            let type_name = element
                .get_prop("unresolved_type")
                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                .or_else(|| {
                    graph
                        .children_of(&element.id)
                        .find(|c| {
                            c.kind == ElementKind::FeatureTyping
                                || c.kind.is_subtype_of(ElementKind::FeatureTyping)
                        })
                        .and_then(|ft| {
                            ft.get_prop("unresolved_type")
                                .and_then(|v| v.as_str().map(|s| s.to_owned()))
                        })
                });

            // Omit keyword when the compartment already implies the element type.
            // E.g., in comp:attributes all children are attributes — no need to
            // prefix every line with "attribute".
            let visual = classify::VisualKind::from_element_kind(&element.kind);
            let allowed = compartment.allowed_child_kinds();
            let keyword_redundant = allowed.len() == 1 && allowed[0] == visual;

            let keyword = if keyword_redundant {
                String::new()
            } else {
                format!("{} ", classify::element_keyword(&element.kind))
            };

            let mut text = match type_name.as_deref() {
                Some(t) if !t.is_empty() => format!("{}{} : {}", keyword, name, t),
                _ => format!("{}{}", keyword, name),
            };

            // `= default` tail (spec graphical BNF: `usage-cp =
            // usageDeclaration ValuePart?`). Value features get the full
            // extraction (literal prop or pretty-printed AST); other kinds
            // only surface an explicit literal `value` prop — pretty-printing
            // the first expression child of e.g. a calc would misread its
            // body as a value.
            let value = if visual == VisualKind::Attribute {
                feature_value_text(element, graph)
            } else {
                element
                    .get_prop("value")
                    .map(|v| v.to_string().trim_matches('"').to_owned())
                    .filter(|s| !s.is_empty())
            };
            if let Some(v) = value {
                text.push_str(&format!(" = {}", v));
            }
            text
        }
    }
}

/// Apply source location metadata (URI, range, tooltip) from an element
/// to a diagram node. This is the most commonly repeated pattern across
/// all generators.
pub(crate) fn apply_source_metadata(
    node: &mut DiagramNode,
    element: &Element,
    graph: &ModelGraph,
) {
    // Source location now lives only in the ViewModel text-map (3.15); the node
    // no longer carries source_uri/source_range.
    node.tooltip = view_text::tooltip_text(element, graph);
}

/// Append the requirement-specific compartments to a node — the shared notation
/// home for requirement-bearing elements (reqId, the Constraints compartment,
/// subject, the assume/require split, and nested requirements).
///
/// This folds in what `general.rs` previously emitted inline (the Constraints
/// compartment) plus the remaining compartments the legacy
/// `requirements::generate_requirement_node` built, so the General render path
/// reaches notation parity with the peer generator (Phase 5 then just deletes
/// `requirements.rs`). Element-id suffixes match `requirements.rs` exactly so
/// the two paths produce consistent output.
///
/// `nested_builder` produces a full `DiagramNode` for each nested requirement
/// (general.rs passes its own `generate_node`), matching requirements.rs's
/// recursive nested handling. The metadata compartment is intentionally NOT
/// emitted here — general.rs already renders MetadataUsage children for every
/// element, so emitting it here would double-render.
///
/// Returns `true` when nested requirements were rendered (the caller may use
/// this to surface an expand affordance).
pub(crate) fn apply_requirement_compartments(
    node: &mut DiagramNode,
    element: &Element,
    graph: &ModelGraph,
    nested_builder: impl Fn(&ModelGraph, &Element) -> DiagramNode,
) -> bool {
    use crate::ir::types::{CompartmentItemSource, DiagramChild};
    use crate::visual_kind::CompartmentKind;

    let id = element.id.to_string();

    // reqId label → General compartment text "id = {reqId}".
    if let Some(req_id) = element.get_prop("reqId") {
        let req_id_text = req_id.to_string().trim_matches('"').to_owned();
        if !req_id_text.is_empty() {
            node.children.push(DiagramChild::Text {
                compartment: CompartmentKind::General,
                text: format!("id = {}", req_id_text),
                element_id: format!("{}/reqId", id),
                source: CompartmentItemSource::Owned,
            });
        }
    }

    // Constraints compartment: documentation and text properties.
    let mut constraint_texts = Vec::new();
    if let Some(doc) = element.get_prop("documentation") {
        constraint_texts.push(DiagramChild::Text {
            compartment: CompartmentKind::Constraints,
            text: doc.to_string().trim_matches('"').to_owned(),
            element_id: format!("{}/constraints/doc", id),
            source: CompartmentItemSource::Owned,
        });
    }
    if let Some(text) = element.get_prop("text") {
        constraint_texts.push(DiagramChild::Text {
            compartment: CompartmentKind::Constraints,
            text: text.to_string().trim_matches('"').to_owned(),
            element_id: format!("{}/constraints/text", id),
            source: CompartmentItemSource::Owned,
        });
    }
    if !constraint_texts.is_empty() {
        node.children.push(DiagramChild::Compartment {
            kind: CompartmentKind::Constraints,
            children: constraint_texts,
        });
    }

    // Subject compartment.
    let subjects: Vec<_> = ordered_children(graph, &element.id)
        .into_iter()
        .filter(|c| {
            c.get_prop("isSubject")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || c.kind == ElementKind::ReferenceUsage && c.name.as_deref() == Some("subject")
        })
        .collect();
    if !subjects.is_empty() {
        let children: Vec<_> = subjects
            .iter()
            .enumerate()
            .map(|(i, s)| DiagramChild::Text {
                compartment: CompartmentKind::Subject,
                text: format!("subject {}", s.name.as_deref().unwrap_or("unnamed")),
                element_id: format!("{}/subject/{}", id, i),
                source: CompartmentItemSource::Owned,
            })
            .collect();
        node.children.push(DiagramChild::Compartment {
            kind: CompartmentKind::Subject,
            children,
        });
    }

    // Assume/require constraint compartments.
    {
        let mut assume_children = Vec::new();
        let mut require_children = Vec::new();
        for (i, child) in ordered_children(graph, &element.id)
            .into_iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
                )
            })
            .enumerate()
        {
            let is_assume = child
                .get_prop("isAssume")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let diagram_child = DiagramChild::Text {
                compartment: if is_assume {
                    CompartmentKind::AssumeConstraints
                } else {
                    CompartmentKind::RequireConstraints
                },
                text: child.name.as_deref().unwrap_or("constraint").to_owned(),
                element_id: format!("{}/constraint/{}", id, i),
                source: CompartmentItemSource::Owned,
            };
            if is_assume {
                assume_children.push(diagram_child);
            } else {
                require_children.push(diagram_child);
            }
        }
        if !assume_children.is_empty() {
            node.children.push(DiagramChild::Compartment {
                kind: CompartmentKind::AssumeConstraints,
                children: assume_children,
            });
        }
        if !require_children.is_empty() {
            node.children.push(DiagramChild::Compartment {
                kind: CompartmentKind::RequireConstraints,
                children: require_children,
            });
        }
    }

    // Nested requirements — full nested nodes via the caller's node builder
    // (recursion), matching requirements.rs.
    let nested_reqs: Vec<_> = ordered_children(graph, &element.id)
        .into_iter()
        .filter(|c| classify::is_requirement_kind(&c.kind))
        .collect();
    let has_nested = !nested_reqs.is_empty();
    if has_nested {
        let nested_children: Vec<_> = nested_reqs
            .into_iter()
            .map(|nested| DiagramChild::Node(nested_builder(graph, nested)))
            .collect();
        node.children.push(DiagramChild::Compartment {
            kind: CompartmentKind::Requirements,
            children: nested_children,
        });
    }

    has_nested
}

/// Whether a child element is consumed by `apply_requirement_compartments`
/// (subject, assume/require constraint, or nested requirement) and therefore
/// must NOT also be rendered via the generic owned-children path — otherwise
/// it would be double-emitted.
pub(crate) fn is_requirement_compartment_child(child: &Element) -> bool {
    let is_subject = child
        .get_prop("isSubject")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || (child.kind == ElementKind::ReferenceUsage && child.name.as_deref() == Some("subject"));
    is_subject
        || matches!(
            child.kind,
            ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
        )
        || classify::is_requirement_kind(&child.kind)
}

/// Render non-port children as nested `DiagramChild::Node` elements (expanded mode).
///
/// Calls the generator-specific `generate_fn` for each non-port child.
/// Skips: ports (handled separately), MetadataUsage (handled by metadata compartment),
/// and unnamed redefinition attributes (handled by redefinitions compartment).
///
/// C11/C12a: VALUE-features (attribute-family leaves) and Comment/Documentation
/// children are NOT promoted to nested nodes — they render as text compartment
/// rows exactly like the collapsed path (contract §D: compartment items are
/// textual; a part def with 23 scalar attributes must not become 23 boxes).
pub(crate) fn render_expanded_children(
    graph: &ModelGraph,
    parent_kind: &ElementKind,
    owned: &[&Element],
    expanded_ids: &std::collections::HashSet<String>,
    node: &mut DiagramNode,
    generate_fn: impl Fn(&ModelGraph, &Element, &std::collections::HashSet<String>) -> DiagramNode,
) {
    use crate::ir::types::DiagramChild;
    let parent_gk = VisualKind::from_element_kind(parent_kind);
    for child in owned {
        if classify::is_port_kind(&child.kind) {
            continue;
        }
        // Skip MetadataUsage — rendered in metadata compartment
        if child.kind == ElementKind::MetadataUsage {
            continue;
        }
        // Skip unnamed redefinition attributes — rendered in redefinitions compartment
        if child.name.is_none() && is_redefinition_child(child, graph) {
            continue;
        }
        // C12a: doc/comment bodies are text, never "unnamed" note nodes.
        // C11: value features are compartment rows, never child boxes.
        if matches!(child.kind, ElementKind::Comment | ElementKind::Documentation)
            || is_value_feature(child, graph)
        {
            render_child_text_row(graph, &parent_gk, child, node);
            continue;
        }
        let child_node = generate_fn(graph, child, expanded_ids);
        node.children.push(DiagramChild::Node(child_node));
    }
}

/// Check if an element has a Redefinition child (used to identify redefinition attributes).
fn is_redefinition_child(element: &Element, graph: &ModelGraph) -> bool {
    graph
        .children_of(&element.id)
        .any(|c| c.kind == ElementKind::Redefinition)
}

/// Render non-port children as `DiagramChild::Text` compartment labels (collapsed mode).
///
/// Each child is classified into a compartment kind based on the parent's
/// visual kind, then formatted as text via `compartment_text_for_element`.
/// Ports are skipped — callers handle port rendering separately.
pub(crate) fn render_collapsed_children(
    graph: &ModelGraph,
    parent_kind: &ElementKind,
    owned: &[&Element],
    node: &mut DiagramNode,
) {
    let parent_gk = VisualKind::from_element_kind(parent_kind);
    for child in owned {
        if classify::is_port_kind(&child.kind) {
            continue;
        }
        render_child_text_row(graph, &parent_gk, child, node);
    }
}

/// Render one child element as text row(s) in its routed compartment.
///
/// Shared by the collapsed path (every non-port child) and the expanded path
/// (value features + doc/comment children, C11/C12a).
pub(crate) fn render_child_text_row(
    graph: &ModelGraph,
    parent_gk: &VisualKind,
    child: &Element,
    node: &mut DiagramNode,
) {
    use crate::ir::types::DiagramChild;
    let comp_kind = parent_gk.compartment_for_element_with_graph(child, graph);

    // Documentation/Comment: emit multiple wrapped lines for readability
    if matches!(child.kind, ElementKind::Comment | ElementKind::Documentation) {
        let raw = child
            .get_prop("body")
            .or_else(|| child.get_prop("documentation"))
            .map(|v| v.to_string().trim_matches('"').to_owned())
            .unwrap_or_else(|| child.name.as_deref().unwrap_or("").to_owned());
        let lines = wrap_doc_text(&raw);
        for (li, line) in lines.into_iter().enumerate() {
            node.children.push(DiagramChild::Text {
                compartment: comp_kind,
                text: line,
                element_id: format!("{}/doc/{}", child.id, li),
                source: crate::ir::types::CompartmentItemSource::Owned,
            });
        }
        return;
    }

    // Specialized formatters for new compartment types
    let text = match comp_kind {
        classify::CompartmentKind::Redefinitions => match redefinition_text(child, graph) {
            Some(t) => t,
            None => return,
        },
        classify::CompartmentKind::Metadata => metadata_text(child, graph),
        _ => compartment_text_for_element(child, graph, comp_kind),
    };
    node.children.push(DiagramChild::Text {
        compartment: comp_kind,
        text,
        element_id: child.id.to_string(),
        source: crate::ir::types::CompartmentItemSource::Owned,
    });
}

/// Format a redefinition element as `name = value` (public for general.rs).
pub(crate) fn redefinition_text_pub(element: &Element, graph: &ModelGraph) -> Option<String> {
    redefinition_text(element, graph)
}

/// Format a redefinition element as `name = value`.
///
/// Returns `None` if the value is empty (caller should skip the entry).
fn redefinition_text(element: &Element, graph: &ModelGraph) -> Option<String> {
    // Get the redefined feature name from the Redefinition child
    let redef_name = graph
        .children_of(&element.id)
        .find(|c| c.kind == ElementKind::Redefinition)
        .and_then(|c| {
            c.get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str().map(|s| s.to_owned()))
        })?;

    // Get value from the element's props. Prefer the typed `value` prop,
    // fall back to the pretty-printed AST subtree (AST-first), then legacy
    // string-only graphs.
    let value = element
        .get_prop("value")
        .map(|v| {
            let s = v.to_string();
            s.trim_matches('"').to_owned()
        })
        .or_else(|| {
            sysml_core::expression_pretty::pretty_print_owner(element, graph).map(|s| {
                if let Some((_, variant)) = s.split_once("::") {
                    variant.to_owned()
                } else {
                    s
                }
            })
        });
        // Note: `unresolved_value` is no longer written by either parser
        // (removed in Phase 6D). The value/AST paths above are sufficient.

    match value {
        Some(v) if !v.is_empty() => Some(format!("{} = {}", redef_name, v)),
        _ => None, // Skip empty redefinitions
    }
}

/// Format a MetadataUsage element as `@TypeName [key=val, ...]` (public for builders.rs).
pub(crate) fn metadata_text_pub(element: &Element, graph: &ModelGraph) -> String {
    metadata_text(element, graph)
}

/// Return the metadata type name and individual key=value lines for expanded rendering.
///
/// Returns `(type_name, vec_of_key_value_strings)`.
/// Used by general.rs and requirements.rs to render metadata as a section header
/// with individual lines below instead of a single compact line.
pub(crate) fn metadata_lines(element: &Element, graph: &ModelGraph) -> (String, Vec<String>) {
    let type_name = element
        .get_prop("unresolvedTypeName")
        .and_then(|v| v.as_str())
        .or(element.name.as_deref())
        .unwrap_or("metadata")
        .to_owned();

    let mut pairs: Vec<String> = Vec::new();
    for child in ordered_children(graph, &element.id) {
        match child.kind {
            ElementKind::AttributeUsage => {
                if let Some(text) = redefinition_text(child, graph) {
                    pairs.push(text);
                }
            }
            ElementKind::ReferenceUsage => {
                if let Some(name) = &child.name {
                    if let Some(val) = child.get_prop("value").and_then(|v| v.as_str()) {
                        pairs.push(format!("{} = {}", name, val));
                    }
                }
            }
            _ => {}
        }
    }

    (type_name, pairs)
}

/// Format a MetadataUsage element as `@TypeName [key=val, ...]`.
fn metadata_text(element: &Element, graph: &ModelGraph) -> String {
    let type_name = element
        .name
        .as_deref()
        .or_else(|| {
            element
                .get_prop("unresolvedTypeName")
                .and_then(|v| v.as_str())
        })
        .unwrap_or("metadata");

    // Collect key=value pairs from metadata body children.
    // Children can be:
    // - AttributeUsage with Redefinition children (standard pattern)
    // - ReferenceUsage with name + value prop (MetadataBodyUsage pattern)
    let mut pairs: Vec<String> = Vec::new();
    for child in ordered_children(graph, &element.id) {
        match child.kind {
            ElementKind::AttributeUsage => {
                if let Some(text) = redefinition_text(child, graph) {
                    pairs.push(text);
                }
            }
            ElementKind::ReferenceUsage => {
                // MetadataBodyUsage children: named refs with direct value props
                if let Some(name) = &child.name {
                    if let Some(val) = child.get_prop("value").and_then(|v| v.as_str()) {
                        pairs.push(format!("{} = {}", name, val));
                    }
                }
            }
            _ => {}
        }
    }

    if pairs.is_empty() {
        format!("@{}", type_name)
    } else if pairs.len() <= 3 {
        format!("@{} [{}]", type_name, pairs.join(", "))
    } else {
        // Truncate to first 3 pairs
        format!(
            "@{} [{}, ...]",
            type_name,
            pairs[..3].join(", ")
        )
    }
}

/// Apply expand/collapse controls to a container node.
///
/// Handles the common pattern:
/// 1. If expandable: add expand button + set expanded state
/// 2. If has children but not expandable: set expanded state (for layout)
/// 3. Set layout mode: Free (ELK) for expanded, VBox for collapsed
///
/// Generator-specific overrides (e.g., sub-diagram embedding forcing
/// `expanded=true`) should be applied AFTER calling this function.
pub(crate) fn apply_expand_controls(
    node: &mut DiagramNode,
    is_expandable: bool,
    has_children: bool,
    is_expanded: bool,
) {
    use crate::ir::types::{DiagramButton, NodeLayout};

    if is_expandable {
        node.buttons.push(DiagramButton::expand());
        node.expanded = Some(is_expanded);
    } else if has_children {
        // Containers with children need expanded state for proper layout
        // even when not "expandable" (no nested sub-diagrams).
        node.expanded = Some(is_expanded);
    }

    // Always use Free layout — the view handles collapsed content
    // rendering directly as SVG (no VBox needed). ELK handles
    // inter-node positioning; the Rust `size` field tells ELK
    // how big the node is.
    node.layout = NodeLayout::Free;
}

/// Walk up the ownership chain to find the nearest ancestor matching a predicate.
///
/// Unifies `find_expanded_ancestor` and `find_rendered_ancestor` — callers
/// provide their own predicate (e.g., checking expanded_ids or rendered_node_ids).
/// Stops after 50 levels to prevent infinite loops from ownership cycles.
pub(crate) fn find_ancestor_by(
    graph: &ModelGraph,
    element_id: &ElementId,
    predicate: impl Fn(&ElementId) -> bool,
) -> Option<ElementId> {
    let mut current = graph.get_element(element_id)?.owner.clone();
    for _ in 0..50 {
        let owner_id = current?;
        if predicate(&owner_id) {
            return Some(owner_id);
        }
        current = graph.get_element(&owner_id)?.owner.clone();
    }
    None
}

// ── Edge labels ──────────────────────────────────────────────────────────

/// Spec-conformant display label for a connector-family edge.
///
/// SysML v2 §8.2.3.13/§8.2.3.16 give these edges (and only these) a *text*
/// label, via the graphical BNF:
///
/// ```text
/// connection-label = UsageDeclaration
/// flow-label       = UsageDeclaration? ('of' FlowPayloadFeatureMember)? | FlowPayloadFeatureMember
/// ```
///
/// So the label is the connector *usage's declared name* — never the metaclass
/// name. Grepping the whole spec for the token `typing` returns zero hits, and
/// the `connection`/`flow` keywords appear only inside the «…» name compartment
/// of an elaborating NODE, never on a plain edge. The specialization family
/// (FeatureTyping / Subclassification / Subsetting / Redefinition) has no label
/// production at all — its BNF productions are bare images, i.e. line style +
/// arrowhead carry the whole meaning. Those kinds must NOT come through here;
/// they get an empty label and let `EdgeStyle::from_relationship_kind` supply
/// the «redefines»/«subsets» keyword where the spec shows one.
///
/// Priority: declared name > `of <payload>` > `source → target` endpoint paths
/// from the origin element > resolved endpoint element names > empty.
pub(crate) fn edge_label_text(graph: &ModelGraph, rel: &sysml_core::Relationship) -> String {
    use sysml_core::RelationshipKind;
    let origin_key = match rel.kind {
        RelationshipKind::Flow | RelationshipKind::SuccessionFlow => "origin_flow",
        _ => "origin_connector",
    };
    if let Some(origin_id) = rel.props.get(origin_key).and_then(|v| v.as_ref()) {
        if let Some(origin) = graph.get_element(origin_id) {
            // Named connector (e.g. `connection waterLine : WaterPipe connect ...`)
            if let Some(name) = &origin.name {
                return name.clone();
            }
            // Flow with payload type
            if let Some(payload) = origin.get_prop("payloadType").and_then(|v| v.as_str()) {
                return format!("of {}", payload);
            }
            // Anonymous connector — use source→target path names from the element props
            // (e.g. InterfaceUsage with source="phaseIn", target="isolationSwitch.powerIn")
            if let (Some(src), Some(tgt)) = (
                origin.get_prop("source").and_then(|v| v.as_str()),
                origin.get_prop("target").and_then(|v| v.as_str()),
            ) {
                return format!("{} → {}", src, tgt);
            }
        }
    }
    // Fallback: derive label from resolved endpoint element names
    // (works even if origin_connector is missing)
    let src_name = graph.get_element(&rel.source).and_then(|e| e.name.clone());
    let tgt_name = graph.get_element(&rel.target).and_then(|e| e.name.clone());
    match (src_name, tgt_name) {
        (Some(s), Some(t)) => format!("{} → {}", s, t),
        (Some(s), None) => s,
        (None, Some(t)) => t,
        (None, None) => String::new(),
    }
}

/// Whether this relationship kind is a connector — the family the spec gives a
/// text label (see [`edge_label_text`]). Every other kind's notation is line
/// style + arrowhead, optionally plus a «keyword» from the design tokens.
pub(crate) fn is_connector_kind(kind: &sysml_core::RelationshipKind) -> bool {
    use sysml_core::RelationshipKind;
    matches!(
        kind,
        RelationshipKind::Flow
            | RelationshipKind::SuccessionFlow
            | RelationshipKind::Connection
            | RelationshipKind::Binding
            | RelationshipKind::Allocate
            | RelationshipKind::InterfaceConnection
    )
}
