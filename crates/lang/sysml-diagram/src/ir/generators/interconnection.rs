//! InterconnectionView (IBD) IR generator.
//!
//! Generates `DiagramIR` for Internal Block Diagrams — shows the internal
//! structure of a context block: its part usages with ports (inherited from
//! type definitions) connected by flows/connections.
//!
//! Key features:
//! - Context block discovery: top-level parts that own internal PartUsage children
//! - Port inheritance: ports discovered from type definitions via `find_type_definition`
//! - Recursive port hierarchy: composite ports with sub-ports
//! - Proxy ports on the context frame boundary
//! - Edge remapping: definition IDs → usage IDs
//! - Edge deduplication: highest priority wins (Flow > InterfaceConnection > Binding > Connection)
//! - Connection edges use `EndpointMode::StrictPort` with source_port_id/target_port_id
//! - ID scoping for embedded subtrees to avoid collisions

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementId, ModelGraph, RelationshipKind};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramNode, NodeLayout, DiagramChild, DiagramButton, DiagramPort, PortDirection, PortSide, PortTag, DiagramEdge, DiagramEdgeKind, EndpointMode, EdgeLabelPlacement, EdgeTag, NodeTag, CompartmentItemSource};
use crate::smodel::builders;
use crate::smodel::ViewType;
use crate::visual_kind::{self as classify, CompartmentKind, VisualKind};

/// Interconnection (IBD) view generator.
pub struct InterconnectionViewGenerator;

impl ViewGenerator for InterconnectionViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::Interconnection
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("RIGHT")
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!("InterconnectionView IR generate");
        let graph = ctx.graph;
        let expanded_ids = ctx.expanded_ids;
        let mut ir = DiagramIR::new(ViewType::Interconnection);

        // Expose-aware scope (steward ruling 2026-06-26; SysML v2 §8.2.3
        // InterconnectionView + Expose semantics). An IBD's subject is a
        // *container whose members* are the canvas — unlike General, whose subject
        // IS the exposed element — so `is_canvas_root` (true only for the exposed
        // element) is the wrong lens here. The declared-view path always sets
        // `expose`, so resolve scope from the exposed element's kind:
        //   • Package (NamespaceExpose) → its member part usages are peer nodes;
        //     the package is never a node; connectors it owns are top-level edges.
        //   • Part (MembershipExpose) → the exposed part IS the context block
        //     (no `has_internal_structure` gate — the spec makes the exposed
        //     feature the subject regardless of internal parts).
        //   • Constraint (parametric subject) → renders as a constraint block
        //     (`{expr}` header + parameter ports).
        // EVERY expose contributes — a view may declare several Expose clauses
        // (e.g. two constraint defs forming one parametric diagram); rendering
        // only the first silently dropped the rest.
        // Fail loud (empty + error log) if an id doesn't resolve — never silently
        // degrade. NOTE: ActionFlow/Sequence (same InterconnectionView supertype)
        // likely share this latent expose-scope bug — tracked, not fixed here.
        if !ctx.expose_ids.is_empty() {
            for expose_id in ctx.expose_ids {
                match graph.get_element(expose_id) {
                    Some(target) => generate_exposed_ibd_into(ctx, target, &mut ir),
                    None => {
                        tracing::error!(
                            "InterconnectionView: expose target {expose_id} not found in graph"
                        );
                    }
                }
            }
            return ir;
        }

        // Find context blocks: top-level parts that own internal PartUsage children
        // AND connection/flow relationships. Spec ViewFilter (4.5) is applied
        // here at the top-level collection.
        let mut context_blocks: Vec<_> = graph
            .elements
            .values()
            .filter(|e| {
                ctx.is_canvas_root(e)
                    && classify::is_part_kind(&e.kind)
                    && has_internal_structure(graph, e)
            })
            .filter(|e| ctx.passes_filter(e))
            .collect();

        // When multiple context blocks exist, filter out sub-component definitions
        // that are referenced as types by usages within other context blocks.
        // e.g. if CircuitModule contains `part paths : CircuitPath`, exclude CircuitPath.
        if context_blocks.len() > 1 {
            let context_names: HashSet<&str> = context_blocks
                .iter()
                .filter_map(|e| e.name.as_deref())
                .collect();
            let mut used_as_type: HashSet<String> = HashSet::new();
            for ctx in &context_blocks {
                // Check FeatureTyping relationships owned by children of this context
                for child in graph.children_of(&ctx.id) {
                    for grandchild in graph.children_of(&child.id) {
                        if grandchild.kind == sysml_core::ElementKind::FeatureTyping {
                            if let Some(type_name) =
                                grandchild.get_prop("unresolved_type").and_then(|v| v.as_str())
                            {
                                if context_names.contains(type_name) {
                                    used_as_type.insert(type_name.to_owned());
                                }
                            }
                        }
                    }
                }
            }
            if !used_as_type.is_empty() {
                context_blocks
                    .retain(|e| e.name.as_deref().is_none_or(|n| !used_as_type.contains(n)));
            }
        }

        if !context_blocks.is_empty() {
            // Spec-compliant mode: show internal structure of context blocks
            for context in &context_blocks {
                let mut node = generate_context_block_ir(graph, context, expanded_ids);

                // Nest connection edges inside the context node
                // (ELK requires edges at the LCA level of their source/target)
                let edges = generate_edges_for_context(graph, context);
                for edge in edges {
                    node.children.push(DiagramChild::Edge(edge));
                }

                ir.nodes.push(node);
            }
        } else {
            // Fallback: no context blocks found — show all top-level parts flat
            let top_level_parts: Vec<_> = graph
                .elements
                .values()
                .filter(|e| ctx.is_canvas_root(e) && classify::is_part_kind(&e.kind))
                .filter(|e| ctx.passes_filter(e))
                .collect();

            for part in &top_level_parts {
                let node = generate_usage_node_ir(graph, part, expanded_ids);
                ir.nodes.push(node);
            }

            // All connection-like edges (with deduplication)
            let flat_edges = generate_flat_edges(graph);
            ir.edges.extend(flat_edges);
        }

        // ── Constraint notation (parametric blocks) ──
        // Constraint elements render as constraint blocks with a `{expression}`
        // header and small square parameter ports (`PortTag::Parametric`),
        // mirroring the legacy parametric.rs peer generator MINUS solver /
        // satisfaction state (handled by a separate sidecar phase). Gated on
        // element kind so a constraint shown in any Interconnection view keeps
        // its notation. Only top-level (canvas-root) constraints are emitted;
        // constraints nested inside a part are handled within that part.
        for element in graph.elements.values() {
            if !is_constraint_kind(&element.kind) {
                continue;
            }
            if !ctx.is_canvas_root(element) || !ctx.passes_filter(element) {
                continue;
            }
            ir.nodes.push(generate_constraint_node_ir(graph, element));
        }

        ir
    }

    fn generate_for_owner(
        &self,
        ctx: &GeneratorContext,
        owner_id: &str,
    ) -> Option<DiagramIR> {
        let graph = ctx.graph;
        let expanded_ids = ctx.expanded_ids;
        let owner_eid = ElementId::from_string(owner_id);
        let owner = graph.get_element(&owner_eid)?;

        // Only generate IBD for elements with internal structure
        if !has_internal_structure(graph, owner) {
            return None;
        }

        let mut ir = DiagramIR::new(ViewType::Interconnection);
        let scope_prefix = format!("owner-{}", owner_id);

        // Build a map of original ID → scoped ID for nodes and ports.
        let mut id_remap: HashMap<String, String> = HashMap::new();

        // Generate usage nodes for internal PartUsage children
        let owned: Vec<_> = graph.children_of(&owner_eid).collect();
        for child in &owned {
            if child.kind.is_usage() && classify::is_part_kind(&child.kind) {
                let mut usage_node = generate_usage_node_ir(graph, child, expanded_ids);
                let original_id = usage_node.element_id.clone();
                let scoped_id = format!("{}/{}", scope_prefix, original_id);
                id_remap.insert(original_id, scoped_id.clone());
                usage_node.element_id = scoped_id;

                // Scope port IDs and build remap entries
                for port in &mut usage_node.ports {
                    scope_port_id_with_remap(port, &scope_prefix, &mut id_remap);
                }

                ir.nodes.push(usage_node);
            }
        }

        // Generate edges using context-edge logic, then remap IDs
        let raw_edges = generate_edges_for_context(graph, owner);
        for mut edge in raw_edges {
            let src_remapped = id_remap.get(&edge.source_id).cloned();
            let tgt_remapped = id_remap.get(&edge.target_id).cloned();

            // Skip edges where either endpoint doesn't map to a known node
            if src_remapped.is_none() && edge.source_port_id.is_none() {
                continue;
            }
            if tgt_remapped.is_none() && edge.target_port_id.is_none() {
                continue;
            }

            edge.id = format!("{}/{}", scope_prefix, edge.id);
            if let Some(remapped) = src_remapped {
                edge.source_id = remapped;
            } else {
                edge.source_id = format!("{}/{}", scope_prefix, edge.source_id);
            }
            if let Some(remapped) = tgt_remapped {
                edge.target_id = remapped;
            } else {
                edge.target_id = format!("{}/{}", scope_prefix, edge.target_id);
            }

            // Remap port IDs
            if let Some(ref port_id) = edge.source_port_id {
                edge.source_port_id = Some(
                    id_remap
                        .get(port_id)
                        .cloned()
                        .unwrap_or_else(|| format!("{}/{}", scope_prefix, port_id)),
                );
            }
            if let Some(ref port_id) = edge.target_port_id {
                edge.target_port_id = Some(
                    id_remap
                        .get(port_id)
                        .cloned()
                        .unwrap_or_else(|| format!("{}/{}", scope_prefix, port_id)),
                );
            }

            ir.edges.push(edge);
        }

        Some(ir)
    }
}

