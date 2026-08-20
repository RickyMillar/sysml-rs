//! Shared builder helpers for SModel generators.
//!
//! Each pattern that was duplicated across 3+ view generators lives here once.
//! View-specific logic stays in the respective generator module.

use std::collections::HashMap;

use sysml_core::{Element, ElementKind, ModelGraph, Relationship};

use super::types::{SCompartment, SModelElement, SLabel, Point, Dimension, SPort, SEdge, EdgePlacement};
use crate::visual_kind::{self as classify, CompartmentKind, VisualKind};

/// Resolve a display name for an element.
///
/// For named elements, returns the name. Unnamed elements synthesize a
/// meaningful label instead of leaking "unnamed" (C12):
/// - TransitionUsage: `source → target` from the parser's source/target props
/// - Comment/Documentation: the first wrapped line of the body text
/// - Redefinition attribute (e.g., `attribute :>> hasEV = true`): the
///   redefined feature name prefixed with `:>>`
pub(crate) fn element_display_name(element: &Element, graph: &ModelGraph) -> String {
    if let Some(name) = &element.name {
        return name.clone();
    }
    match element.kind {
        // C12b: unnamed transitions read `idle → driving`, not "unnamed".
        ElementKind::TransitionUsage => {
            let source = element
                .get_prop("source")
                .map(|v| v.to_string().trim_matches('"').to_owned());
            let target = element
                .get_prop("target")
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let (Some(s), Some(t)) = (source, target) {
                if !s.is_empty() && !t.is_empty() {
                    return format!("{} \u{2192} {}", s, t);
                }
            }
        }
        // C12a: a doc/comment note surfaces its body text, not "unnamed".
        ElementKind::Comment | ElementKind::Documentation => {
            let body = element
                .get_prop("body")
                .or_else(|| element.get_prop("documentation"))
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let Some(body) = body {
                let lines = crate::ir::generators::container::wrap_doc_text(&body);
                if let Some(first) = lines.into_iter().next() {
                    return first;
                }
            }
        }
        _ => {}
    }
    // Check Redefinition children for the redefined feature name
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::Redefinition {
            if let Some(name) = child
                .get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str())
            {
                return format!(":>> {}", name);
            }
        }
    }
    "unnamed".to_owned()
}

/// Convert `ElementKind` to `«spaced lowercase»` stereotype text.
///
/// Example: `ElementKind::PartDefinition` → `"«part def»"`
///
/// Uses the DECLARATION-TEXT keyword (contract A.3/A.4, D-N1): the guillemet
/// stereotype reads as the element's textual declaration keyword — `«part
/// def»`, `«requirement def»`, `«state»` — never the expanded metaclass name
/// (`«part definition»`). `element_keyword` derives it from the spec-generated
/// `syntax_keyword()` (+" def" for definitions), falling back to
/// `display_name()` for kinds with no syntax keyword.
pub(crate) fn stereotype_text(kind: &ElementKind) -> String {
    format!("\u{00ab}{}\u{00bb}", crate::visual_kind::element_keyword(kind))
}

/// Build a standard header compartment with stereotype + name labels.
pub(crate) fn make_header_compartment(
    parent_id: &str,
    stereotype: &str,
    name: &str,
) -> SCompartment {
    SCompartment {
        id: format!("{}/header", parent_id),
        type_: "comp:header".to_owned(),
        children: vec![
            SModelElement::Label(SLabel {
                id: format!("{}/header/stereotype", parent_id),
                type_: "label:stereotype".to_owned(),
                text: stereotype.to_owned(),
                position: None,
                css_classes: vec!["stereotype".to_owned()],
                edge_placement: None,
                semantic_element_id: None,
            diagnostic: None,
            }),
            SModelElement::Label(SLabel {
                id: format!("{}/header/name", parent_id),
                type_: "label:name".to_owned(),
                text: name.to_owned(),
                position: None,
                css_classes: vec!["name".to_owned()],
                edge_placement: None,
                semantic_element_id: None,
            diagnostic: None,
            }),
        ],
        css_classes: vec![],
        layout: Some("vbox".to_owned()),
        layout_options: None,
    }
}

/// Build an SPort with optional position/size overrides.
///
/// When `include_direction_css` is true, appends the port direction CSS class
/// (used by general and interconnection views). Geometry view passes false and
/// supplies explicit position/size from element properties.
pub(crate) fn make_port(
    element: &Element,
    counter: &mut u32,
    position: Option<Point>,
    size: Option<Dimension>,
    include_direction_css: bool,
) -> SPort {
    make_port_inner(element, counter, position, size, include_direction_css, false)
}