// ── Helper: context detection ────────────────────────────────────────────

/// Check if a part element has internal structure (owns PartUsage children).
fn has_internal_structure(graph: &ModelGraph, element: &sysml_core::Element) -> bool {
    graph
        .children_of(&element.id)
        .any(|c| c.kind.is_usage() && classify::is_part_kind(&c.kind))
}

/// Whether an element is a namespace container (Package / LibraryPackage) — the
/// `NamespaceExpose` case for an IBD subject.
fn is_package_kind(kind: &sysml_core::ElementKind) -> bool {
    matches!(
        kind,
        sysml_core::ElementKind::Package | sysml_core::ElementKind::LibraryPackage
    )
}

/// Whether an element kind is a constraint block (drawn with parametric
/// notation in any Interconnection view that contains it).
fn is_constraint_kind(kind: &sysml_core::ElementKind) -> bool {
    matches!(
        kind,
        sysml_core::ElementKind::ConstraintDefinition
            | sysml_core::ElementKind::ConstraintUsage
            | sysml_core::ElementKind::AssertConstraintUsage
    )
}

/// Build a constraint block node with a `{expression}` header and small square
/// parameter ports, mirroring the legacy `parametric.rs` `generate_constraint_node`
/// MINUS solver value badges / satisfaction icons (a separate sidecar phase).
fn generate_constraint_node_ir(
    graph: &ModelGraph,
    element: &sysml_core::Element,
) -> DiagramNode {
    let id = element.id.to_string();
    let name = element.name.as_deref().unwrap_or("constraint").to_owned();
    let visual_kind = VisualKind::from_element_kind(&element.kind);
    let stereotype = builders::stereotype_text(&element.kind);

    let mut node = DiagramNode::new(id.clone(), visual_kind, &name)
        .with_stereotype(stereotype)
        .with_tag(NodeTag::ParametricConstraint)
        .with_element_kind(element.kind.clone());
    node.tooltip = builders::tooltip_text(element, graph);
    node.tags.extend(classify::property_tags(element));

    // Expression header — prefer the structured AST, fall back to the legacy
    // string props parametric.rs reads. Skip entirely if no expression.
    let expression = crate::ir::expression_pretty::pretty_print_owner(element, graph).or_else(|| {
        element
            .get_prop("constraint")
            .or_else(|| element.get_prop("expression"))
            .or_else(|| element.get_prop("expr"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
    });
    if let Some(ref expr) = expression {
        node.children.push(DiagramChild::Text {
            compartment: CompartmentKind::Documentation,
            text: format!("{{{}}}", expr),
            element_id: format!("{}/expr", id),
            source: CompartmentItemSource::Owned,
        });
    } else if let Some(doc_text) = graph.children_of(&element.id).find_map(|c| {
        (c.kind == sysml_core::ElementKind::Documentation)
            .then(|| c.get_prop("body").and_then(|v| v.as_str()))
            .flatten()
            .map(|s| s.to_owned())
    }) {
        // Doc-only authoring shape (R2-7): the constraint carries no expression
        // AST or expression prop — only a `doc /* ... */` body. Render the doc
        // text so the block isn't name-only. NOT wrapped in `{…}`: braces are
        // the spec's constraint-expression notation, and a doc comment is an
        // annotation, not an expression — claiming otherwise would fabricate
        // semantics the model doesn't have.
        node.children.push(DiagramChild::Text {
            compartment: CompartmentKind::Documentation,
            text: doc_text,
            element_id: format!("{}/doc", id),
            source: CompartmentItemSource::Owned,
        });
    }

    // Parameter ports — small square Parametric ports (size 8x8),
    // distinguishing them from flow ports. Three child shapes qualify:
    //   • directed usages (`in x : Real;` — parses as ReferenceUsage with a
    //     `direction` prop; these ARE the constraint's parameters, KerML §7.4
    //     directed features of a behavior/function). This was the R2-7 gap:
    //     the old gate only matched AttributeUsage, so directed `in` params
    //     never emitted ports.
    //   • attribute children (legacy parametric.rs behavior, preserved)
    //   • port-kind children (preserved)
    for child in graph.children_of(&element.id) {
        let direction = classify::port_direction_css_class(child);
        let is_directed_param = child.kind.is_usage() && direction.is_some();
        if is_directed_param
            || child.kind == sysml_core::ElementKind::AttributeUsage
            || classify::is_port_kind(&child.kind)
        {
            let child_name = child.name.as_deref().unwrap_or("param");
            let mut port = DiagramPort::new(child.id.to_string(), child_name);
            port.tags.push(PortTag::Parametric);
            port.size = Some((8.0, 8.0));
            // Declared parameter direction drives placement: `in` params on the
            // west side, `out`/`inout` on the east (mirrors flow-port layout).
            // Attribute/port children without a direction keep the legacy
            // unplaced behavior.
            match direction.as_deref() {
                Some("port-in") => {
                    port.direction = Some(PortDirection::In);
                    port.side = Some(PortSide::West);
                }
                Some("port-out") => {
                    port.direction = Some(PortDirection::Out);
                    port.side = Some(PortSide::East);
                }
                Some("port-inout") => {
                    port.direction = Some(PortDirection::InOut);
                    port.side = Some(PortSide::East);
                }
                _ => {}
            }
            node.ports.push(port);
        }
    }

    node
}

/// Build an IBD scene scoped to an `expose` target. See the call site in
/// `InterconnectionViewGenerator::generate` for the spec-grounded semantics
/// (steward ruling 2026-06-26).
fn generate_exposed_ibd_into(
    ctx: &GeneratorContext,
    target: &sysml_core::Element,
    ir: &mut DiagramIR,
) {
    let graph = ctx.graph;
    let expanded_ids = ctx.expanded_ids;

    if is_package_kind(&target.kind) {
        // NamespaceExpose: the exposed namespace's member part *usages* are the
        // canvas peers; the package itself is never a node.
        for child in graph.children_of(&target.id) {
            if child.kind.is_usage()
                && classify::is_part_kind(&child.kind)
                && ctx.passes_filter(child)
            {
                ir.nodes
                    .push(generate_usage_node_ir(graph, child, expanded_ids));
            }
        }
        // Constraint members render as parametric constraint blocks (notation
        // gated on element kind, mirroring parametric.rs minus solver state).
        for child in graph.children_of(&target.id) {
            if is_constraint_kind(&child.kind) && ctx.passes_filter(child) {
                ir.nodes.push(generate_constraint_node_ir(graph, child));
            }
        }
        // Connectors owned by the exposed namespace are top-level edges (the
        // peers' LCA is the root, so the edges live at the root, not nested).
        ir.edges
            .extend(generate_edges_for_context(graph, target));
    } else if classify::is_part_kind(&target.kind) {
        // MembershipExpose of a part: the exposed part IS the IBD context block.
        let mut node = generate_context_block_ir(graph, target, expanded_ids);
        for edge in generate_edges_for_context(graph, target) {
            node.children.push(DiagramChild::Edge(edge));
        }
        ir.nodes.push(node);
    } else if is_constraint_kind(&target.kind) {
        // MembershipExpose of a constraint: parametric subject — renders as a
        // constraint block ({expr} header + parameter ports). Before this arm
        // a constraint-exposing Interconnection view rendered NOTHING (D-B4).
        ir.nodes.push(generate_constraint_node_ir(graph, target));
    } else {
        tracing::warn!(
            "InterconnectionView: expose target {} is neither a namespace, a \
             part-kind element, nor a constraint; nothing to render",
            target.id
        );
    }
}

/// Whether a relationship kind is relevant for IBD edges.
fn is_ibd_edge_kind(kind: &RelationshipKind) -> bool {
    matches!(
        kind,
        RelationshipKind::Flow
            | RelationshipKind::Reference
            | RelationshipKind::Connection
            | RelationshipKind::Binding
            | RelationshipKind::Allocate
            | RelationshipKind::InterfaceConnection
    )
}

/// Edge type priority for deduplication: higher = preferred.
/// Flow > InterfaceConnection > Binding > Connection > others.
fn edge_kind_priority(kind: &RelationshipKind) -> u8 {
    match kind {
        RelationshipKind::Flow => 4,
        RelationshipKind::InterfaceConnection => 3,
        RelationshipKind::Binding => 2,
        RelationshipKind::Connection => 1,
        _ => 0,
    }
}

// ── Context block node ───────────────────────────────────────────────────

/// Generate a context block node (the frame of the IBD).
/// Its PartUsage children become internal nodes with inherited ports.
fn generate_context_block_ir(
    graph: &ModelGraph,
    context: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
) -> DiagramNode {
    let id = context.id.to_string();
    let name = context.name.as_deref().unwrap_or("unnamed").to_owned();
    let visual_kind = VisualKind::from_element_kind(&context.kind);
    let stereotype = builders::stereotype_text(&context.kind);

    let mut node = DiagramNode::new(id.clone(), visual_kind, &name)
        .with_stereotype(stereotype)
        .with_layout(NodeLayout::Free); // Let ELK's INCLUDE_CHILDREN handle internal arrangement
    // The IBD context frame IS the diagram subject — it exists to show its
    // internal structure (spec §8.2.3 InterconnectionView), so it renders
    // expanded. Without this the renderer showed a collapsed bare box (D-B4).
    node.expanded = Some(true);

    node.tooltip = builders::tooltip_text(context, graph);

    // Element kind drives lowercased-kind / definition / usage / visual-kind classes
    // in the adapter.
    node = node.with_element_kind(context.kind.clone());
    node.tags.extend(classify::property_tags(context));

    // Proxy ports — context block's own ports shown on the frame boundary
    let owned: Vec<_> = graph.children_of(&context.id).collect();
    let mut proxy_port_count = 0usize;
    for child in &owned {
        if classify::is_port_kind(&child.kind) {
            let mut port = make_port_ir_recursive(child, graph, 0, &mut Vec::new());
            // Scope port IDs to context block and mark as proxy
            port.element_id = format!("ctx:{}:{}", id, port.element_id);
            port.is_proxy = true;
            scope_proxy_port_ids_ir(&mut port, &format!("ctx:{}", id));
            node.ports.push(port);
            proxy_port_count += 1;
        }
    }

    // Also inherit ports from type definition if context is a usage
    if proxy_port_count == 0 {
        if let Some(type_def) = classify::find_type_definition(graph, context) {
            for child in graph.children_of(&type_def.id) {
                if classify::is_port_kind(&child.kind) {
                    let mut port = make_port_ir_recursive(child, graph, 0, &mut Vec::new());
                    port.element_id = format!("ctx:{}:{}", id, port.element_id);
                    port.is_proxy = true;
                    scope_proxy_port_ids_ir(&mut port, &format!("ctx:{}", id));
                    node.ports.push(port);
                }
            }
        }
    }

    // Internal part usages — the IBD's main content
    for child in &owned {
        if child.kind.is_usage() && classify::is_part_kind(&child.kind) {
            let usage_node = generate_usage_node_ir(graph, child, expanded_ids);
            node.children.push(DiagramChild::Node(usage_node));
        }
    }

    node
}

// ── Usage node ───────────────────────────────────────────────────────────

/// Generate a PartUsage node with ports inherited from its type definition.
fn generate_usage_node_ir(
    graph: &ModelGraph,
    element: &sysml_core::Element,
    expanded_ids: &HashSet<String>,
) -> DiagramNode {
    let id = element.id.to_string();
    let name = element.name.as_deref().unwrap_or("unnamed").to_owned();
    let visual_kind = VisualKind::from_element_kind(&element.kind);

    // Show type name: "engine : Engine"
    let type_label = classify::find_type_definition(graph, element)
        .and_then(|def| def.name.as_deref())
        .map(|tn| format!("{} : {}", name, tn))
        .unwrap_or_else(|| name.clone());

    // Use type_label as the name (for header rendering)
    let mut node = DiagramNode::new(id, visual_kind, &type_label);

    node.tooltip = builders::tooltip_text(element, graph);

    // Element kind drives lowercased-kind / definition / usage / visual-kind classes
    // in the adapter.
    node = node.with_element_kind(element.kind.clone());
    node.tags.extend(classify::property_tags(element));

    // Ports: first check direct children, then inherit from type definition
    let mut port_count = 0usize;
    let owned: Vec<_> = graph.children_of(&element.id).collect();
    for child in &owned {
        if classify::is_port_kind(&child.kind) {
            let port = make_port_ir_recursive(child, graph, 0, &mut Vec::new());
            node.ports.push(port);
            port_count += 1;
        }
    }

    // If no direct ports, inherit from type definition.
    // Port IDs are prefixed with usage ID to avoid duplicates when multiple
    // usages share the same type definition.
    if port_count == 0 {
        if let Some(type_def) = classify::find_type_definition(graph, element) {
            let usage_prefix = element.id.to_string();
            for child in graph.children_of(&type_def.id) {
                if classify::is_port_kind(&child.kind) {
                    let mut port = make_port_ir_recursive(child, graph, 0, &mut Vec::new());
                    scope_port_ids_ir(&mut port, &usage_prefix);
                    node.ports.push(port);
                    port_count += 1;
                }
            }
        }
    }

    let _ = port_count;

    // Behavioral content embedding + collapsed content
    let id_str = element.id.to_string();
    let is_expanded = expanded_ids.contains(&id_str);

    let has_state_children = graph
        .children_of(&element.id)
        .any(|c| classify::is_state_kind(&c.kind));
    let has_action_children = graph
        .children_of(&element.id)
        .any(|c| classify::is_action_kind(&c.kind));
    let has_behavioral = has_state_children || has_action_children;

    // Non-port structural children that are meaningful for expansion.
    // In IBD context, connectors, typings, subsettings, etc. are not expandable
    // content — they are relationships, not displayable sub-parts.
    let structural_children: Vec<_> = owned
        .iter()
        .filter(|c| {
            !classify::is_port_kind(&c.kind)
                && !classify::is_membership_kind(&c.kind)
                && !classify::is_import_kind(&c.kind)
                && !matches!(
                    c.kind,
                    // Connectors are relationships (edges), not structural parts
                    sysml_core::ElementKind::InterfaceUsage
                        | sysml_core::ElementKind::ConnectionUsage
                        | sysml_core::ElementKind::BindingConnectorAsUsage
                        | sysml_core::ElementKind::ConnectorAsUsage
                    // Typing/specialization relationships
                        | sysml_core::ElementKind::FeatureTyping
                        | sysml_core::ElementKind::Subsetting
                        | sysml_core::ElementKind::Subclassification
                        | sysml_core::ElementKind::Redefinition
                        | sysml_core::ElementKind::ReferenceSubsetting
                    // Properties and metadata — not expandable sub-parts
                        | sysml_core::ElementKind::AttributeUsage
                        | sysml_core::ElementKind::Documentation
                        | sysml_core::ElementKind::Comment
                        | sysml_core::ElementKind::SatisfyRequirementUsage
                        | sysml_core::ElementKind::AssertConstraintUsage
                        | sysml_core::ElementKind::ExhibitStateUsage
                        | sysml_core::ElementKind::ReferenceUsage
                )
        })
        .copied()
        .collect();
    let has_structural = !structural_children.is_empty();

    if is_expanded {
        if has_state_children {
            node.children.push(DiagramChild::Island {
                view_type: ViewType::StateTransition,
                display_name: "State Machine".to_owned(),
                subtree: DiagramIR::new(ViewType::StateTransition),
                expanded: true,
            });
        }
        if has_action_children {
            node.children.push(DiagramChild::Island {
                view_type: ViewType::ActionFlow,
                display_name: "Actions".to_owned(),
                subtree: DiagramIR::new(ViewType::ActionFlow),
                expanded: true,
            });
        }
    } else if has_structural || has_behavioral {
        // Collapsed: show structural children as compartment text
        super::container::render_collapsed_children(
            graph, &element.kind, &structural_children, &mut node,
        );
    }

    if has_behavioral || has_structural {
        node.buttons.push(DiagramButton::expand());
        node.expanded = Some(is_expanded);
    }

    node
}

// ── Port IR construction ─────────────────────────────────────────────────

/// Build a `DiagramPort` with recursive sub-port discovery.
///
/// Discovers nested sub-ports in two ways:
/// 1. Direct children of this element
/// 2. Children inherited from the type definition
#[allow(clippy::indexing_slicing)] // child_sizes indexed in lockstep with nested_ports
fn make_port_ir_recursive(
    element: &sysml_core::Element,
    graph: &ModelGraph,
    depth: u32,
    // Type-definition ids currently being expanded on this recursion path.
    // Guards against self-referential port types (a port typed by a definition
    // that transitively contains a port of the same type), which would
    // otherwise re-expand the same type def at every level — exponential
    // fan-out / non-termination, not merely deep recursion.
    seen_type_ids: &mut Vec<ElementId>,
) -> DiagramPort {
    let name = element.name.as_deref().unwrap_or("port").to_owned();
    let is_conjugated = classify::is_conjugated_port(element, graph);

    // Scale port size by depth: 12 → 8.4 → 5.9 → 4.1 (x0.7 per level)
    let scale = 0.7_f64.powi(depth as i32);
    let base_size = 12.0 * scale;

    let mut port = DiagramPort::new(element.id.to_string(), name)
        .with_size(base_size, base_size);

    // Conjugation — covered by the typed `is_conjugated` flag.
    if is_conjugated {
        port.is_conjugated = true;
    }

    // Direction from element properties.
    // The declared direction (in/out from the `in port`/`out port` keyword) already
    // represents the correct visual placement for IBD rendering. Conjugation affects
    // type semantics (handled by effectiveDirection in elaboration) but should NOT
    // reverse port side placement — `in port steamIn : ~WaterPort` should still be
    // on the WEST (input) side.
    let dir_class = classify::port_direction_css_class(element);
    let effective_dir = dir_class;

    // R4 (§7.12.1, Table 10): direction notation belongs to DECLARED directed
    // features only. A `port` with an `in`/`out`/`inout` keyword drives both the
    // in/out coloring and the W/E side placement. A BARE `port fuelIn;` has no
    // declared direction — inferring in/out from a camelCase name is
    // non-normative (it colored/placed ports the spec treats as undirected), so
    // it stays direction-less: neutral coloring, elk FREE side placement.
    if let Some(ref dir) = effective_dir {
        port.direction = match dir.as_str() {
            "port-in" => Some(PortDirection::In),
            "port-out" => Some(PortDirection::Out),
            "port-inout" => Some(PortDirection::InOut),
            _ => None,
        };
        // Typed ELK port side; the adapter emits `elk.port.side` from this.
        let side = match dir.as_str() {
            "port-in" => PortSide::West,
            _ => PortSide::East,
        };
        port.side = Some(side);
    }

    // Discover nested sub-ports. Depth-capped to prevent unbounded recursion
    // (mirrors the state generator's `MAX_STATE_DEPTH`). A self-referential
    // port type — a port whose type definition contains a port of the same
    // type — would otherwise recurse forever on the same element and overflow
    // the stack. 20 levels is far beyond any real nested-port structure.
    const MAX_PORT_DEPTH: u32 = 20;
    let mut nested_ports: Vec<DiagramPort> = Vec::new();
    if depth < MAX_PORT_DEPTH {
        for child in graph.children_of(&element.id) {
            if classify::is_port_kind(&child.kind) && child.name.is_some() {
                nested_ports.push(make_port_ir_recursive(child, graph, depth + 1, seen_type_ids));
            }
        }

        // If no direct port children, inherit from type definition.
        if nested_ports.is_empty() {
            if let Some(type_def) = classify::find_type_definition(graph, element) {
                // Cycle guard: only descend into a given type definition ONCE
                // per recursion path. A port typed by a definition that
                // (transitively) contains a port of the same type would
                // otherwise re-expand `type_def` at every level — exponential
                // blow-up that presents as a hang (and a stack overflow once
                // deep enough). The depth cap above is a backstop; this is the
                // real terminator.
                if !seen_type_ids.contains(&type_def.id) {
                    seen_type_ids.push(type_def.id.clone());
                    let usage_prefix = element.id.to_string();
                    for child in graph.children_of(&type_def.id) {
                        if classify::is_port_kind(&child.kind) && child.name.is_some() {
                            let mut nested =
                                make_port_ir_recursive(child, graph, depth + 1, seen_type_ids);
                            nested.element_id = format!("{}:{}", usage_prefix, nested.element_id);
                            nested_ports.push(nested);
                        }
                    }
                    seen_type_ids.pop();
                }
            }
        }
    }

    if !nested_ports.is_empty() {
        // Determine composite direction (typed) and ELK side string for layout math.
        let own_direction = effective_dir;
        let (composite_dir, composite_side): (PortDirection, &str) =
            if let Some(ref dir) = own_direction {
                match dir.as_str() {
                    "port-in" => (PortDirection::In, "WEST"),
                    "port-inout" => (PortDirection::InOut, "EAST"),
                    _ => (PortDirection::Out, "EAST"),
                }
            } else {
                // Infer from sub-port directions
                let mut has_in = false;
                let mut has_out = false;
                for sp in &nested_ports {
                    match sp.direction {
                        Some(PortDirection::In) => has_in = true,
                        Some(PortDirection::Out) => has_out = true,
                        Some(PortDirection::InOut) => {
                            has_in = true;
                            has_out = true;
                        }
                        None => {}
                    }
                }
                match (has_in, has_out) {
                    (true, false) => (PortDirection::In, "WEST"),
                    (false, true) => (PortDirection::Out, "EAST"),
                    _ => (PortDirection::InOut, "EAST"),
                }
            };

        // Layout sub-ports based on composite side
        let spacing = (3.0 * scale).max(1.5);
        let pad = (2.0 * scale).max(1.0);
        let is_horizontal = composite_side == "NORTH" || composite_side == "SOUTH";

        // Collect actual sizes of each sub-port
        let child_sizes: Vec<(f64, f64)> = nested_ports
            .iter()
            .map(|sp| sp.size.unwrap_or((base_size, base_size)))
            .collect();

        // Compute composite port size from children
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

        // Position sub-ports within the container
        let mut offset = pad;
        for (i, sp) in nested_ports.iter_mut().enumerate() {
            let (cw, ch) = child_sizes[i];
            if is_horizontal {
                sp.position = Some((offset, pad));
                offset += cw + spacing;
            } else {
                let x_center = (composite_w - cw) / 2.0;
                sp.position = Some((x_center, offset));
                offset += ch + spacing;
            }
        }

        port.sub_ports = nested_ports;
        port.size = Some((composite_w, composite_h));

        // Apply inferred direction/side to composite port (typed; adapter emits ELK).
        port.direction = Some(composite_dir);
        port.side = Some(match composite_side {
            "WEST" => PortSide::West,
            "NORTH" => PortSide::North,
            "SOUTH" => PortSide::South,
            _ => PortSide::East,
        });
    }

    port
}

/// Recursively prefix a port's ID and all nested sub-port IDs with a usage scope.
fn scope_port_ids_ir(port: &mut DiagramPort, prefix: &str) {
    port.element_id = format!("{}:{}", prefix, port.element_id);
    for sub in &mut port.sub_ports {
        scope_port_ids_ir(sub, prefix);
    }
}

/// Recursively scope nested sub-port IDs under a proxy port prefix.
/// The top-level proxy port ID is already set by the caller.
fn scope_proxy_port_ids_ir(port: &mut DiagramPort, prefix: &str) {
    for sub in &mut port.sub_ports {
        sub.element_id = format!("{}:{}", prefix, sub.element_id);
        scope_proxy_port_ids_ir(sub, prefix);
    }
}

/// Recursively scope port IDs and build remap entries for embedded subtrees.
fn scope_port_id_with_remap(
    port: &mut DiagramPort,
    prefix: &str,
    remap: &mut HashMap<String, String>,
) {
    let orig = port.element_id.clone();
    let scoped = format!("{}/{}", prefix, orig);
    remap.insert(orig, scoped.clone());
    port.element_id = scoped;
    for sub in &mut port.sub_ports {
        scope_port_id_with_remap(sub, prefix, remap);
    }
}

// ── Edge generation ──────────────────────────────────────────────────────

/// Generate edges for connections/flows owned by a context block.
///
/// Relationships reference ports on type definitions. We remap:
/// definition ID → usage ID, so edges connect to usage nodes.
#[allow(clippy::indexing_slicing)] // candidates indexed by HashMap-stored indices
fn generate_edges_for_context(
    graph: &ModelGraph,
    context: &sysml_core::Element,
) -> Vec<DiagramEdge> {
    // Build mapping: definition ID → usage ID (for parts inside this context)
    // NOTE: This is 1:1 — when multiple usages share a type, only one survives.
    // Name-based resolution (below) handles the 1:N case correctly.
    let mut def_to_usage: HashMap<String, String> = HashMap::new();
    for child in graph.children_of(&context.id) {
        if child.kind.is_usage() && classify::is_part_kind(&child.kind) {
            if let Some(type_def) = classify::find_type_definition(graph, child) {
                def_to_usage.insert(type_def.id.to_string(), child.id.to_string());
            }
        }
    }

    // Build mapping: usage/port name → ElementId (for name-based endpoint resolution)
    // This handles the 1:N case where multiple usages share the same type definition.
    // Includes both part usages and port usages (context-level ports).
    let mut name_to_usage: HashMap<String, sysml_core::ElementId> = HashMap::new();
    for child in graph.children_of(&context.id) {
        if child.kind.is_usage()
            && (classify::is_part_kind(&child.kind) || classify::is_port_kind(&child.kind))
        {
            if let Some(name) = &child.name {
                name_to_usage.insert(name.clone(), child.id.clone());
            }
        }
    }
    // Also include ports from the context's type definition (inherited ports)
    if let Some(type_def) = classify::find_type_definition(graph, context) {
        for child in graph.children_of(&type_def.id) {
            if classify::is_port_kind(&child.kind) {
                if let Some(name) = &child.name {
                    name_to_usage.entry(name.clone()).or_insert(child.id.clone());
                }
            }
        }
    }

    // Build mapping: context-owned port ID → proxy port ID on the context node
    let context_id = context.id.to_string();
    let mut context_port_ids: HashMap<String, String> = HashMap::new();
    for child in graph.children_of(&context.id) {
        if classify::is_port_kind(&child.kind) {
            context_port_ids.insert(
                child.id.to_string(),
                format!("ctx:{}:{}", context_id, child.id),
            );
        }
    }
    // Also map ports from context's type definition
    if let Some(type_def) = classify::find_type_definition(graph, context) {
        for child in graph.children_of(&type_def.id) {
            if classify::is_port_kind(&child.kind) {
                context_port_ids.insert(
                    child.id.to_string(),
                    format!("ctx:{}:{}", context_id, child.id),
                );
            }
        }
    }

    // Build context IDs set for matching
    let mut context_def_ids: HashSet<String> = HashSet::new();
    context_def_ids.insert(context.id.to_string());
    for child in graph.children_of(&context.id) {
        if child.kind.is_usage() && classify::is_part_kind(&child.kind) {
            context_def_ids.insert(child.id.to_string());
            if let Some(type_def) = classify::find_type_definition(graph, child) {
                context_def_ids.insert(type_def.id.to_string());
            }
        }
    }
    // Include context port IDs so binding edges referencing them are found
    for child in graph.children_of(&context.id) {
        if classify::is_port_kind(&child.kind) {
            context_def_ids.insert(child.id.to_string());
        }
    }

    // Collect candidate edges, then deduplicate
    struct CandidateEdge {
        rel_kind: RelationshipKind,
        edge: DiagramEdge,
        endpoint_key: (String, String),
    }

    let mut candidates: Vec<CandidateEdge> = Vec::new();

    for rel in graph.relationships.values() {
        if !is_ibd_edge_kind(&rel.kind) {
            continue;
        }
        // Only include edges whose origin connector is directly owned by the context block.
        // Connectors inside child definitions (e.g., CircuitPath's internal connections)
        // should not appear in the parent context's IBD.
        let origin_owned_by_context = rel
            .props
            .get("origin_connector")
            .or_else(|| rel.props.get("origin_flow"))
            .and_then(|v| match v {
                sysml_core::Value::Ref(id) => graph.get_element(id),
                _ => None,
            })
            .map(|origin| origin.owner.as_ref() == Some(&context.id))
            .unwrap_or(true); // no origin → use legacy path
        if !origin_owned_by_context {
            continue;
        }
        let src_in_context = is_in_context(graph, &rel.source, &context_def_ids);
        let tgt_in_context = is_in_context(graph, &rel.target, &context_def_ids);
        if src_in_context || tgt_in_context {
            let edge = generate_connection_edge_remapped(
                graph,
                rel,
                &def_to_usage,
                &name_to_usage,
                &context_port_ids,
                &context_id,
            );
            // Safety guard: skip edges where context node ID is an endpoint
            // without a port. With the proxy port ID fix above this should not
            // fire for binding edges, but guards against malformed edges.
            if (edge.source_id == context_id && edge.source_port_id.is_none())
                || (edge.target_id == context_id && edge.target_port_id.is_none())
            {
                continue;
            }
            // Use resolved source+target plus port IDs for dedup key
            let src_key = edge
                .source_port_id
                .as_deref()
                .unwrap_or(&edge.source_id).to_owned();
            let tgt_key = edge
                .target_port_id
                .as_deref()
                .unwrap_or(&edge.target_id).to_owned();
            let endpoint_key = if src_key <= tgt_key {
                (src_key, tgt_key)
            } else {
                (tgt_key, src_key)
            };
            candidates.push(CandidateEdge {
                rel_kind: rel.kind.clone(),
                edge,
                endpoint_key,
            });
        }
    }

    // Deduplicate: for each endpoint pair, keep the edge with highest priority
    let mut best_by_pair: HashMap<(String, String), usize> = HashMap::new();
    for (idx, candidate) in candidates.iter().enumerate() {
        let key = &candidate.endpoint_key;
        if let Some(&existing_idx) = best_by_pair.get(key) {
            if edge_kind_priority(&candidate.rel_kind)
                > edge_kind_priority(&candidates[existing_idx].rel_kind)
            {
                best_by_pair.insert(key.clone(), idx);
            }
        } else {
            best_by_pair.insert(key.clone(), idx);
        }
    }

    let keep_indices: HashSet<usize> = best_by_pair.values().copied().collect();
    candidates
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| keep_indices.contains(idx))
        .map(|(_, c)| c.edge)
        .collect()
}


/// Generate flat edges (no context block) with deduplication.
#[allow(clippy::indexing_slicing)] // candidates indexed by HashMap-stored indices
fn generate_flat_edges(graph: &ModelGraph) -> Vec<DiagramEdge> {
    struct FlatCandidate {
        rel_kind: RelationshipKind,
        edge: DiagramEdge,
        endpoint_key: (String, String),
    }

    let mut candidates: Vec<FlatCandidate> = Vec::new();
    for rel in graph.relationships.values() {
        if is_ibd_edge_kind(&rel.kind) {
            let edge = generate_connection_edge_simple(graph, rel);
            let src_key = edge
                .source_port_id
                .as_deref()
                .unwrap_or(&edge.source_id).to_owned();
            let tgt_key = edge
                .target_port_id
                .as_deref()
                .unwrap_or(&edge.target_id).to_owned();
            let endpoint_key = if src_key <= tgt_key {
                (src_key, tgt_key)
            } else {
                (tgt_key, src_key)
            };
            candidates.push(FlatCandidate {
                rel_kind: rel.kind.clone(),
                edge,
                endpoint_key,
            });
        }
    }

    // Deduplicate
    let mut best: HashMap<(String, String), usize> = HashMap::new();
    for (idx, c) in candidates.iter().enumerate() {
        if let Some(&existing) = best.get(&c.endpoint_key) {
            if edge_kind_priority(&c.rel_kind)
                > edge_kind_priority(&candidates[existing].rel_kind)
            {
                best.insert(c.endpoint_key.clone(), idx);
            }
        } else {
            best.insert(c.endpoint_key.clone(), idx);
        }
    }
    let keep: HashSet<usize> = best.values().copied().collect();
    candidates
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| keep.contains(idx))
        .map(|(_, c)| c.edge)
        .collect()
}

/// Generate a connection edge with definition→usage ID remapping.
fn generate_connection_edge_remapped(
    graph: &ModelGraph,
    rel: &sysml_core::Relationship,
    def_to_usage: &HashMap<String, String>,
    name_to_usage: &HashMap<String, sysml_core::ElementId>,
    context_port_ids: &HashMap<String, String>,
    context_node_id: &str,
) -> DiagramEdge {
    // Try name-based resolution from the origin connector's source/target strings.
    // This correctly handles 1:N mappings (multiple usages of the same type).
    let name_resolved = get_origin_endpoint_names(graph, rel).and_then(|(src_name, tgt_name)| {
        let src = resolve_endpoint_from_origin_name(
            graph,
            src_name,
            name_to_usage,
            context_port_ids,
            context_node_id,
        );
        let tgt = resolve_endpoint_from_origin_name(
            graph,
            tgt_name,
            name_to_usage,
            context_port_ids,
            context_node_id,
        );
        match (src, tgt) {
            (Some(s), Some(t)) => Some((s, t)),
            _ => None,
        }
    });

    let (source_id, source_port_id, target_id, target_port_id);

    if let Some(((sid, spid), (tid, tpid))) = name_resolved {
        source_id = sid;
        source_port_id = spid;
        target_id = tid;
        target_port_id = tpid;
    } else {
        // Fall back to existing def_to_usage remapping (works for 1:1 cases)
        let (mut sid, mut spid) =
            resolve_nested_port_endpoint(graph, &rel.source, def_to_usage)
                .unwrap_or_else(|| resolve_port_endpoint(graph, &rel.source));
        let (mut tid, mut tpid) =
            resolve_nested_port_endpoint(graph, &rel.target, def_to_usage)
                .unwrap_or_else(|| resolve_port_endpoint(graph, &rel.target));

        // Remap context-owned ports to proxy port IDs on the context frame.
        // Use proxy port ID as node endpoint — ELK skips edges with unknown
        // node IDs (no crash), and the custom binding router in the TS side
        // handles the cross-hierarchy routing via obstacle avoidance.
        if let Some(proxy_id) = spid.as_deref().and_then(|pid| context_port_ids.get(pid)) {
            sid = proxy_id.clone();
            spid = Some(proxy_id.clone());
        }
        if let Some(proxy_id) = tpid.as_deref().and_then(|pid| context_port_ids.get(pid)) {
            tid = proxy_id.clone();
            tpid = Some(proxy_id.clone());
        }

        // Remap definition IDs to usage IDs
        if let Some(usage_id) = def_to_usage.get(&sid) {
            if let Some(ref port_id) = spid {
                spid = Some(format!("{}:{}", usage_id, port_id));
            }
            sid = usage_id.clone();
        }
        if let Some(usage_id) = def_to_usage.get(&tid) {
            if let Some(ref port_id) = tpid {
                tpid = Some(format!("{}:{}", usage_id, port_id));
            }
            tid = usage_id.clone();
        }

        source_id = sid;
        source_port_id = spid;
        target_id = tid;
        target_port_id = tpid;
    }

    // Label: named connector > payload type > source→target endpoint paths
    let mut label = edge_label_text(graph, rel);
    if label.is_empty() {
        if let Some((src, tgt)) = get_origin_endpoint_names(graph, rel) {
            label = format!("{} → {}", src, tgt);
        }
    }
    let mut edge = DiagramEdge::relationship(
        rel.id.to_string(),
        source_id,
        target_id,
        rel.kind.clone(),
        label,
    );
    edge.endpoint_mode = EndpointMode::StrictPort;
    edge.label_placement = EdgeLabelPlacement {
        position: 0.5,
        side: "left".to_owned(),
        rotate: false,
        offset: Some(4.0),
    };

    if let Some(sp) = source_port_id {
        edge.source_port_id = Some(sp);
    }
    if let Some(tp) = target_port_id {
        edge.target_port_id = Some(tp);
    }

    // Binding connectors carry the parametric binding-connector tag (notation
    // parity with parametric.rs).
    if rel.kind == RelationshipKind::Binding {
        edge.tags.push(EdgeTag::BindingConnector);
    }

    edge
}