/// Build a port with recursive child port discovery for nested port hierarchies.
///
/// Discovers nested sub-ports in two ways:
/// 1. Direct children of this element (e.g., inline port declarations)
/// 2. Children inherited from the type definition (e.g., `port composite : CompositePort;`
///    inherits sub-ports from the `CompositePort` definition)
#[allow(clippy::indexing_slicing)] // child_sizes indexed in lockstep with nested_ports
pub(crate) fn make_port_recursive(
    element: &Element,
    graph: &sysml_core::ModelGraph,
    counter: &mut u32,
    position: Option<Point>,
    size: Option<Dimension>,
    include_direction_css: bool,
    depth: u32,
) -> SPort {
    let is_conjugated = classify::is_conjugated_port(element, graph);
    // Scale leaf port size by depth: 12 → 8.4 → 5.9 → 4.1 (×0.7 per level)
    let scale = 0.7_f64.powi(depth as i32);
    let base_size = 12.0 * scale;
    let depth_size = size.or(Some(Dimension { width: base_size, height: base_size }));
    let mut port = make_port_inner(element, counter, position, depth_size, include_direction_css, is_conjugated);

    // Find nested ports: first check direct children, then inherit from type definition.
    // Skip nameless ports — these are bare direction keywords (e.g., `out` in `out item data;`
    // gets parsed as a nameless port_usage, not a real sub-port).
    let mut nested_ports = Vec::new();
    for child in graph.children_of(&element.id) {
        if classify::is_port_kind(&child.kind) && child.name.is_some() {
            *counter += 1;
            let nested = make_port_recursive(child, graph, counter, None, None, include_direction_css, depth + 1);
            nested_ports.push(SModelElement::Port(nested));
        }
    }

    // If no direct port children, inherit from the type definition.
    // e.g., `port composite : CompositePort;` has no direct children, but
    // CompositePort defines `port data : DataPort; port power : PowerPort;`
    if nested_ports.is_empty() {
        if let Some(type_def) = classify::find_type_definition(graph, element) {
            let usage_prefix = element.id.to_string();
            for child in graph.children_of(&type_def.id) {
                if classify::is_port_kind(&child.kind) && child.name.is_some() {
                    *counter += 1;
                    let mut nested = make_port_recursive(child, graph, counter, None, None, include_direction_css, depth + 1);
                    nested.id = format!("{}:{}", usage_prefix, nested.id);
                    nested_ports.push(SModelElement::Port(nested));
                }
            }
        }
    }

    if !nested_ports.is_empty() {
        // --- Step 1: Determine composite direction ---
        // Prefer the element's own declared direction (e.g., `out port power`).
        // Only infer from children when the element has no declared direction.
        let own_direction = classify::port_direction_css_class(element);
        let (composite_dir, composite_side) = if let Some(ref dir) = own_direction {
            let side = match dir.as_str() {
                "port-in" => "WEST",
                "port-out" | "port-inout" => "EAST",
                _ => "EAST",
            };
            (dir.as_str(), side)
        } else {
            // No declared direction — infer from sub-port directions.
            let mut has_in = false;
            let mut has_out = false;
            for child in &nested_ports {
                if let SModelElement::Port(ref sp) = child {
                    for cls in &sp.css_classes {
                        match cls.as_str() {
                            "port-in" => has_in = true,
                            "port-out" => has_out = true,
                            "port-inout" => { has_in = true; has_out = true; }
                            _ => {}
                        }
                    }
                }
            }
            match (has_in, has_out) {
                (true, false) => ("port-in", "WEST"),
                (false, true) => ("port-out", "EAST"),
                _ => ("port-inout", "EAST"),
            }
        };

        // --- Step 2: Layout sub-ports based on composite side ---
        // Sub-ports are stacked along the layout axis with minimal padding.
        // The node-boundary side has zero padding so the composite port
        // sits flush on the boundary. Spacing scales with depth.
        let spacing = (3.0 * scale).max(1.5);
        let pad = (2.0 * scale).max(1.0);
        let is_horizontal = composite_side == "NORTH" || composite_side == "SOUTH";

        // Collect actual sizes of each sub-port (respecting recursive composites)
        let child_sizes: Vec<(f64, f64)> = nested_ports.iter().map(|child| {
            if let SModelElement::Port(ref sp) = child {
                let w = sp.size.as_ref().map_or(base_size, |s| s.width);
                let h = sp.size.as_ref().map_or(base_size, |s| s.height);
                (w, h)
            } else {
                (base_size, base_size)
            }
        }).collect();

        // Compute composite port size from children — tight packing
        let (composite_w, composite_h) = if is_horizontal {
            let total_w: f64 = child_sizes.iter().map(|(w, _)| w).sum::<f64>()
                + (child_sizes.len().saturating_sub(1)) as f64 * spacing;
            let max_h: f64 = child_sizes.iter().map(|(_, h)| *h).fold(0.0, f64::max);
            (total_w + pad * 2.0, max_h + pad * 2.0)
        } else {
            let max_w: f64 = child_sizes.iter().map(|(w, _)| *w).fold(0.0, f64::max);
            let total_h: f64 = child_sizes.iter().map(|(_, h)| h).sum::<f64>()
                + (child_sizes.len().saturating_sub(1)) as f64 * spacing;
            (max_w + pad * 2.0, total_h + pad * 2.0)
        };

        // Position sub-ports within the container, centered on the layout axis
        let mut offset = pad;
        for (i, child) in nested_ports.iter_mut().enumerate() {
            if let SModelElement::Port(ref mut sp) = child {
                let (cw, ch) = child_sizes[i];
                if is_horizontal {
                    sp.position = Some(Point { x: offset, y: pad });
                    offset += cw + spacing;
                } else {
                    // Center sub-port horizontally within the container
                    let x_center = (composite_w - cw) / 2.0;
                    sp.position = Some(Point { x: x_center, y: offset });
                    offset += ch + spacing;
                }
                sp.layout_options = None;
            }
        }

        port.children = nested_ports;
        port.size = Some(Dimension {
            width: composite_w,
            height: composite_h,
        });

        // --- Step 3: Apply inferred direction to composite port ---
        // CRITICAL: Keep port-* CSS class — the WASM router needs it for
        // directional edge anchoring. Stripping it breaks edge routing.
        port.css_classes.retain(|c| !c.starts_with("port-"));
        port.css_classes.push(composite_dir.to_owned());
        let mut opts = HashMap::new();
        opts.insert("elk.port.side".to_owned(), composite_side.to_owned());
        port.layout_options = Some(opts);
    }

    port
}