/// Generate a simple connection edge (no remapping, for flat mode).
fn generate_connection_edge_simple(
    graph: &ModelGraph,
    rel: &sysml_core::Relationship,
) -> DiagramEdge {
    let (source_id, source_port_id) = resolve_port_endpoint(graph, &rel.source);
    let (target_id, target_port_id) = resolve_port_endpoint(graph, &rel.target);

    // Label: named connector > payload type > source→target endpoint paths
    let mut label = edge_label_text(graph, rel);
    if label.is_empty() {
        if let Some((src, tgt)) = get_origin_endpoint_names(graph, rel) {
            label = format!("{} → {}", src, tgt);
        }
    }
    let mut edge = DiagramEdge::relationship(
        rel.id.to_string(),
        source_id,
        target_id,
        rel.kind.clone(),
        label,
    );
    edge.endpoint_mode = EndpointMode::StrictPort;
    edge.label_placement = EdgeLabelPlacement {
        position: 0.5,
        side: "left".to_owned(),
        rotate: false,
        offset: Some(4.0),
    };

    if let Some(sp) = source_port_id {
        edge.source_port_id = Some(sp);
    }
    if let Some(tp) = target_port_id {
        edge.target_port_id = Some(tp);
    }

    // Binding connectors carry the parametric binding-connector tag (notation
    // parity with parametric.rs).
    if rel.kind == RelationshipKind::Binding {
        edge.tags.push(EdgeTag::BindingConnector);
    }

    edge
}

// `edge_label_text` now lives in `container.rs` — the General view needs the
// same spec-conformant connector label (§8.2.3.13 `connection-label =
// UsageDeclaration`) and was open-coding `format!("{:?}", rel.kind)` instead,
// which put raw metaclass debug names ("Connection", "Flow") on the canvas.
use super::container::edge_label_text;

// ── Endpoint resolution ──────────────────────────────────────────────────

/// If the element is a port, return (parent_id, Some(port_id)).
/// Otherwise return (element_id, None).
fn resolve_port_endpoint(
    graph: &ModelGraph,
    element_id: &ElementId,
) -> (String, Option<String>) {
    if let Some(element) = graph.get_element(element_id) {
        if classify::is_port_kind(&element.kind) {
            if let Some(owner_id) = &element.owner {
                return (owner_id.to_string(), Some(element_id.to_string()));
            }
        }
    }
    (element_id.to_string(), None)
}

/// Resolve a port endpoint that may be nested inside a composite port definition.
fn resolve_nested_port_endpoint(
    graph: &ModelGraph,
    element_id: &ElementId,
    def_to_usage: &HashMap<String, String>,
) -> Option<(String, Option<String>)> {
    let element = graph.get_element(element_id)?;
    if !classify::is_port_kind(&element.kind) {
        return None;
    }

    let owner_id = element.owner.as_ref()?;
    let owner = graph.get_element(owner_id)?;
    if !owner.kind.is_definition() || !classify::is_port_kind(&owner.kind) {
        return None;
    }

    // Owner is a port definition — find the usage of this definition
    let def_name = owner.name.as_deref()?;
    let usage = graph.elements.values().find(|e| {
        e.kind.is_usage() && classify::is_port_kind(&e.kind) && {
            (e.get_prop("unresolved_type")
                .and_then(|v| v.as_str()) == Some(def_name))
                || graph
                    .children_of(&e.id)
                    .any(|c| {
                        (c.kind == sysml_core::ElementKind::FeatureTyping
                            || c.kind.is_subtype_of(sysml_core::ElementKind::FeatureTyping))
                            && (c.get_prop("unresolved_type")
                                .and_then(|v| v.as_str()) == Some(def_name))
                    })
        }
    })?;

    let composite_owner_id = usage.owner.as_ref()?;
    if let Some(node_usage_id) = def_to_usage.get(&composite_owner_id.to_string()) {
        let scoped_port_id = format!("{}:{}:{}", node_usage_id, usage.id, element_id);
        return Some((node_usage_id.clone(), Some(scoped_port_id)));
    }

    None
}