fn make_port_inner(
    element: &Element,
    counter: &mut u32,
    position: Option<Point>,
    size: Option<Dimension>,
    include_direction_css: bool,
    is_conjugated: bool,
) -> SPort {
    let id = element.id.to_string();
    let name = element.name.as_deref().unwrap_or("port").to_owned();
    *counter += 1;

    let mut css_classes = classify::element_css_classes(&element.kind);
    let mut port_side: Option<&str> = None;
    if include_direction_css {
        let mut dir_class = classify::port_direction_css_class(element);

        // Conjugated ports reverse direction: in↔out
        if is_conjugated {
            dir_class = dir_class.map(|d| match d.as_str() {
                "port-in" => "port-out".to_owned(),
                "port-out" => "port-in".to_owned(),
                other => other.to_owned(),
            });
            css_classes.push("conjugated".to_owned());
        }

        if let Some(ref dir) = dir_class {
            // Derive ELK port side from direction: in→WEST, out→EAST, inout→EAST
            port_side = Some(match dir.as_str() {
                "port-in" => "WEST",
                "port-out" | "port-inout" => "EAST",
                _ => "EAST",
            });
            css_classes.push(dir.clone());
        } else {
            // Fallback: infer side + direction CSS from port name (common convention)
            let name_lower = name.to_lowercase();
            if name_lower.ends_with("in") || name_lower.starts_with("in") {
                port_side = Some("WEST");
                css_classes.push("port-in".to_owned());
            } else {
                port_side = Some("EAST");
                css_classes.push("port-out".to_owned());
            }
        }
    }

    let layout_options = port_side.map(|side| {
        let mut opts = HashMap::new();
        opts.insert("elk.port.side".to_owned(), side.to_owned());
        opts
    });

    SPort {
        id,
        type_: "port".to_owned(),
        position,
        size: size.or(Some(Dimension {
            width: 12.0,
            height: 12.0,
        })),
        children: vec![],
        css_classes,
        layout_options,
        name: Some(name),
    }
}

/// Build a standard edge with a text label.
///
/// Callers pass the label text directly — `format!("{:?}", kind)` for general/geometry,
/// `format!("«{}»", kind.as_str().to_lowercase())` for requirements.
///
/// Default endpoint mode is `auto-side` — the WASM router picks the optimal
/// connection side based on actual node positions. This is correct for all views
/// except InterconnectionView, where edges connect to specific ports.
pub(crate) fn make_edge(rel: &Relationship, counter: &mut u32, label_text: String) -> SEdge {
    let id = rel.id.to_string();
    let edge_type = classify::smodel_edge_type(&rel.kind).to_owned();
    let css_classes = classify::relationship_css_classes(&rel.kind);
    *counter += 1;

    SEdge {
        id: id.clone(),
        type_: edge_type,
        source_id: rel.source.to_string(),
        target_id: rel.target.to_string(),
        children: vec![SModelElement::Label(SLabel {
            id: format!("{}/label", id),
            type_: "label:edge".to_owned(),
            text: label_text,
            position: None,
            css_classes: vec!["edge-label".to_owned()],
            edge_placement: Some(EdgePlacement {
                position: 0.5,
                side: "on".to_owned(),
                rotate: false,
                offset: None,
            }),
            semantic_element_id: None,
        diagnostic: None,
        })],
        css_classes,
        endpoint_mode: Some("auto-side".to_owned()),
        ..Default::default()
    }
}

/// Render a child element as a textual compartment line.
///
/// Per the SysML v2 graphical BNF, compartment elements are rendered as text:
///   `«keyword» name : Type`
/// Handles special cases: TransitionUsage, EnumerationUsage, Comment/Documentation.
pub(crate) fn render_as_text(element: &Element, graph: &ModelGraph) -> SLabel {
    let name = element_display_name(element, graph);
    let keyword = classify::element_keyword(&element.kind);

    let text = match element.kind {
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

        // Comments/Documentation: wrap into short lines, use first line for single-text rendering
        ElementKind::Comment | ElementKind::Documentation => {
            let raw = element
                .get_prop("body")
                .or_else(|| element.get_prop("documentation"))
                .map(|v| v.to_string().trim_matches('"').to_owned())
                .unwrap_or_else(|| name.to_owned());
            let lines = crate::ir::generators::container::wrap_doc_text(&raw);
            if lines.is_empty() {
                format!("/* {} */", name)
            } else {
                lines[0].clone()
            }
        }

        // MetadataUsage: compact annotation format
        ElementKind::MetadataUsage => {
            crate::ir::generators::container::metadata_text_pub(element, graph)
        }

        // Default: "keyword name : Type"
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

            match type_name.as_deref() {
                Some(t) if !t.is_empty() => format!("{} {} : {}", keyword, name, t),
                _ => format!("{} {}", keyword, name),
            }
        }
    };

    SLabel {
        id: format!("{}/text", element.id),
        type_: "label:name".to_owned(),
        text,
        position: None,
        css_classes: vec!["compartment-text".to_owned()],
        edge_placement: None,
        semantic_element_id: None,
    diagnostic: None,
    }
}

/// Emit compartments in spec-defined `allowed_compartments()` order, then remaining.
pub(crate) fn emit_compartments_in_order(
    parent_id: &str,
    parent_gk: &VisualKind,
    mut compartment_map: HashMap<CompartmentKind, Vec<SModelElement>>,
    out: &mut Vec<SModelElement>,
) {
    // Left-align ALL compartments (header + members).
    // Header: left-aligns «stereotype» + name near the package tab
    // Members: left-aligns text labels flush to node edge
    let left_align_opts: Option<HashMap<String, String>> = {
        let mut m = HashMap::new();
        m.insert("hAlign".to_owned(), "left".to_owned());
        Some(m)
    };

    let allowed = parent_gk.allowed_compartments();
    for comp_kind in &allowed {
        if let Some(children) = compartment_map.remove(comp_kind) {
            let _is_header = *comp_kind == CompartmentKind::Header;
            out.push(SModelElement::Compartment(SCompartment {
                id: format!(
                    "{}/{}",
                    parent_id,
                    comp_kind.type_string().replace(':', "_")
                ),
                type_: comp_kind.type_string().to_owned(),
                children,
                css_classes: vec![],
                layout: Some("vbox".to_owned()),
                layout_options: left_align_opts.clone(),
            }));
        }
    }
    for (comp_kind, children) in compartment_map {
        out.push(SModelElement::Compartment(SCompartment {
            id: format!(
                "{}/{}",
                parent_id,
                comp_kind.type_string().replace(':', "_")
            ),
            type_: comp_kind.type_string().to_owned(),
            children,
            css_classes: vec![],
            layout: Some("vbox".to_owned()),
            layout_options: left_align_opts.clone(),
        }));
    }
}