/// Extract the original source/target endpoint name strings from a relationship's
/// origin connector (or origin flow) element.
fn get_origin_endpoint_names<'a>(
    graph: &'a ModelGraph,
    rel: &sysml_core::Relationship,
) -> Option<(&'a str, &'a str)> {
    let origin_key = match rel.kind {
        RelationshipKind::Flow => "origin_flow",
        _ => "origin_connector",
    };
    let origin_ref = rel.props.get(origin_key)?;
    let sysml_core::Value::Ref(origin_id) = origin_ref else { return None };
    let origin = graph.get_element(origin_id)?;
    let source_name = origin.get_prop("source")?.as_str()?;
    let target_name = origin.get_prop("target")?.as_str()?;
    Some((source_name, target_name))
}

/// Resolve an endpoint from its original connector name string (e.g. `"pathA.phaseIn"`).
///
/// Uses the first segment of a dotted path to find the correct usage among context
/// children by name, then walks the type definition's port hierarchy for remaining
/// segments. Returns `(node_id, Option<port_id>)` with scoped IDs matching
/// `scope_port_ids_ir()` format (`"{usage_id}:{def_port_id}"`).
#[allow(clippy::indexing_slicing)] // `segments[0]` guarded by `.is_empty()` check; `segments[1..]` always valid
fn resolve_endpoint_from_origin_name(
    graph: &ModelGraph,
    name: &str,
    name_to_usage: &HashMap<String, sysml_core::ElementId>,
    context_port_ids: &HashMap<String, String>,
    _context_node_id: &str,
) -> Option<(String, Option<String>)> {
    let segments: Vec<&str> = name.split('.').collect();
    if segments.is_empty() {
        return None;
    }

    let first_segment = segments[0];

    // Check if first segment is a child usage or port
    if let Some(usage_id) = name_to_usage.get(first_segment) {
        let elem = graph.get_element(usage_id)?;
        let is_port = classify::is_port_kind(&elem.kind);

        // If the element is a context port, resolve as proxy port on the context frame
        if is_port {
            if let Some(proxy_id) = context_port_ids.get(&usage_id.to_string()) {
                if segments.len() == 1 {
                    // Use proxy port ID as both node and port endpoint.
                    // ELK skips edges with unknown node IDs (no crash),
                    // and the TS-side custom binding router handles routing.
                    return Some((proxy_id.clone(), Some(proxy_id.clone())));
                }
                // Dotted path through a context port (e.g., circuitOutputs.phase)
                // Walk sub-ports within the port's type definition
                let type_def = classify::find_type_definition(graph, elem);
                let mut search_id = usage_id.clone();
                let mut sub_port_id = None;
                for &seg in &segments[1..] {
                    let found = graph
                        .children_of(&search_id)
                        .find(|e| classify::is_port_kind(&e.kind) && e.name.as_deref() == Some(seg));
                    let found = found.or_else(|| {
                        type_def.as_ref().and_then(|td| {
                            graph.children_of(&td.id).find(|e| {
                                classify::is_port_kind(&e.kind) && e.name.as_deref() == Some(seg)
                            })
                        })
                    });
                    if let Some(fp) = found {
                        sub_port_id = Some(fp.id.clone());
                        search_id = fp.id.clone();
                    } else {
                        return None;
                    }
                }
                // Use the sub-port's proxy ID if available, otherwise connect to
                // the context node without a port (better than a phantom ID that
                // causes ELK to crash).
                let sub_id = sub_port_id?;
                let sub_proxy = context_port_ids.get(&sub_id.to_string()).cloned();
                return Some((proxy_id.clone(), sub_proxy));
            }
        }

        if segments.len() == 1 {
            // Simple name — resolves to the usage/part node itself
            return Some((usage_id.to_string(), None));
        }

        // Dotted path: find port by walking the usage's own children first,
        // then the type definition's children (for inherited ports).
        let type_def = classify::find_type_definition(graph, elem);

        let mut search_owner_id = usage_id.clone();
        let mut port_id = None;
        let mut is_inherited = false;

        for &segment in &segments[1..] {
            // Search direct children of the current owner
            let found = graph
                .children_of(&search_owner_id)
                .find(|e| classify::is_port_kind(&e.kind) && e.name.as_deref() == Some(segment));

            if let Some(found_port) = found {
                port_id = Some(found_port.id.clone());
                search_owner_id = found_port.id.clone();
            } else if let Some(td) = type_def {
                // Fall back to type definition's children (inherited ports)
                let found_in_def = graph
                    .children_of(&td.id)
                    .find(|e| classify::is_port_kind(&e.kind) && e.name.as_deref() == Some(segment));
                if let Some(found_port) = found_in_def {
                    port_id = Some(found_port.id.clone());
                    search_owner_id = found_port.id.clone();
                    is_inherited = true;
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }

        let port_id = port_id?;
        // Inherited ports use scoped IDs "{usage_id}:{def_port_id}" (matching
        // scope_port_ids_ir). Direct/inline ports use raw element IDs.
        let resolved_port_id = if is_inherited {
            format!("{}:{}", usage_id, port_id)
        } else {
            port_id.to_string()
        };
        return Some((usage_id.to_string(), Some(resolved_port_id)));
    }

    None
}

/// Check if an element or any of its ancestors is in the context set.
fn is_in_context(
    graph: &ModelGraph,
    element_id: &ElementId,
    context_ids: &HashSet<String>,
) -> bool {
    let mut current = graph.get_element(element_id);
    while let Some(el) = current {
        if context_ids.contains(&el.id.to_string()) {
            return true;
        }
        current = el.owner.as_ref().and_then(|oid| graph.get_element(oid));
    }
    false
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

    use crate::ir::generator::GeneratorContext;
    use crate::ir::render::render;

    static EMPTY_SET: std::sync::LazyLock<HashSet<String>> =
        std::sync::LazyLock::new(HashSet::new);

    fn make_ctx(graph: &ModelGraph) -> GeneratorContext {
        GeneratorContext::new(graph, &EMPTY_SET)
    }

    fn make_ctx_with<'a>(
        graph: &'a ModelGraph,
        expanded: &'a HashSet<String>,
    ) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded)
    }

    // ── Empty graph ──────────────────────────────────────────────────

    #[test]
    fn ibd_ir_empty_graph() {
        let graph = ModelGraph::new();
        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(ir.view_type, ViewType::Interconnection);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());

        let sgraph = render(&ir);
        assert_eq!(sgraph.type_, "graph");
        assert!(sgraph.children.is_empty());
    }

    // ── Constraint notation parity (Phase 1) ─────────────────────────

    #[test]
    fn ibd_ir_renders_constraint_block_with_expression_and_parametric_port() {
        use crate::ir::types::{DiagramChild, PortTag};

        let mut graph = ModelGraph::new();
        let mut constraint =
            Element::new_with_kind(ElementKind::ConstraintDefinition).with_name("massConstraint");
        constraint.set_prop("constraint", "m == rho * v");
        let constraint_id = graph.add_element(constraint);

        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("m")
            .with_owner(constraint_id.clone());
        graph.add_element(attr);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let node = ir
            .nodes
            .iter()
            .find(|n| n.name == "massConstraint")
            .expect("constraint block should render in the Interconnection view");

        // Expression header text "{expr}" in the Documentation compartment.
        assert!(
            node.children.iter().any(|c| matches!(
                c,
                DiagramChild::Text { text, element_id, .. }
                    if text == "{m == rho * v}"
                        && element_id == &format!("{}/expr", constraint_id)
            )),
            "expression header missing, got children: {:?}",
            node.children
        );

        // Parameter attribute became a small square Parametric port.
        let port = node
            .ports
            .iter()
            .find(|p| p.name == "m")
            .expect("parameter port missing");
        assert!(port.tags.contains(&PortTag::Parametric));
        assert_eq!(port.size, Some((8.0, 8.0)));
    }

    // ── View type metadata ───────────────────────────────────────────

    #[test]
    fn ibd_ir_view_type_and_algorithm() {
        let gen = InterconnectionViewGenerator;
        assert_eq!(gen.view_type(), ViewType::Interconnection);
        assert_eq!(gen.elk_algorithm(), "layered");
        assert_eq!(gen.elk_direction(), Some("RIGHT"));
    }

    // ── Flat mode: two parts with flow ───────────────────────────────

    #[test]
    fn ibd_ir_flat_with_ports_and_flow() {
        let mut graph = ModelGraph::new();

        let part_a = Element::new_with_kind(ElementKind::PartUsage).with_name("PartA");
        let part_a_id = graph.add_element(part_a);
        let port_a = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("outPort")
            .with_owner(part_a_id.clone());
        let port_a_id = graph.add_element(port_a);

        let part_b = Element::new_with_kind(ElementKind::PartUsage).with_name("PartB");
        let part_b_id = graph.add_element(part_b);
        let port_b = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("inPort")
            .with_owner(part_b_id.clone());
        let port_b_id = graph.add_element(port_b);

        let flow =
            Relationship::new(RelationshipKind::Flow, port_a_id.clone(), port_b_id.clone());
        graph.add_relationship(flow);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        // Should have 2 nodes
        assert_eq!(ir.nodes.len(), 2);

        // Nodes should have ports
        let node_a = ir.nodes.iter().find(|n| n.name == "PartA").unwrap();
        assert!(!node_a.ports.is_empty(), "PartA should have ports");

        let node_b = ir.nodes.iter().find(|n| n.name == "PartB").unwrap();
        assert!(!node_b.ports.is_empty(), "PartB should have ports");

        // Should have 1 edge with port routing
        assert_eq!(ir.edges.len(), 1);
        let edge = &ir.edges[0];
        assert!(edge.source_port_id.is_some(), "edge should have source port");
        assert!(edge.target_port_id.is_some(), "edge should have target port");
        assert_eq!(edge.endpoint_mode, EndpointMode::StrictPort);

        // Render to SGraph
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("PartA"));
        assert!(json.contains("PartB"));
        assert!(json.contains("sourcePortId"));
        assert!(json.contains("targetPortId"));
    }

    // ── Expose scoping (declared-view path) ──────────────────────────

    #[test]
    fn ibd_ir_expose_package_renders_member_part_usages_as_peers() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("System");
        let pkg_id = graph.add_element(pkg);

        let pump = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("pump")
            .with_owner(pkg_id.clone());
        let pump_id = graph.add_element(pump);
        let inlet = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("inlet")
            .with_owner(pump_id.clone());
        let inlet_id = graph.add_element(inlet);

        let tank = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("tank")
            .with_owner(pkg_id.clone());
        let tank_id = graph.add_element(tank);
        let drain = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("drain")
            .with_owner(tank_id.clone());
        let drain_id = graph.add_element(drain);

        graph.add_relationship(Relationship::new(RelationshipKind::Flow, drain_id, inlet_id));

        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph).with_expose(&pkg_id);
        let ir = gen.generate(&ctx);

        // Member part usages are peers; the exposed package is never a node.
        assert_eq!(ir.nodes.len(), 2, "pump + tank should be peer nodes");
        assert!(ir.nodes.iter().all(|n| n.name != "System"));
        assert!(ir.nodes.iter().any(|n| n.name == "pump"));
        assert!(ir.nodes.iter().any(|n| n.name == "tank"));
        // The connector owned by the package is a top-level edge.
        assert_eq!(ir.edges.len(), 1, "package-owned connector is a top-level edge");
    }

    #[test]
    fn ibd_ir_expose_part_def_is_the_context_block_without_internal_structure() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("System");
        let pkg_id = graph.add_element(pkg);
        let pump_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Pump")
            .with_owner(pkg_id.clone());
        let pump_def_id = graph.add_element(pump_def);
        let inlet = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("inlet")
            .with_owner(pump_def_id.clone());
        graph.add_element(inlet);

        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph).with_expose(&pump_def_id);
        let ir = gen.generate(&ctx);

        // The exposed part is the single context block, even with no internal
        // part usages — the `has_internal_structure` gate must not apply here.
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "Pump");
        assert!(
            !ir.nodes[0].ports.is_empty(),
            "context block shows its boundary ports"
        );
    }

    #[test]
    fn ibd_ir_expose_missing_target_is_empty_not_a_panic() {
        let graph = ModelGraph::new();
        let bogus = ElementId::from_string("does-not-exist");
        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph).with_expose(&bogus);
        let ir = gen.generate(&ctx);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());
    }

    // ── Context block with inherited ports ────────────────────────────

    #[test]
    fn ibd_ir_context_block_with_inherited_ports() {
        let mut graph = ModelGraph::new();

        // PartDefinition with ports
        let engine_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine");
        let engine_def_id = graph.add_element(engine_def);
        let _fuel_port_id = {
            let fuel_port = Element::new_with_kind(ElementKind::PortUsage)
                .with_name("fuelIn")
                .with_owner(engine_def_id.clone());
            graph.add_element(fuel_port)
        };

        // Context block
        let vehicle = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let vehicle_id = graph.add_element(vehicle);

        // PartUsage typed by Engine
        let mut engine_usage =
            Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
        engine_usage.props.insert(
            "unresolved_type".into(),
            sysml_core::Value::String("Engine".to_string()),
        );
        let engine_usage = engine_usage.with_owner(vehicle_id.clone());
        let _engine_usage_id = graph.add_element(engine_usage);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        // Should have 1 top-level node (Vehicle context)
        assert_eq!(ir.nodes.len(), 1);
        let context_node = &ir.nodes[0];
        assert_eq!(context_node.name, "Vehicle");

        // Context node should contain usage as child node
        let has_usage_child = context_node.children.iter().any(|c| {
            matches!(c, DiagramChild::Node(n) if n.name.contains("engine"))
        });
        assert!(has_usage_child, "context should contain engine usage");

        // Usage should have inherited port from definition
        let engine_child = context_node.children.iter().find_map(|c| match c {
            DiagramChild::Node(n) if n.name.contains("engine") => Some(n),
            _ => None,
        });
        let engine_child = engine_child.unwrap();
        assert!(
            !engine_child.ports.is_empty(),
            "engine usage should inherit fuelIn port"
        );

        // Render roundtrip
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("Vehicle"));
        assert!(json.contains("engine : Engine"));
        assert!(json.contains("fuelIn"));
    }

    // ── Context block with nested parts ──────────────────────────────

    #[test]
    fn ibd_ir_nested_parts() {
        let mut graph = ModelGraph::new();

        let outer = Element::new_with_kind(ElementKind::PartDefinition).with_name("Outer");
        let outer_id = graph.add_element(outer);
        let inner = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("inner")
            .with_owner(outer_id);
        graph.add_element(inner);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("Outer"));
        assert!(json.contains("inner"));
    }

    // ── Edge deduplication ───────────────────────────────────────────

    #[test]
    fn ibd_ir_dedup_keeps_higher_priority_edge() {
        let mut graph = ModelGraph::new();

        let part_a = Element::new_with_kind(ElementKind::PartUsage).with_name("PartA");
        let part_a_id = graph.add_element(part_a);
        let port_a = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portA")
            .with_owner(part_a_id.clone());
        let port_a_id = graph.add_element(port_a);

        let part_b = Element::new_with_kind(ElementKind::PartUsage).with_name("PartB");
        let part_b_id = graph.add_element(part_b);
        let port_b = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portB")
            .with_owner(part_b_id.clone());
        let port_b_id = graph.add_element(port_b);

        // Connection edge (lower priority)
        let conn = Relationship::new(
            RelationshipKind::Connection,
            port_a_id.clone(),
            port_b_id.clone(),
        );
        graph.add_relationship(conn);

        // Binding edge (higher priority)
        let binding = Relationship::new(RelationshipKind::Binding, port_a_id, port_b_id);
        graph.add_relationship(binding);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        // Should have exactly 1 edge after dedup
        assert_eq!(ir.edges.len(), 1, "should have exactly 1 edge after dedup");

        // It should be the binding (higher priority)
        match &ir.edges[0].kind {
            DiagramEdgeKind::Relationship(RelationshipKind::Binding) => {}
            other => panic!("expected Binding edge, got {:?}", other),
        }

        // Render and verify
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("edge:binding"), "should keep binding edge");
        assert!(
            !json.contains("edge:connection"),
            "should drop duplicate connection edge"
        );
    }

    // ── Different endpoints not deduped ──────────────────────────────

    #[test]
    fn ibd_ir_different_endpoints_not_deduped() {
        let mut graph = ModelGraph::new();

        let part_a = Element::new_with_kind(ElementKind::PartUsage).with_name("PartA");
        let part_a_id = graph.add_element(part_a);
        let port_a1 = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portA1")
            .with_owner(part_a_id.clone());
        let port_a1_id = graph.add_element(port_a1);
        let port_a2 = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portA2")
            .with_owner(part_a_id.clone());
        let port_a2_id = graph.add_element(port_a2);

        let part_b = Element::new_with_kind(ElementKind::PartUsage).with_name("PartB");
        let part_b_id = graph.add_element(part_b);
        let port_b1 = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portB1")
            .with_owner(part_b_id.clone());
        let port_b1_id = graph.add_element(port_b1);
        let port_b2 = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("portB2")
            .with_owner(part_b_id.clone());
        let port_b2_id = graph.add_element(port_b2);

        let conn = Relationship::new(RelationshipKind::Connection, port_a1_id, port_b1_id);
        graph.add_relationship(conn);

        let flow = Relationship::new(RelationshipKind::Flow, port_a2_id, port_b2_id);
        graph.add_relationship(flow);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(ir.edges.len(), 2, "different endpoints should both survive");
    }

    // ── Behavioral embedding ─────────────────────────────────────────

    #[test]
    fn ibd_ir_part_with_state_children_has_expand_button() {
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Controller");
        let part_id = graph.add_element(part);
        let _state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(part_id.clone());
        graph.add_element(_state);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let node = &ir.nodes[0];
        assert!(
            !node.buttons.is_empty(),
            "should have expand button"
        );
        assert_eq!(node.expanded, Some(false), "should be collapsed by default");
    }

    #[test]
    fn ibd_ir_expanded_part_embeds_state_island() {
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Controller");
        let part_id = graph.add_element(part);
        let state_def = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("ControllerStates")
            .with_owner(part_id.clone());
        let state_def_id = graph.add_element(state_def);
        let _child_state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("running")
            .with_owner(state_def_id);
        graph.add_element(_child_state);

        let mut expanded = HashSet::new();
        expanded.insert(part_id.to_string());

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx_with(&graph, &expanded));

        let node = &ir.nodes[0];
        assert_eq!(node.expanded, Some(true));

        // Should have a State island
        let has_island = node.children.iter().any(|c| {
            matches!(
                c,
                DiagramChild::Island {
                    view_type: ViewType::StateTransition,
                    ..
                }
            )
        });
        assert!(has_island, "should embed state diagram island");
    }

    #[test]
    fn ibd_ir_part_without_behavioral_no_button() {
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Sensor");
        let part_id = graph.add_element(part);
        let _port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("dataOut")
            .with_owner(part_id);
        graph.add_element(_port);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let node = &ir.nodes[0];
        assert!(node.buttons.is_empty(), "should NOT have expand button");
        assert_eq!(node.expanded, None);
    }

    // ── Proxy ports ──────────────────────────────────────────────────

    #[test]
    fn ibd_ir_context_block_has_proxy_ports() {
        let mut graph = ModelGraph::new();

        let vehicle = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let vehicle_id = graph.add_element(vehicle);

        let vehicle_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("fuelIn")
            .with_owner(vehicle_id.clone());
        let vehicle_port_id = graph.add_element(vehicle_port);

        let engine = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("engine")
            .with_owner(vehicle_id.clone());
        let engine_id = graph.add_element(engine);

        let engine_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("fuelIn")
            .with_owner(engine_id.clone());
        let engine_port_id = graph.add_element(engine_port);

        let binding = Relationship::new(
            RelationshipKind::Binding,
            vehicle_port_id.clone(),
            engine_port_id.clone(),
        );
        graph.add_relationship(binding);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        // Context node should have proxy ports
        assert_eq!(ir.nodes.len(), 1);
        let context_node = &ir.nodes[0];
        let has_proxy = context_node.ports.iter().any(|p| p.is_proxy);
        assert!(has_proxy, "context block should have proxy ports");

        // Should have a binding edge (nested inside context node)
        let has_nested_edges = context_node.children.iter().any(|c| matches!(c, DiagramChild::Edge(_)));
        assert!(
            has_nested_edges,
            "should have edges nested in context node"
        );

        // Proxy port ID should have ctx: prefix
        let proxy_port = context_node.ports.iter().find(|p| p.is_proxy).unwrap();
        let ctx_prefix = format!("ctx:{}:", vehicle_id);
        assert!(
            proxy_port.element_id.starts_with(&ctx_prefix),
            "proxy port should have ctx: prefix, got: {}",
            proxy_port.element_id
        );

        // Render to SGraph
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("proxy"), "should have proxy CSS class");
        assert!(json.contains("edge:binding"), "should have binding edge");
    }

    // ── generate_for_owner ───────────────────────────────────────────

    #[test]
    fn ibd_ir_generate_for_owner_returns_subtree() {
        let mut graph = ModelGraph::new();

        let vehicle = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let vehicle_id = graph.add_element(vehicle);

        let engine = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("engine")
            .with_owner(vehicle_id.clone());
        graph.add_element(engine);

        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen
            .generate_for_owner(&ctx, &vehicle_id.to_string())
            .expect("should return Some for element with internal structure");

        assert_eq!(ir.view_type, ViewType::Interconnection);
        assert!(!ir.nodes.is_empty(), "should have usage nodes");

        // Node IDs should be scoped with owner prefix
        let first_node = &ir.nodes[0];
        let expected_prefix = format!("owner-{}/", vehicle_id);
        assert!(
            first_node.element_id.starts_with(&expected_prefix),
            "node ID should be scoped, got: {}",
            first_node.element_id
        );
    }

    #[test]
    fn ibd_ir_generate_for_owner_nonexistent_returns_none() {
        let graph = ModelGraph::new();
        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate_for_owner(&ctx, "nonexistent");
        assert!(ir.is_none());
    }

    #[test]
    fn ibd_ir_generate_for_owner_no_internal_structure_returns_none() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Leaf");
        let part_id = graph.add_element(part);

        let gen = InterconnectionViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate_for_owner(&ctx, &part_id.to_string());
        assert!(ir.is_none(), "element without internal structure should return None");
    }

    // ── Render roundtrip ─────────────────────────────────────────────

    #[test]
    fn ibd_ir_renders_to_sgraph() {
        let mut graph = ModelGraph::new();

        let vehicle = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let vehicle_id = graph.add_element(vehicle);

        let engine = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("engine")
            .with_owner(vehicle_id.clone());
        let engine_id = graph.add_element(engine);

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("fuelIn")
            .with_owner(engine_id);
        graph.add_element(port);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(json.contains("Vehicle"));
        assert!(json.contains("engine"));
        assert!(json.contains("fuelIn"));
    }

    // ── Edge label text ──────────────────────────────────────────────

    #[test]
    fn ibd_ir_edge_label_from_origin() {
        let mut graph = ModelGraph::new();

        let part_a = Element::new_with_kind(ElementKind::PartUsage).with_name("PartA");
        let part_a_id = graph.add_element(part_a);

        let part_b = Element::new_with_kind(ElementKind::PartUsage).with_name("PartB");
        let part_b_id = graph.add_element(part_b);

        let mut flow = Relationship::new(
            RelationshipKind::Flow,
            part_a_id.clone(),
            part_b_id.clone(),
        );
        // Create an origin flow element
        let origin = Element::new_with_kind(ElementKind::FlowUsage).with_name("fuelFlow");
        let origin_id = graph.add_element(origin);
        flow.props.insert(
            "origin_flow".into(),
            sysml_core::Value::Ref(origin_id),
        );
        graph.add_relationship(flow);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(ir.edges.len(), 1);
        assert_eq!(ir.edges[0].label, "fuelFlow");
    }

    // ── Port direction (R4, §7.12.1): DECLARED-only ──────────────────

    #[test]
    fn ibd_ir_bare_ports_have_no_inferred_direction() {
        // R4: a bare `port dataIn;` / `port dataOut;` declares NO direction.
        // Inferring in/out from the camelCase name is non-normative, so these
        // ports render direction-less (neutral coloring, no fixed W/E side).
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Block");
        let part_id = graph.add_element(part);

        let port_in = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("dataIn")
            .with_owner(part_id.clone());
        graph.add_element(port_in);

        let port_out = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("dataOut")
            .with_owner(part_id);
        graph.add_element(port_out);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let node = &ir.nodes[0];
        assert_eq!(node.ports.len(), 2);

        let pin = node.ports.iter().find(|p| p.name == "dataIn").unwrap();
        assert_eq!(pin.direction, None, "bare port name must not infer a direction");
        assert_eq!(pin.side, None, "bare port must not infer a fixed side");

        let pout = node.ports.iter().find(|p| p.name == "dataOut").unwrap();
        assert_eq!(pout.direction, None);
        assert_eq!(pout.side, None);
    }

    #[test]
    fn ibd_ir_declared_direction_drives_placement() {
        // The conformant path: a port that DECLARES `in`/`out` keeps its
        // direction (coloring) and W/E side placement.
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Block");
        let part_id = graph.add_element(part);

        let mut port_in = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("feed")
            .with_owner(part_id.clone());
        port_in.set_prop("direction", "in");
        graph.add_element(port_in);

        let mut port_out = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("drain")
            .with_owner(part_id);
        port_out.set_prop("direction", "out");
        graph.add_element(port_out);

        let gen = InterconnectionViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let node = &ir.nodes[0];
        let pin = node.ports.iter().find(|p| p.name == "feed").unwrap();
        assert_eq!(pin.direction, Some(PortDirection::In));
        assert_eq!(pin.side, Some(PortSide::West));

        let pout = node.ports.iter().find(|p| p.name == "drain").unwrap();
        assert_eq!(pout.direction, Some(PortDirection::Out));
        assert_eq!(pout.side, Some(PortSide::East));
    }
}