/// Generate structured tooltip text for an element.
///
/// Produces a multi-line tooltip with a line protocol:
/// ```text
/// «part definition» Vehicle
/// type: Engine
/// hierarchy: Vehicle > PhysicalObject > Anything
/// ```
///
/// - Line 1: `«keyword» name`
/// - `type:` line: walks FeatureTyping children to find the typed element's name
/// - `hierarchy:` line: walks Specialization/Subclassification up to 3 levels
///
/// Uses the declaration-text keyword (contract A.3/A.4 — same rule as
/// `stereotype_text`, D-N1): `«part def»`, not `«part definition»`.
pub(crate) fn tooltip_text(element: &Element, graph: &ModelGraph) -> Option<String> {
    let keyword = crate::visual_kind::element_keyword(&element.kind);
    let name = element_display_name(element, graph);
    let mut lines = vec![format!("«{}» {}", keyword, name)];

    // type: line — walk FeatureTyping children
    if let Some(type_name) = find_element_type_for_tooltip(element, graph) {
        lines.push(format!("type: {}", type_name));
    }

    // hierarchy: line — walk Specialization/Subclassification up to 3 levels
    let hierarchy = collect_hierarchy(element, graph, 3);
    if !hierarchy.is_empty() {
        lines.push(format!("hierarchy: {}", hierarchy.join(" > ")));
    }

    Some(lines.join("\n"))
}

/// Find the type name for a usage element by following FeatureTyping children.
///
/// Mirrors the LSP's `find_element_type()` pattern.
fn find_element_type_for_tooltip(element: &Element, graph: &ModelGraph) -> Option<String> {
    // Check direct typing prop first
    if let Some(typing) = element.props.get("typing").and_then(|v| v.as_str()) {
        return Some(typing.to_owned());
    }

    // Look for FeatureTyping children
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            // Try resolved reference
            if let Some(target_id) = child.props.get("type").and_then(|v| v.as_ref()) {
                if let Some(target) = graph.get_element(target_id) {
                    if let Some(name) = &target.name {
                        return Some(name.clone());
                    }
                }
            }
            // Try unresolved name
            if let Some(name) = child.props.get("unresolved_type").and_then(|v| v.as_str()) {
                return Some(name.to_owned());
            }
        }
    }

    None
}

/// Walk Specialization/Subclassification relationships up to `max_depth` levels,
/// collecting the chain of general type names.
fn collect_hierarchy(element: &Element, graph: &ModelGraph, max_depth: usize) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current_id = element.id.clone();

    for _ in 0..max_depth {
        let mut found_general = None;

        // Find Specialization/Subclassification children of the current element
        for child in graph.children_of(&current_id) {
            if child.kind == ElementKind::Specialization
                || child.kind.is_subtype_of(ElementKind::Specialization)
            {
                // Try resolved "general" property
                if let Some(general_id) = child.props.get("general").and_then(|v| v.as_ref()) {
                    if let Some(general_elem) = graph.get_element(general_id) {
                        if let Some(name) = &general_elem.name {
                            found_general = Some((name.clone(), general_id.clone()));
                            break;
                        }
                    }
                }
                // Try unresolved name
                if let Some(name) = child
                    .props
                    .get("unresolved_general")
                    .and_then(|v| v.as_str())
                {
                    chain.push(name.to_owned());
                    return chain; // Can't continue walking without a resolved ID
                }
            }
        }

        if let Some((name, general_id)) = found_general {
            chain.push(name);
            current_id = general_id;
        } else {
            break;
        }
    }

    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ModelGraph};

    #[test]
    fn tooltip_text_returns_keyword_name() {
        let graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let tt = tooltip_text(&elem, &graph);
        // Declaration-text keyword (contract A.3/A.4, D-N1).
        assert_eq!(tt, Some("\u{00ab}part def\u{00bb} Vehicle".to_string()));
    }
}
