//! GeneralView IR generator.
//!
//! Produces `DiagramIR` for Block Definition Diagrams (BDD), Package Diagrams,
//! and composite diagrams. This is the most feature-rich generator:
//!
//! - Top-level element filtering (`is_effectively_top_level` + `is_bdd_relevant`)
//! - Context-aware visual kind (actor detection via `effective_graphical_kind`)
//! - Metadata stereotype overrides (MetadataUsage children)
//! - Documentation compartments (element doc + TextualRepresentation children)
//! - Relationship-reference compartments (satisfies, verifies, includes, performedBy, successions)
//! - Enumeration literals compartment
//! - Collapsed: text labels in typed compartments via `compartment_for_element()`
//! - Expanded: direct child DiagramNodes
//! - Sub-diagram island embedding (State, Action, Sequence, IBD)
//! - Edge rerouting for collapsed endpoints (`find_rendered_ancestor`)
//! - N-ary connection dot detection
//! - Port rendering

use std::collections::HashSet;

use sysml_core::{Element, ElementKind, ModelGraph, RelationshipKind};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramEdge, DiagramEdgeKind, DiagramNode, HeaderStyle, DiagramChild, DiagramPort, PortDirection, CompartmentItemSource, NodeTag, EdgeTag};
use crate::smodel::builders;
use crate::smodel::ViewType;
use crate::visual_kind::{self as classify, CompartmentKind, VisualKind};

/// Generates General (BDD) diagrams.
pub struct GeneralViewGenerator;

impl ViewGenerator for GeneralViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::General
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!(
            expanded_count = ctx.expanded_ids.len(),
            "GeneralView IR generate"
        );

        let graph = ctx.graph;
        let mut ir = DiagramIR::new(ViewType::General);

        // Auto-expand root-level packages that have a manageable number of
        // BDD-relevant children. Large packages (e.g., 100+ requirements) stay
        // collapsed to avoid overwhelming ELK layout.
        const AUTO_EXPAND_MAX_CHILDREN: usize = 30;
        let mut auto_expanded = ctx.expanded_ids.clone();
        for element in graph.elements.values() {
            if matches!(
                element.kind,
                ElementKind::Package | ElementKind::LibraryPackage
            ) && element.owner.is_none()
            {
                let child_count = graph
                    .children_of(&element.id)
                    .filter(|c| !classify::is_membership_kind(&c.kind))
                    .filter(|c| !classify::is_import_kind(&c.kind))
                    .filter(|c| classify::is_bdd_relevant(c))
                    .count();
                if child_count <= AUTO_EXPAND_MAX_CHILDREN {
                    auto_expanded.insert(element.id.to_string());
                }
            }
        }
        let expanded_ids = &auto_expanded;

        // Collect top-level elements. An element is top-level if:
        // - It has no owner, OR
        // - Its owner is expanded (children rendered inside expanded parent are
        //   excluded — they are generated as nested children, not top-level nodes)
        // - Its owner is a collapsed Package that will be rendered as a node
        //   (children shown as text compartments inside the package, not as
        //   separate nodes beside it)
        //
        // The key insight: elements owned by a collapsed Package should NOT appear
        // as top-level nodes — they're already rendered as compartment text inside
        // the package. Only truly ownerless elements and elements whose Package
        // ancestor chain leads to the root should be top-level.
        let top_level: Vec<_> = graph
            .elements
            .values()
            .filter(|e| !classify::is_membership_kind(&e.kind))
            .filter(|e| !classify::is_import_kind(&e.kind))
            .filter(|e| !classify::is_port_kind(&e.kind))
            .filter(|e| e.kind != ElementKind::MetadataUsage) // rendered as compartment text
            .filter(|e| classify::is_bdd_relevant(e))
            // No-expose path: exclude standard-library content so a view with
            // no Expose clause doesn't dump the whole stdlib as top-level nodes
            // (tracker 3.10; mirrors the Grid/Sequence WI2 exclusion). When
            // `expose` is set, `is_canvas_root` already restricts the canvas to
            // the exposed element, so an author who explicitly exposes a library
            // element still gets it.
            .filter(|e| !ctx.expose_ids.is_empty() || !graph.is_library_element(&e.id))
            .filter(|e| {
                // Phase 5 expose: when an authored ViewUsage names an
                // exposed subject, the canvas is centred on it
                // regardless of nesting. Short-circuit the package-aware
                // filter below.
                if !ctx.expose_ids.is_empty() {
                    return ctx.is_canvas_root(e);
                }
                match &e.owner {
                    None => true,
                    Some(owner_id) => {
                        // If owner is expanded, children are rendered inside it
                        if expanded_ids.contains(&owner_id.to_string()) {
                            return false;
                        }
                        // If owner is a Package/Namespace, this element is rendered
                        // as compartment text inside the package, not standalone.
                        if let Some(owner) = graph.get_element(owner_id) {
                            if matches!(
                                owner.kind,
                                ElementKind::Package | ElementKind::LibraryPackage
                            ) {
                                return false;
                            }
                        }
                        // For non-package owners, check if effectively top-level
                        classify::is_effectively_top_level(e, graph)
                    }
                }
            })
            // Spec ViewFilter (4.5): apply at top-level collection. Edges
            // into filtered-out elements get pruned by the existing dangling-
            // endpoint check in the edge loop below.
            .filter(|e| ctx.passes_filter(e))
            .collect();
        // C13: deterministic source-order for top-level nodes — the elements
        // map is hash-ordered (random per run since ElementIds are random).
        let top_level = {
            let mut v = top_level;
            sysml_core::element_ordering::sort_elements_by_source_order(&mut v);
            v
        };

        // Build set of IDs that will exist as rendered nodes (not text labels).
        let mut rendered_node_ids: HashSet<String> = HashSet::new();
        collect_rendered_node_ids(ctx, expanded_ids, &mut rendered_node_ids);

        // Generate nodes for top-level elements
        for element in &top_level {
            let node = generate_node(graph, element, expanded_ids);
            ir.nodes.push(node);
        }

        // Generate edges for non-ownership relationships.
        // When an endpoint is inside a collapsed container (rendered as text, not a
        // node), reroute the edge to the nearest rendered ancestor.
        // Skip edges where both endpoints share the same expanded ancestor — those
        // are internal to a sub-diagram and already rendered by the subtree generators.
        for rel in graph.relationships.values() {
            if rel.kind == RelationshipKind::Owning {
                continue;
            }

            let source_str = rel.source.to_string();
            let target_str = rel.target.to_string();

            // Skip edges that are internal to the same expanded behavioral container.
            if let (Some(sa), Some(ta)) = (
                find_expanded_ancestor(graph, &rel.source, expanded_ids),
                find_expanded_ancestor(graph, &rel.target, expanded_ids),
            ) {
                if sa == ta {
                    if let Some(ancestor_el) = graph.get_element(&sa) {
                        let is_behavioral = matches!(
                            ancestor_el.kind,
                            ElementKind::StateDefinition | ElementKind::ActionDefinition
                        );
                        let is_structural_with_behavioral = matches!(
                            VisualKind::from_element_kind(&ancestor_el.kind),
                            VisualKind::Part
                                | VisualKind::Item
                                | VisualKind::Connection
                                | VisualKind::Occurrence
                        ) && graph.children_of(&ancestor_el.id).any(|c| {
                            classify::is_state_kind(&c.kind)
                                || classify::is_action_kind(&c.kind)
                                || matches!(
                                    c.kind,
                                    ElementKind::FlowUsage | ElementKind::SuccessionFlowUsage
                                )
                        });
                        if is_behavioral || is_structural_with_behavioral {
                            continue;
                        }
                    }
                }
            }

            // Resolve endpoints: use directly if rendered, otherwise walk up to ancestor
            let effective_source = if rendered_node_ids.contains(&source_str) {
                source_str
            } else if let Some(ancestor) =
                find_rendered_ancestor(graph, &rel.source, &rendered_node_ids)
            {
                ancestor
            } else {
                continue;
            };

            let effective_target = if rendered_node_ids.contains(&target_str) {
                target_str
            } else if classify::is_requirement_relationship(&rel.kind) {
                // A verify/satisfy target is a requirement USAGE (spec: the
                // RequirementVerificationMembership's verifiedRequirement).
                // When the usage isn't on canvas, the ancestor walk lands the
                // edge back on its own verification case (self-loop →
                // suppressed), erasing the semantic edge and leaving only the
                // usage's raw typing edge (D-N7). The rendered requirement
                // DEFINITION the usage is typed by is the visible proxy.
                match typed_definition_proxy(graph, &rel.target, &rendered_node_ids)
                    .or_else(|| find_rendered_ancestor(graph, &rel.target, &rendered_node_ids))
                {
                    Some(t) => t,
                    None => continue,
                }
            } else if let Some(ancestor) =
                find_rendered_ancestor(graph, &rel.target, &rendered_node_ids)
            {
                ancestor
            } else {
                continue;
            };

            // Skip self-loops on collapsed containers (meaningless visually)
            if effective_source == effective_target {
                continue;
            }

            // Edge label vocabulary, per SysML v2 §8.2.3 graphical BNF:
            //
            // - Requirement relationships (satisfy/verify/derive/trace) carry a
            //   guillemet-wrapped stereotype label.
            // - Connector-family edges (connection/flow/binding/allocate/
            //   interface) are the ONLY kinds with a text-label production —
            //   `connection-label = UsageDeclaration` — so they show the
            //   connector usage's declared name (`engineToGearbox`, `torqueFlow`).
            // - Everything else gets NO label: its production is a bare image,
            //   i.e. line style + arrowhead carry the meaning. Any «keyword»
            //   the spec does show (e.g. «redefines») comes from the single
            //   vocabulary home, `EdgeStyle::from_relationship_kind`, which the
            //   renderer applies as `edge.label || style.label`.
            //
            // This previously emitted `format!("{:?}", rel.kind)` — the Rust
            // metaclass debug name. That is how "Connection" / "Flow" ended up
            // painted on AllPartsView; the spec contains no such labels (a grep
            // for `typing` across the whole spec returns zero hits).
            let label_text = if classify::is_requirement_relationship(&rel.kind) {
                format!("\u{00ab}{}\u{00bb}", rel.kind.as_str().to_lowercase())
            } else if super::container::is_connector_kind(&rel.kind) {
                super::container::edge_label_text(graph, rel)
            } else {
                String::new()
            };
            let edge = DiagramEdge::relationship(
                rel.id.to_string(),
                effective_source,
                effective_target,
                rel.kind.clone(),
                label_text,
            );
            ir.edges.push(edge);
        }

        // Generate edges from element-based relationships (Subclassification,
        // FeatureTyping, Redefinition, Subsetting). These are stored as Elements
        // (not Relationships) whose owner is the source and a resolved prop holds
        // the target ElementId.
        //
        // Tuples: (ElementKind, resolved_prop, unresolved_prop, RelationshipKind)
        // The resolved prop contains Value::Ref(ElementId) after resolution.
        // The unresolved prop contains Value::String(name) from the parser.
        // We try resolved first, then fall back to resolving the name.
        //
        // NO label column: §8.2.3.6 gives this whole family bare-image BNF
        // productions — solid line + open triangle (plus a tick for
        // redefinition). The literal strings "specialization"/"typing"/
        // "redefinition"/"subsetting" that used to live here appear nowhere in
        // the spec, and stacking them on the canvas is what made AllPartsView's
        // `typing` knot (two wheels typed by Wheel → two identically-labelled
        // arrows over `frontRight : Wheel`). Where the spec DOES show a keyword
        // («redefines», «subsets») it comes from the one vocabulary home,
        // `EdgeStyle::from_relationship_kind`, applied by the renderer as
        // `edge.label || style.label`.
        let rel_edge_kinds: &[(ElementKind, &str, &str, RelationshipKind)] = &[
            (ElementKind::Subclassification, "general", "unresolved_superclassifier", RelationshipKind::Specialize),
            (ElementKind::FeatureTyping, "type", "unresolved_type", RelationshipKind::TypeOf),
            (ElementKind::Redefinition, "redefinedFeature", "unresolved_redefinedFeature", RelationshipKind::Redefine),
            (ElementKind::Subsetting, "subsettedFeature", "unresolved_subsettedFeature", RelationshipKind::Subsetting),
        ];
        for (kind, resolved_prop, unresolved_prop, rel_kind) in rel_edge_kinds {
            for element in graph.elements_by_kind(kind) {
                let source_id = match &element.owner {
                    Some(id) => id,
                    None => continue,
                };
                // Try resolved property first (Value::Ref from resolution pass),
                // then fall back to name-based lookup from unresolved property
                let target_id = if let Some(id) = element.get_prop(resolved_prop).and_then(|v| v.as_ref()) {
                    id.clone()
                } else if let Some(name) = element.get_prop(unresolved_prop).and_then(|v| v.as_str()) {
                    // Resolve name to ElementId by searching rendered nodes
                    if let Some(found) = graph.elements.values().find(|e| {
                        e.name.as_deref() == Some(name) && rendered_node_ids.contains(&e.id.to_string())
                    }) {
                        found.id.clone()
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                let source_str = source_id.to_string();
                let target_str = target_id.to_string();

                // Resolve to rendered nodes (same logic as relationship edges)
                let effective_source = if rendered_node_ids.contains(&source_str) {
                    source_str
                } else if let Some(ancestor) =
                    find_rendered_ancestor(graph, source_id, &rendered_node_ids)
                {
                    ancestor
                } else {
                    continue;
                };

                let effective_target = if rendered_node_ids.contains(&target_str) {
                    target_str
                } else if let Some(ancestor) =
                    find_rendered_ancestor(graph, &target_id, &rendered_node_ids)
                {
                    ancestor
                } else {
                    continue;
                };

                if effective_source == effective_target {
                    continue;
                }

                let edge = DiagramEdge::relationship(
                    element.id.to_string(),
                    effective_source,
                    effective_target,
                    rel_kind.clone(),
                    String::new(),
                );
                ir.edges.push(edge);
            }
        }

        // Collapse specialization-family edges that the ancestor fold made
        // indistinguishable.
        //
        // `frontLeft : Wheel` and `frontRight : Wheel` are two distinct
        // FeatureTyping elements, but neither usage is a rendered node, so both
        // fold to (Vehicle → Wheel). The fold is lossy: once the distinguishing
        // endpoint is gone, and with no text label to tell them apart (see
        // above — the spec gives this family none), the second arrow carries
        // exactly zero information and just adds a crossing over the box it
        // lands on. One arrow already states "Vehicle has a feature typed by
        // Wheel"; drawing it twice does not say it twice.
        //
        // Deliberately scoped to the label-less family. Two Connections between
        // the same pair come from different connector usages and DO carry their
        // own declared names, so they stay distinguishable and both survive.
        {
            let mut seen: HashSet<(String, String, RelationshipKind)> = HashSet::new();
            ir.edges.retain(|e| {
                let DiagramEdgeKind::Relationship(k) = &e.kind else { return true };
                if !matches!(
                    k,
                    RelationshipKind::TypeOf
                        | RelationshipKind::Specialize
                        | RelationshipKind::Redefine
                        | RelationshipKind::Subsetting
                ) {
                    return true;
                }
                seen.insert((e.source_id.clone(), e.target_id.clone(), k.clone()))
            });
        }

        // D-N7 dedup: a typing edge whose (source, target) pair duplicates a
        // requirement-relationship edge is notation noise — the semantic edge
        // («verify»/«satisfy»/…) wins; the typing arrow between the same two
        // boxes restates the same link as its mechanism.
        let req_pairs: HashSet<(String, String)> = ir
            .edges
            .iter()
            .filter(|e| {
                matches!(&e.kind, DiagramEdgeKind::Relationship(k)
                    if classify::is_requirement_relationship(k))
            })
            .map(|e| (e.source_id.clone(), e.target_id.clone()))
            .collect();
        if !req_pairs.is_empty() {
            ir.edges.retain(|e| {
                !(matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::TypeOf))
                    && req_pairs.contains(&(e.source_id.clone(), e.target_id.clone())))
            });
        }

        // N-ary connection dot detection.
        // ConnectionUsage elements with > 2 end features should render as an NaryDot
        // central node with individual segment edges to each end.
        for element in graph.elements.values() {
            if !matches!(element.kind, ElementKind::ConnectionUsage) {
                continue;
            }
            if !rendered_node_ids.contains(&element.id.to_string()) {
                continue;
            }

            let end_features: Vec<_> = graph
                .children_of(&element.id)
                .filter(|c| {
                    c.get_prop("isEnd")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .collect();

            if end_features.len() > 2 {
                let dot_id = format!("{}/nary-dot", element.id);
                let mut dot_node = DiagramNode::new(
                    dot_id.clone(),
                    VisualKind::Generic,
                    String::new(),
                );
                dot_node.header_style = HeaderStyle::None;
                dot_node.size = Some((12.0, 12.0));
                dot_node.tags.push(NodeTag::NaryDot);
                dot_node.tooltip =
                    Some(format!("n-ary connection ({} ends)", end_features.len()));
                ir.nodes.push(dot_node);

                // Generate segment edges from each end feature to the dot
                for (i, end_feat) in end_features.iter().enumerate() {
                    let edge_id = format!("{}/nary-seg-{}", element.id, i);
                    let mut edge = DiagramEdge::relationship(
                        edge_id,
                        end_feat.id.to_string(),
                        dot_id.clone(),
                        RelationshipKind::Reference,
                        String::new(),
                    );
                    edge.tags.push(EdgeTag::NarySegment);
                    ir.edges.push(edge);
                }
            }
        }

        ir
    }

    fn generate_for_owner(
        &self,
        ctx: &GeneratorContext,
        owner_id: &str,
    ) -> Option<DiagramIR> {
        // The General view can generate a subtree for a specific owner —
        // used when another view embeds a General sub-diagram.
        let graph = ctx.graph;
        let expanded_ids = ctx.expanded_ids;
        let owner_eid = sysml_core::ElementId::from_string(owner_id);
        let element = graph.get_element(&owner_eid)?;

        let mut ir = DiagramIR::new(ViewType::General);
        let node = generate_node(graph, element, expanded_ids);
        ir.nodes.push(node);
        Some(ir)
    }
}

// ── Helper functions ─────────────────────────────────────────────────────

/// Find the nearest expanded ancestor of an element.
fn find_expanded_ancestor(
    graph: &ModelGraph,
    element_id: &sysml_core::ElementId,
    expanded_ids: &HashSet<String>,
) -> Option<sysml_core::ElementId> {
    super::container::find_ancestor_by(graph, element_id, |eid| {
        expanded_ids.contains(&eid.to_string())
    })
}

/// Walk up the ownership chain to find the nearest ancestor that is rendered
/// **and is a real container** (not a namespace).
///
/// 3.15: an off-canvas edge endpoint may be lifted to its nearest rendered
/// PartDefinition/PartUsage/etc., but NOT promoted all the way to a root
/// `Package`/`LibraryPackage` — doing so manufactures Package→Package edges that
/// correspond to no authored relationship at that level. When the only rendered
/// ancestor is a namespace, return `None` so the edge is pruned by the
/// dangling-endpoint check rather than rerouted to a package box.
/// The rendered requirement DEFINITION a requirement usage is typed by, as a
/// visible proxy endpoint for requirement-relationship edges whose usage
/// target is off-canvas (D-N7 — see the call site).
fn typed_definition_proxy(
    graph: &ModelGraph,
    usage_id: &sysml_core::ElementId,
    rendered: &HashSet<String>,
) -> Option<String> {
    let usage = graph.get_element(usage_id)?;
    let def = classify::find_type_definition(graph, usage)?;
    let def_str = def.id.to_string();
    rendered.contains(&def_str).then_some(def_str)
}

fn find_rendered_ancestor(
    graph: &ModelGraph,
    element_id: &sysml_core::ElementId,
    rendered_node_ids: &HashSet<String>,
) -> Option<String> {
    super::container::find_ancestor_by(graph, element_id, |eid| {
        rendered_node_ids.contains(&eid.to_string())
            && graph.get_element(eid).is_some_and(|e| {
                !matches!(
                    e.kind,
                    ElementKind::Package | ElementKind::LibraryPackage | ElementKind::Namespace
                )
            })
    })
    .map(|eid| eid.to_string())
}

/// Collect element IDs that will be rendered as connectable shapes in the GeneralView.
///
/// Honours the `GeneratorContext`:
/// - When `ctx.expose_ids` is non-empty, only the exposed elements and their descendants
///   count as rendered. Edges to other elements get pruned by the dangling-
///   endpoint check, instead of slipping through and bloating the SGraph
///   payload.
/// - When `ctx.filter` is set, elements that fail `passes_filter` are
///   excluded for the same reason.
fn collect_rendered_node_ids(
    ctx: &GeneratorContext,
    expanded_ids: &HashSet<String>,
    rendered: &mut HashSet<String>,
) {
    let graph = ctx.graph;
    for element in graph.elements.values() {
        if classify::is_membership_kind(&element.kind) || classify::is_import_kind(&element.kind) {
            continue;
        }
        if !classify::is_bdd_relevant(element) {
            continue;
        }
        if !ctx.passes_filter(element) {
            continue;
        }
        // No-expose path: exclude standard-library content (tracker 3.10).
        // Must mirror the top-level node collection in `generate()` — otherwise
        // stdlib IDs would enter `rendered` and the edge loop would emit edges
        // referencing nodes that were never pushed into `ir.nodes`. When
        // `expose` is set, `is_within_canvas_subject` already bounds the canvas.
        if ctx.expose_ids.is_empty() && graph.is_library_element(&element.id) {
            continue;
        }
        if !is_within_canvas_subject(ctx, element) {
            continue;
        }

        let is_top_level = classify::is_effectively_top_level(element, graph);
        let parent_expanded = element
            .owner
            .as_ref()
            .map(|owner_id| expanded_ids.contains(&owner_id.to_string()))
            .unwrap_or(false);
        // Children of root-level packages are promoted to top-level nodes
        let promoted_from_root_pkg = element.owner.as_ref().map_or(false, |owner_id| {
            graph.get_element(owner_id).map_or(false, |owner| {
                matches!(owner.kind, ElementKind::Package | ElementKind::LibraryPackage)
                    && owner.owner.is_none()
            })
        });

        if is_top_level || parent_expanded || promoted_from_root_pkg {
            rendered.insert(element.id.to_string());
        }
    }

    // Ports are rendered as SPort only when their owner is rendered as an SNode.
    for port in graph
        .elements
        .values()
        .filter(|e| classify::is_port_kind(&e.kind))
    {
        if let Some(owner_id) = &port.owner {
            if rendered.contains(&owner_id.to_string()) {
                rendered.insert(port.id.to_string());
            }
        } else {
            rendered.insert(port.id.to_string());
        }
    }
}

/// True when `element` is the canvas subject (or any of its descendants)
/// while `ctx.expose_ids` is non-empty. When expose is unset every element passes —
/// this is the no-op case for the unexposed General view.
fn is_within_canvas_subject(ctx: &GeneratorContext, element: &Element) -> bool {
    if ctx.expose_ids.is_empty() {
        return true;
    }
    for subject_id in ctx.expose_ids {
        if &element.id == subject_id {
            return true;
        }
        if super::container::find_ancestor_by(ctx.graph, &element.id, |id| id == subject_id).is_some() {
            return true;
        }
    }
    false
}

/// Check if an element has children worth expanding to full nested node boxes.
///
/// Any node with non-port, BDD-relevant STRUCTURAL children is expandable.
/// When expanded, those children render as nested DiagramChild::Node entries
/// inside the parent container. Value features (scalar attributes) and
/// doc/comment children render as compartment text in BOTH modes (C11/C12a),
/// so they don't make a node expandable on their own.
fn has_expandable_children(graph: &ModelGraph, element: &Element) -> bool {
    graph
        .children_of(&element.id)
        .filter(|c| !classify::is_membership_kind(&c.kind))
        .filter(|c| !classify::is_import_kind(&c.kind))
        .filter(|c| classify::is_bdd_relevant(c))
        .filter(|c| !classify::is_port_kind(&c.kind))
        .filter(|c| !matches!(c.kind, ElementKind::Comment | ElementKind::Documentation))
        .any(|c| !super::container::is_value_feature(c, graph))
}

/// Generate a DiagramNode for an element in the General view.
fn generate_node(
    graph: &ModelGraph,
    element: &Element,
    expanded_ids: &HashSet<String>,
) -> DiagramNode {
    let id = element.id.to_string();
    let kind = &element.kind;
    let name = crate::smodel::builders::element_display_name(element, graph);

    // Use context-aware graphical kind (e.g. PartUsage via ActorMembership -> Actor)
    let gk = classify::effective_graphical_kind(element, graph);

    // Metadata stereotype override: elements typed by a MetadataDefinition use
    // the metadata type name as their stereotype instead of the default element-kind
    // stereotype.
    let stereotype = {
        let metadata_type_name = graph
            .outgoing(&element.id)
            .find(|r| r.kind == RelationshipKind::TypeOf)
            .and_then(|r| graph.get_element(&r.target))
            .filter(|target| matches!(target.kind, ElementKind::MetadataDefinition))
            .and_then(|target| target.name.clone());
        if let Some(meta_name) = metadata_type_name {
            format!("\u{00ab}{}\u{00bb}", meta_name)
        } else {
            builders::stereotype_text(kind)
        }
    };

    let is_expanded = expanded_ids.contains(&id);

    let mut node = DiagramNode::new(id.clone(), gk, name)
        .with_stereotype(stereotype)
        .with_element_kind(kind.clone());

    // Typed style tags (replaces former CSS-class strings)
    if gk == VisualKind::Actor {
        node.tags.push(NodeTag::Actor);
    }
    node.tags.extend(classify::property_tags(element));

    // Source location and tooltip
    super::container::apply_source_metadata(&mut node, element, graph);

    // ── Requirement compartments (reqId, Constraints, subject, assume/require,
    //    nested requirements) — shared notation home in container.rs so the
    //    General render path matches the legacy requirements.rs peer generator
    //    (Phase 1 of the requirements/parametric retirement). Gated on element
    //    kind, never on view kind. Nested requirements recurse through the same
    //    `generate_node` builder. ──
    if classify::is_requirement_kind(kind) {
        super::container::apply_requirement_compartments(
            &mut node,
            element,
            graph,
            |g, e| generate_node(g, e, expanded_ids),
        );
    }

    // ── Documentation compartment for non-requirement elements ──
    // Includes both the element's own `documentation` property and any
    // TextualRepresentation children.
    if !classify::is_requirement_kind(kind) {
        let mut doc_children: Vec<DiagramChild> = Vec::new();

        if let Some(doc) = element.get_prop("documentation") {
            let raw = doc.to_string().trim_matches('"').to_owned();
            let lines = super::container::wrap_doc_text(&raw);
            for (li, line) in lines.into_iter().enumerate() {
                doc_children.push(DiagramChild::Text {
                    compartment: CompartmentKind::Documentation,
                    text: line,
                    element_id: format!("{}/documentation/text/{}", id, li),
                    source: CompartmentItemSource::Owned,
                });
            }
        }

        // TextualRepresentation children
        for (i, tr_child) in graph
            .children_of(&element.id)
            .filter(|c| c.kind == ElementKind::TextualRepresentation)
            .enumerate()
        {
            let body = tr_child
                .get_prop("body")
                .or_else(|| tr_child.get_prop("language"))
                .map(|v| v.to_string().trim_matches('"').to_owned())
                .unwrap_or_else(|| {
                    tr_child
                        .name
                        .as_deref()
                        .unwrap_or("(textual representation)").to_owned()
                });
            if !body.is_empty() {
                let lines = super::container::wrap_doc_text(&body);
                for (li, line) in lines.into_iter().enumerate() {
                    doc_children.push(DiagramChild::Text {
                        compartment: CompartmentKind::Documentation,
                        text: line,
                        element_id: format!("{}/documentation/textrep-{}/{}", id, i, li),
                        source: CompartmentItemSource::Owned,
                    });
                }
            }
        }

        if !doc_children.is_empty() {
            node.children.push(DiagramChild::Compartment {
                kind: CompartmentKind::Documentation,
                children: doc_children,
            });
        }
    }

    // ── Redefinitions compartment (expanded only) ──
    // For expanded nodes, unnamed redefinition attributes are skipped in
    // render_expanded_children() and rendered here as compact "name = value" text.
    // Collapsed nodes get this via render_collapsed_children() routing.
    if is_expanded {
        for child in super::container::ordered_children(graph, &element.id) {
            if child.name.is_some() {
                continue;
            }
            if let Some(text) = super::container::redefinition_text_pub(child, graph) {
                node.children.push(DiagramChild::Text {
                    compartment: CompartmentKind::Redefinitions,
                    text,
                    element_id: child.id.to_string(),
                    source: CompartmentItemSource::Owned,
                });
            }
        }
    }

    // ── Metadata compartment (always) ──
    // MetadataUsage is excluded from `owned` by is_bdd_relevant(), so this
    // doesn't duplicate with render_collapsed_children(). Runs for both
    // expanded and collapsed nodes.
    for child in super::container::ordered_children(graph, &element.id) {
        if child.kind == ElementKind::MetadataUsage {
            let (type_name, pairs) = super::container::metadata_lines(child, graph);
            node.children.push(DiagramChild::Text {
                compartment: CompartmentKind::Metadata,
                text: format!("@{}", type_name),
                element_id: format!("{}/header", child.id),
                source: CompartmentItemSource::Owned,
            });
            for (i, pair) in pairs.iter().enumerate() {
                node.children.push(DiagramChild::Text {
                    compartment: CompartmentKind::Metadata,
                    text: pair.clone(),
                    element_id: format!("{}/meta/{}", child.id, i),
                    source: CompartmentItemSource::Owned,
                });
            }
        }
    }

    // ── Relationship-reference compartments ──
    {
        let rel_compartments: &[(RelationshipKind, CompartmentKind)] = &[
            (RelationshipKind::Satisfy, CompartmentKind::Satisfies),
            (RelationshipKind::Verify, CompartmentKind::Verifies),
            (RelationshipKind::Include, CompartmentKind::Includes),
        ];
        for (rel_kind, comp_kind) in rel_compartments {
            let targets: Vec<_> = graph
                .outgoing(&element.id)
                .filter(|r| r.kind == *rel_kind)
                .filter_map(|r| graph.get_element(&r.target))
                .collect();
            if !targets.is_empty() {
                let children: Vec<_> = targets
                    .iter()
                    .enumerate()
                    .map(|(i, t)| DiagramChild::Text {
                        compartment: *comp_kind,
                        text: t.name.as_deref().unwrap_or("unnamed").to_owned(),
                        element_id: format!(
                            "{}/{}/{}",
                            id,
                            comp_kind.type_string().replace(':', "_"),
                            i
                        ),
                        source: CompartmentItemSource::Owned,
                    })
                    .collect();
                node.children.push(DiagramChild::Compartment {
                    kind: *comp_kind,
                    children,
                });
            }
        }
    }

    // ── PerformedBy compartment ──
    {
        let parent_gk = VisualKind::from_element_kind(kind);
        if parent_gk
            .allowed_compartments()
            .contains(&CompartmentKind::PerformedBy)
        {
            let performers: Vec<_> = graph
                .incoming(&element.id)
                .filter(|r| r.kind == RelationshipKind::Perform)
                .filter_map(|r| graph.get_element(&r.source))
                .collect();
            if !performers.is_empty() {
                let children: Vec<_> = performers
                    .iter()
                    .enumerate()
                    .map(|(i, p)| DiagramChild::Text {
                        compartment: CompartmentKind::PerformedBy,
                        text: p.name.as_deref().unwrap_or("unnamed").to_owned(),
                        element_id: format!("{}/performedBy/{}", id, i),
                        source: CompartmentItemSource::Owned,
                    })
                    .collect();
                node.children.push(DiagramChild::Compartment {
                    kind: CompartmentKind::PerformedBy,
                    children,
                });
            }
        }
    }

    // ── Successions compartment ──
    {
        let parent_gk = VisualKind::from_element_kind(kind);
        if parent_gk
            .allowed_compartments()
            .contains(&CompartmentKind::Successions)
        {
            let successions: Vec<_> = graph
                .outgoing(&element.id)
                .filter(|r| r.kind == RelationshipKind::Succession)
                // Implicit `stateSequencing` ordering between exclusive substates
                // (States.sysml:71-77) is a semantic non-overlap invariant, not a
                // user-declared edge — never render it.
                .filter(|r| {
                    !r.props
                        .get("stateSequencing")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .collect();
            if !successions.is_empty() {
                let children: Vec<_> = successions
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let source_name = graph
                            .get_element(&r.source)
                            .and_then(|e| e.name.as_deref())
                            .unwrap_or("?");
                        let target_name = graph
                            .get_element(&r.target)
                            .and_then(|e| e.name.as_deref())
                            .unwrap_or("?");
                        DiagramChild::Text {
                            compartment: CompartmentKind::Successions,
                            text: format!("{} \u{2192} {}", source_name, target_name),
                            element_id: format!("{}/successions/{}", id, i),
                            source: CompartmentItemSource::Owned,
                        }
                    })
                    .collect();
                node.children.push(DiagramChild::Compartment {
                    kind: CompartmentKind::Successions,
                    children,
                });
            }
        }
    }

    // ── Relationships compartment ──
    {
        let parent_gk = VisualKind::from_element_kind(kind);
        if parent_gk
            .allowed_compartments()
            .contains(&CompartmentKind::Relationships)
        {
            let rels: Vec<_> = graph
                .outgoing(&element.id)
                .filter(|r| r.kind != RelationshipKind::Owning)
                .collect();
            if !rels.is_empty() {
                let children: Vec<_> = rels
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let target_name = graph
                            .get_element(&r.target)
                            .and_then(|e| e.name.as_deref())
                            .unwrap_or("?");
                        DiagramChild::Text {
                            compartment: CompartmentKind::Relationships,
                            text: format!("{:?}: {}", r.kind, target_name),
                            element_id: format!("{}/relationships/{}", id, i),
                            source: CompartmentItemSource::Owned,
                        }
                    })
                    .collect();
                node.children.push(DiagramChild::Compartment {
                    kind: CompartmentKind::Relationships,
                    children,
                });
            }
        }
    }

    // ── Enumeration literals compartment (always text, not affected by expand) ──
    if matches!(
        kind,
        ElementKind::EnumerationDefinition | ElementKind::EnumerationUsage
    ) {
        let literals: Vec<_> = super::container::ordered_children(graph, &element.id)
            .into_iter()
            .filter(|c| matches!(c.kind, ElementKind::EnumerationUsage))
            .collect();
        if !literals.is_empty() {
            let children: Vec<_> = literals
                .iter()
                .enumerate()
                .map(|(i, lit)| DiagramChild::Text {
                    compartment: CompartmentKind::Enums,
                    text: lit.name.as_deref().unwrap_or("unnamed").to_owned(),
                    element_id: format!("{}/literals/{}", id, i),
                    source: CompartmentItemSource::Owned,
                })
                .collect();
            node.children.push(DiagramChild::Compartment {
                kind: CompartmentKind::Enums,
                children,
            });
        }
    }

    // ── Child elements: render as text labels (collapsed) or nested nodes (expanded) ──
    // C13: iterate members in source declaration order (ordered_children), so
    // compartment rows and nested nodes match the textual model deterministically.
    let owned: Vec<_> = super::container::ordered_children(graph, &element.id)
        .into_iter()
        .filter(|child| !classify::is_membership_kind(&child.kind))
        .filter(|child| !classify::is_import_kind(&child.kind))
        .filter(|child| classify::is_bdd_relevant(child))
        // For requirement elements, the subject / assume-require constraint /
        // nested-requirement children are already rendered by
        // `apply_requirement_compartments` above — exclude them here so they
        // aren't double-emitted via the generic owned-children path.
        .filter(|child| {
            !(classify::is_requirement_kind(kind)
                && super::container::is_requirement_compartment_child(child))
        })
        .collect();

    // Sub-diagram embedding detection
    let is_behavioral_state_def = matches!(kind, ElementKind::StateDefinition);
    let is_behavioral_action_def = matches!(kind, ElementKind::ActionDefinition);
    let is_behavioral_def = is_behavioral_state_def || is_behavioral_action_def;

    let is_structural_container = matches!(
        VisualKind::from_element_kind(kind),
        VisualKind::Part
            | VisualKind::Item
            | VisualKind::Connection
            | VisualKind::Occurrence
    );
    let has_embeddable_states = !is_behavioral_def
        && is_structural_container
        && owned.iter().any(|c| {
            classify::is_state_kind(&c.kind) && !matches!(c.kind, ElementKind::TransitionUsage)
        });
    let has_embeddable_actions = !is_behavioral_def
        && is_structural_container
        && owned.iter().any(|c| classify::is_action_kind(&c.kind));
    let has_embeddable_flows = !is_behavioral_def
        && is_structural_container
        && owned.iter().any(|c| {
            matches!(
                c.kind,
                ElementKind::FlowUsage | ElementKind::SuccessionFlowUsage
            )
        });
    let has_embeddable_interconnection = !is_behavioral_def
        && is_structural_container
        && owned
            .iter()
            .any(|c| matches!(c.kind, ElementKind::ConnectionUsage));

    // Track whether a sub-diagram was embedded
    let mut has_embedded_subdiagram = false;

    if is_expanded && is_behavioral_def {
        // Behavioral definitions get specialized sub-diagram rendering.
        // General view does NOT render graphical ports — those belong in
        // the IBD (Interconnection) view. Ports are shown as text compartment
        // entries in the collapsed state instead.

        if is_behavioral_state_def {
            if let Some(subtree_ir) = try_generate_for_owner(ViewType::StateTransition, graph, expanded_ids, &element.id) {
                has_embedded_subdiagram = true;
                node.children.push(DiagramChild::Island {
                    view_type: ViewType::StateTransition,
                    display_name: "stv".to_owned(),
                    subtree: subtree_ir,
                    expanded: true,
                });
            }
        } else if let Some(subtree_ir) = try_generate_for_owner(ViewType::ActionFlow, graph, expanded_ids, &element.id) {
            has_embedded_subdiagram = true;
            node.children.push(DiagramChild::Island {
                view_type: ViewType::ActionFlow,
                display_name: "afv".to_owned(),
                subtree: subtree_ir,
                expanded: true,
            });
        }
    } else if is_expanded
        && (has_embeddable_states
            || has_embeddable_actions
            || has_embeddable_flows
            || has_embeddable_interconnection)
    {
        // Structural container with behavioral/structural children:
        // additively embed ALL applicable sub-diagrams.
        // (No graphical ports in general view — see note above.)

        let mut consumed_state = false;
        let mut consumed_action = false;
        let mut consumed_flow = false;
        let mut consumed_connection = false;
        let mut badge_parts = Vec::new();

        if has_embeddable_states {
            if let Some(subtree_ir) = try_generate_for_owner(ViewType::StateTransition, graph, expanded_ids, &element.id) {
                badge_parts.push("stv");
                node.children.push(DiagramChild::Island {
                    view_type: ViewType::StateTransition,
                    display_name: "stv".to_owned(),
                    subtree: subtree_ir,
                    expanded: true,
                });
            }
            consumed_state = true;
        }

        if has_embeddable_actions {
            if let Some(subtree_ir) = try_generate_for_owner(ViewType::ActionFlow, graph, expanded_ids, &element.id) {
                badge_parts.push("afv");
                node.children.push(DiagramChild::Island {
                    view_type: ViewType::ActionFlow,
                    display_name: "afv".to_owned(),
                    subtree: subtree_ir,
                    expanded: true,
                });
            }
            consumed_action = true;
        }

        if has_embeddable_flows {
            if let Some(subtree_ir) = try_generate_for_owner(ViewType::Sequence, graph, expanded_ids, &element.id) {
                badge_parts.push("sv");
                node.children.push(DiagramChild::Island {
                    view_type: ViewType::Sequence,
                    display_name: "sv".to_owned(),
                    subtree: subtree_ir,
                    expanded: true,
                });
            }
            consumed_flow = true;
        }

        if has_embeddable_interconnection {
            if let Some(subtree_ir) = try_generate_for_owner(ViewType::Interconnection, graph, expanded_ids, &element.id) {
                badge_parts.push("iv");
                node.children.push(DiagramChild::Island {
                    view_type: ViewType::Interconnection,
                    display_name: "iv".to_owned(),
                    subtree: subtree_ir,
                    expanded: true,
                });
            }
            consumed_connection = true;
        }

        if !badge_parts.is_empty() {
            has_embedded_subdiagram = true;
        }

        // Render remaining children as BDD nodes (excluding ports + consumed kinds)
        let row_parent_gk = VisualKind::from_element_kind(kind);
        for child in &owned {
            if classify::is_port_kind(&child.kind) {
                continue;
            }
            // C11/C12a: value features and doc/comment children render as
            // compartment text rows here too, not as free boxes beside the
            // embedded sub-diagrams.
            if matches!(
                child.kind,
                ElementKind::Comment | ElementKind::Documentation
            ) || super::container::is_value_feature(child, graph)
            {
                super::container::render_child_text_row(graph, &row_parent_gk, child, &mut node);
                continue;
            }
            if consumed_state && classify::is_state_kind(&child.kind) {
                continue;
            }
            if consumed_action && classify::is_action_kind(&child.kind) {
                continue;
            }
            if consumed_flow
                && matches!(
                    child.kind,
                    ElementKind::FlowUsage | ElementKind::SuccessionFlowUsage
                )
            {
                continue;
            }
            if consumed_connection && matches!(child.kind, ElementKind::ConnectionUsage) {
                continue;
            }
            if consumed_connection
                && child.kind.is_usage()
                && classify::is_part_kind(&child.kind)
            {
                continue;
            }
            let child_node = generate_node(graph, child, expanded_ids);
            node.children.push(DiagramChild::Node(child_node));
        }
    } else if !owned.is_empty() {
        let _parent_gk = VisualKind::from_element_kind(kind);

        // Root-level packages (owner == None) promote children to top-level
        // BDD nodes, so skip rendering them as compartment text here to
        // avoid double-rendering.
        let is_root_package = matches!(
            kind,
            ElementKind::Package | ElementKind::LibraryPackage
        ) && element.owner.is_none();

        if is_expanded {
            // EXPANDED: non-port children as nested nodes (value features and
            // doc/comment children stay compartment text rows — C11/C12a)
            super::container::render_expanded_children(
                graph, kind, &owned, expanded_ids, &mut node, generate_node,
            );
        } else if !is_root_package {
            // COLLAPSED: text labels in typed compartments + ports
            for child in &owned {
                if classify::is_port_kind(&child.kind) {
                    node.ports.push(make_port_ir(child));
                }
            }
            super::container::render_collapsed_children(
                graph, kind, &owned, &mut node,
            );
        }
    }

    // Add expand/collapse controls (button + expanded state + layout mode)
    let expandable = has_expandable_children(graph, element);
    super::container::apply_expand_controls(
        &mut node, expandable, !owned.is_empty(), is_expanded,
    );
    // Override: sub-diagram embedding forces expanded=true regardless
    if has_embedded_subdiagram && !expandable {
        node.expanded = Some(true);
    }

    node
}

/// Add ports from owned children to a node.
fn add_ports_from_owned(owned: &[&Element], node: &mut DiagramNode) {
    for child in owned {
        if classify::is_port_kind(&child.kind) {
            node.ports.push(make_port_ir(child));
        }
    }
}

/// Convert an element to a DiagramPort.
fn make_port_ir(element: &Element) -> DiagramPort {
    let name = element.name.as_deref().unwrap_or("port").to_owned();
    let mut port = DiagramPort::new(element.id.to_string(), name);

    // Direction from element properties
    if let Some(dir_val) = element.get_prop("direction") {
        let s = dir_val.to_string();
        if s.contains("in") && s.contains("out") {
            port = port.with_direction(PortDirection::InOut);
        } else if s.contains("in") {
            port = port.with_direction(PortDirection::In);
        } else if s.contains("out") {
            port = port.with_direction(PortDirection::Out);
        }
    }

    port
}

/// Build an empty island IR for a sub-diagram view type.
/// The actual subtree content is rendered by the old generators via
/// the bridge in the parent. This placeholder IR will be enriched
/// when the sub-generators are migrated to ViewGenerator trait.
/// Try to generate a subtree for embedding using the IR ViewGenerator registry.
///
/// Returns `Some(DiagramIR)` if the generator produced non-empty content for the owner,
/// `None` otherwise. This replaces the old bridge calls to `smodel::xxx::generate_subtree_for_owner`.
fn try_generate_for_owner(
    view: ViewType,
    graph: &ModelGraph,
    expanded_ids: &HashSet<String>,
    owner_id: &sysml_core::ElementId,
) -> Option<DiagramIR> {
    let gen = crate::ir::get_generator(view);
    let ctx = GeneratorContext::new(graph, expanded_ids);
    let ir = gen.generate_for_owner(&ctx, &owner_id.to_string())?;
    if ir.nodes.is_empty() && ir.edges.is_empty() {
        None
    } else {
        Some(ir)
    }
}

/// Render an element as textual compartment content.
/// Format an element as compartment text for collapsed container rendering.
/// Delegates to the shared `container::compartment_text_for_element`.
/// Uses Members as default compartment (keyword always included).
fn compartment_text_for_element(element: &Element, graph: &ModelGraph) -> String {
    super::container::compartment_text_for_element(
        element,
        graph,
        crate::visual_kind::CompartmentKind::Members,
    )
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sysml_core::{Element, ElementKind, ModelGraph, Relationship};

    use crate::ir::generator::GeneratorContext;
    use crate::ir::render::render;

    fn make_ctx<'a>(
        graph: &'a ModelGraph,
        expanded_ids: &'a HashSet<String>,
    ) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded_ids)
    }

    fn create_test_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("TestPackage");
        let pkg_id = graph.add_element(pkg);
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);
        let req = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SafetyReq")
            .with_owner(pkg_id);
        let req_id = graph.add_element(req);
        let satisfy = Relationship::new(RelationshipKind::Satisfy, part_id, req_id);
        graph.add_relationship(satisfy);
        graph
    }

    // ── Basic view tests ──

    #[test]
    fn general_ir_view_type() {
        let gen = GeneralViewGenerator;
        assert_eq!(gen.view_type(), ViewType::General);
        assert_eq!(gen.elk_algorithm(), "layered");
        assert_eq!(gen.elk_direction(), Some("DOWN"));
    }

    #[test]
    fn general_ir_produces_valid_json() {
        let graph = create_test_graph();
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let gen = GeneralViewGenerator;
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string_pretty(&sgraph).unwrap();
        assert!(json.contains("\"type\": \"graph\""));
        assert!(json.contains("TestPackage"));
    }

    #[test]
    fn general_ir_contains_elements() {
        let graph = create_test_graph();
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let gen = GeneralViewGenerator;
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("Engine"));
        assert!(json.contains("SafetyReq"));
    }

    // ── Requirement notation parity (Phase 1) ──

    /// Recursively collect every `DiagramChild` in a node tree, flattening
    /// compartments and nested nodes, for assertion convenience.
    fn flatten_children(node: &DiagramNode, out: &mut Vec<crate::ir::types::DiagramChild>) {
        use crate::ir::types::DiagramChild;
        for child in &node.children {
            out.push(child.clone());
            match child {
                DiagramChild::Compartment { children, .. } => {
                    for c in children {
                        out.push(c.clone());
                        if let DiagramChild::Node(n) = c {
                            flatten_children(n, out);
                        }
                    }
                }
                DiagramChild::Node(n) => flatten_children(n, out),
                _ => {}
            }
        }
    }

    #[test]
    fn general_view_renders_requirement_compartments() {
        use crate::ir::types::DiagramChild;
        use crate::visual_kind::CompartmentKind;

        let mut graph = ModelGraph::new();
        let mut req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("SafetyReq");
        req.set_prop("reqId", "R-1");
        let req_id = graph.add_element(req);

        // subject (ReferenceUsage named "subject")
        let subject = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("subject")
            .with_owner(req_id.clone());
        graph.add_element(subject);

        // assume constraint
        let mut assume = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("assumeC")
            .with_owner(req_id.clone());
        assume.set_prop("isAssume", true);
        graph.add_element(assume);

        // require constraint
        let require = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("requireC")
            .with_owner(req_id.clone());
        graph.add_element(require);

        // nested requirement
        let nested = Element::new_with_kind(ElementKind::RequirementUsage)
            .with_name("SubReq")
            .with_owner(req_id.clone());
        graph.add_element(nested);

        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = GeneralViewGenerator.generate(&ctx);

        let req_node = ir
            .nodes
            .iter()
            .find(|n| n.name == "SafetyReq")
            .expect("requirement node should be top-level");

        let mut flat = Vec::new();
        flatten_children(req_node, &mut flat);

        // reqId text in General compartment with exact element_id suffix.
        assert!(
            flat.iter().any(|c| matches!(
                c,
                DiagramChild::Text { compartment: CompartmentKind::General, text, element_id, .. }
                    if text == "id = R-1" && element_id == &format!("{}/reqId", req_id)
            )),
            "reqId text row missing"
        );
        // subject compartment
        assert!(
            flat.iter().any(|c| matches!(
                c,
                DiagramChild::Text { compartment: CompartmentKind::Subject, text, .. }
                    if text == "subject subject"
            )),
            "subject compartment missing"
        );
        // assume compartment
        assert!(
            flat.iter().any(|c| matches!(
                c,
                DiagramChild::Compartment { kind: CompartmentKind::AssumeConstraints, .. }
            )),
            "assume constraints compartment missing"
        );
        // require compartment
        assert!(
            flat.iter().any(|c| matches!(
                c,
                DiagramChild::Compartment { kind: CompartmentKind::RequireConstraints, .. }
            )),
            "require constraints compartment missing"
        );
        // nested requirement rendered as a node inside the Requirements compartment
        assert!(
            flat.iter().any(|c| matches!(
                c,
                DiagramChild::Node(n) if n.name == "SubReq"
            )),
            "nested requirement node missing"
        );
    }

    #[test]
    fn general_view_satisfy_edge_has_guillemet_label() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Engine");
        let part_id = graph.add_element(part);
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req");
        let req_id = graph.add_element(req);
        graph.add_relationship(Relationship::new(RelationshipKind::Satisfy, part_id, req_id));

        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = GeneralViewGenerator.generate(&ctx);

        let satisfy_edge = ir
            .edges
            .iter()
            .find(|e| matches!(e.kind, crate::ir::types::DiagramEdgeKind::Relationship(RelationshipKind::Satisfy)))
            .expect("satisfy edge should be present");
        assert_eq!(satisfy_edge.label, "\u{00ab}satisfy\u{00bb}");
    }

    #[test]
    fn general_ir_empty_graph() {
        let graph = ModelGraph::new();
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let gen = GeneralViewGenerator;
        let ir = gen.generate(&ctx);

        assert_eq!(ir.view_type, ViewType::General);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());

        let sgraph = render(&ir);
        assert_eq!(sgraph.type_, "graph");
        assert!(sgraph.children.is_empty());
    }

    // ── Edge tests ──

    #[test]
    fn general_ir_contains_edges_when_expanded() {
        let graph = create_test_graph();
        let pkg_id = graph
            .elements
            .values()
            .find(|e| e.kind == ElementKind::Package)
            .unwrap()
            .id
            .to_string();
        let mut expanded = HashSet::new();
        expanded.insert(pkg_id);

        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("edge:satisfy"));
    }

    #[test]
    fn collapsed_children_suppress_dangling_edges() {
        let mut graph = ModelGraph::new();
        let parent = Element::new_with_kind(ElementKind::PartDefinition).with_name("Parent");
        let parent_id = graph.add_element(parent);
        let part_a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(parent_id.clone());
        let a_id = graph.add_element(part_a);
        let part_b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(parent_id);
        let b_id = graph.add_element(part_b);

        let dep = Relationship::new(RelationshipKind::Dependency, a_id, b_id);
        graph.add_relationship(dep);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            !json.contains("edge:dependency"),
            "Edge between collapsed children should be suppressed"
        );
    }

    #[test]
    fn no_expose_view_excludes_library_content() {
        // Tracker 3.10: a General view with no Expose clause must not dump the
        // standard library as top-level nodes (nor leak its internal edges).
        let mut graph = ModelGraph::new();

        // User content.
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("UserPkg");
        let pkg_id = graph.add_element(pkg);
        let engine = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id);
        graph.add_element(engine);

        // Library content: a library package with an internal element + an
        // internal relationship. None of this should reach the scene.
        let lib_pkg = Element::new_with_kind(ElementKind::LibraryPackage).with_name("StdLib");
        let lib_pkg_id = graph.add_library_package(lib_pkg);
        let lib_a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("LibPartA")
            .with_owner(lib_pkg_id.clone());
        let lib_a_id = graph.add_element(lib_a);
        let lib_b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("LibPartB")
            .with_owner(lib_pkg_id);
        let lib_b_id = graph.add_element(lib_b);
        graph.add_relationship(Relationship::new(
            RelationshipKind::Dependency,
            lib_a_id,
            lib_b_id,
        ));

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(json.contains("Engine"), "user content must render");
        assert!(!json.contains("StdLib"), "library package must be excluded");
        assert!(!json.contains("LibPart"), "library members must be excluded");
        assert!(
            !json.contains("edge:dependency"),
            "library-internal edges must prune once library nodes are excluded"
        );
    }

    #[test]
    fn explicitly_exposed_library_element_still_renders() {
        // The library exclusion is gated on `expose.is_none()`: an author who
        // explicitly exposes a library element must still get it as the canvas
        // subject.
        let mut graph = ModelGraph::new();
        let lib_pkg = Element::new_with_kind(ElementKind::LibraryPackage).with_name("StdLib");
        let lib_pkg_id = graph.add_library_package(lib_pkg);
        let lib_part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("ExposedLibPart")
            .with_owner(lib_pkg_id);
        let lib_part_id = graph.add_element(lib_part);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded).with_expose(&lib_part_id);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            json.contains("ExposedLibPart"),
            "an explicitly exposed library element must render as the canvas subject"
        );
    }

    #[test]
    fn find_rendered_ancestor_skips_namespace_containers() {
        // Pkg ▸ Part ▸ inner.  Both Pkg and Part are "rendered".
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Part")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);
        let inner = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("inner")
            .with_owner(part_id.clone());
        let inner_id = graph.add_element(inner);
        // A part owned directly by the package (no real-container ancestor).
        let loose = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("loose")
            .with_owner(pkg_id.clone());
        let loose_id = graph.add_element(loose);

        // inner lifts to Part (a real container), NOT up to the Package.
        let mut rendered = HashSet::new();
        rendered.insert(pkg_id.to_string());
        rendered.insert(part_id.to_string());
        assert_eq!(
            find_rendered_ancestor(&graph, &inner_id, &rendered),
            Some(part_id.to_string()),
        );

        // When the ONLY rendered ancestor is a Package, there is no real container
        // to lift to → None (3.15: the edge is pruned, not rerouted to the package
        // box, which would manufacture a Package→Package edge).
        let only_pkg: HashSet<String> = [pkg_id.to_string()].into_iter().collect();
        assert_eq!(find_rendered_ancestor(&graph, &loose_id, &only_pkg), None);
        assert_eq!(find_rendered_ancestor(&graph, &inner_id, &only_pkg), None);
    }

    #[test]
    fn expanded_children_keep_edges() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part_a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(pkg_id.clone());
        let a_id = graph.add_element(part_a);
        let part_b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(pkg_id.clone());
        let b_id = graph.add_element(part_b);

        let dep = Relationship::new(RelationshipKind::Dependency, a_id, b_id);
        graph.add_relationship(dep);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            json.contains("edge:dependency"),
            "Edge between expanded children should be present"
        );
    }

    #[test]
    fn collapsed_owner_suppresses_port_endpoint_edges() {
        let mut graph = ModelGraph::new();
        let parent = Element::new_with_kind(ElementKind::PartDefinition).with_name("Parent");
        let parent_id = graph.add_element(parent);

        let a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(parent_id.clone());
        let a_id = graph.add_element(a);
        let b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(parent_id);
        let b_id = graph.add_element(b);

        let a_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("aOut")
            .with_owner(a_id.clone());
        let a_port_id = graph.add_element(a_port);
        let b_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("bIn")
            .with_owner(b_id.clone());
        let b_port_id = graph.add_element(b_port);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Dependency,
            a_port_id,
            b_port_id,
        ));

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            !json.contains("edge:dependency"),
            "Edges targeting ports of collapsed owners must be suppressed"
        );
    }

    #[test]
    fn expanded_owner_keeps_port_endpoint_edges() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(pkg_id.clone());
        let a_id = graph.add_element(a);
        let b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(pkg_id.clone());
        let b_id = graph.add_element(b);

        let a_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("aOut")
            .with_owner(a_id.clone());
        let a_port_id = graph.add_element(a_port);
        let b_port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("bIn")
            .with_owner(b_id.clone());
        let b_port_id = graph.add_element(b_port);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Dependency,
            a_port_id,
            b_port_id,
        ));

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            json.contains("edge:dependency"),
            "Edges between ports should be kept when owners are rendered"
        );
    }

    // ── Compartment tests ──

    #[test]
    fn collapsed_children_render_as_text() {
        // Root-level packages promote children to top-level BDD nodes.
        // Use a nested package to test compartment-text rendering.
        let mut graph = ModelGraph::new();
        let outer = Element::new_with_kind(ElementKind::Package).with_name("Outer");
        let outer_id = graph.add_element(outer);
        let inner = Element::new_with_kind(ElementKind::Package)
            .with_name("Inner")
            .with_owner(outer_id.clone());
        let inner_id = graph.add_element(inner);
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(inner_id);
        graph.add_element(part);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        // Engine should appear as compartment text inside the nested Inner package
        assert!(json.contains("part Engine"), "Nested package children should render as compartment text");
        assert!(json.contains("compartment-text"));
    }

    #[test]
    fn root_package_promotes_children_to_top_level() {
        let graph = create_test_graph();
        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        // Root-level package children should be top-level nodes, not compartment text
        assert!(json.contains("node:block"), "Children of root package should be top-level nodes");
        assert!(json.contains("Engine"));
    }

    #[test]
    fn expanded_node_shows_children_as_nodes() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id.clone());
        graph.add_element(part);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("node:block"));
    }

    // ── Expand button tests ──

    #[test]
    fn expandable_node_has_button() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Vehicle")
            .with_owner(pkg_id);
        graph.add_element(part_def);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("button:expand"));
        // Root-level packages auto-expand, so the package should be expanded
        assert!(json.contains("\"expanded\":true"));
    }

    #[test]
    fn leaf_children_get_expand_button() {
        // Any node with BDD-relevant children should be expandable,
        // even if those children are "leaf" usages with no grandchildren.
        // When expanded, children render as nested DiagramChild::Node entries
        // rather than being absorbed into compartment text.
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Wheel")
            .with_owner(pkg_id);
        graph.add_element(part);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            json.contains("button:expand"),
            "Nodes with BDD-relevant children should get expand button"
        );
    }

    // ── Sub-diagram embedding tests ──

    /// Recursively search DiagramIR for a node with a given name.
    fn find_node_recursive<'a>(nodes: &'a [DiagramNode], name: &str) -> Option<&'a DiagramNode> {
        for node in nodes {
            if node.name == name {
                return Some(node);
            }
            for child in &node.children {
                if let DiagramChild::Node(child_node) = child {
                    if let Some(found) = find_node_recursive(std::slice::from_ref(child_node), name) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    #[test]
    fn expanded_state_def_embeds_island() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let state_def = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("MachineSM")
            .with_owner(pkg_id.clone());
        let sd_id = graph.add_element(state_def);
        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Idle")
            .with_owner(sd_id.clone());
        graph.add_element(s1);
        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Running")
            .with_owner(sd_id.clone());
        graph.add_element(s2);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        expanded.insert(sd_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // The state def is a child of the expanded Pkg, so search recursively
        let machine_node = find_node_recursive(&ir.nodes, "MachineSM");
        assert!(machine_node.is_some(), "MachineSM should exist as a node");
        let machine_node = machine_node.unwrap();

        // Should have at least one island child
        let has_island = machine_node.children.iter().any(|c| {
            matches!(c, DiagramChild::Island { view_type, .. } if *view_type == ViewType::StateTransition)
        });
        assert!(
            has_island,
            "Expanded StateDefinition should have a StateTransition island"
        );
    }

    #[test]
    fn expanded_action_def_embeds_island() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let action_def = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("BrewCoffee")
            .with_owner(pkg_id.clone());
        let ad_id = graph.add_element(action_def);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        expanded.insert(ad_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // BrewCoffee is a child of the expanded Pkg, search recursively
        let brew_node = find_node_recursive(&ir.nodes, "BrewCoffee");
        assert!(brew_node.is_some(), "ActionDefinition should be a node");
    }

    #[test]
    fn expanded_part_def_still_uses_bdd_style() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);
        let part_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Vehicle")
            .with_owner(pkg_id.clone());
        let pd_id = graph.add_element(part_def);
        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pd_id.clone());
        graph.add_element(child);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        expanded.insert(pd_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(
            json.contains("node:block"),
            "PartDefinition should use BDD-style nested children"
        );
    }

    // ── Metadata stereotype tests ──

    #[test]
    fn metadata_typed_element_uses_metadata_stereotype() {
        let mut graph = ModelGraph::new();

        let meta_def =
            Element::new_with_kind(ElementKind::MetadataDefinition).with_name("safety");
        let meta_id = graph.add_element(meta_def);

        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("BrakePad");
        let part_id = graph.add_element(part);

        let type_rel = Relationship::new(RelationshipKind::TypeOf, part_id, meta_id);
        graph.add_relationship(type_rel);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // Find the BrakePad node
        let brake_node = ir.nodes.iter().find(|n| n.name == "BrakePad");
        assert!(brake_node.is_some(), "Should have BrakePad node");
        let brake_node = brake_node.unwrap();
        assert_eq!(
            brake_node.stereotype,
            "\u{00ab}safety\u{00bb}",
            "Should use metadata name as stereotype"
        );
    }

    #[test]
    fn non_metadata_typed_element_uses_default_stereotype() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        graph.add_element(part);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        let vehicle_node = ir.nodes.iter().find(|n| n.name == "Vehicle");
        assert!(vehicle_node.is_some());
        let vehicle_node = vehicle_node.unwrap();
        // Declaration-text keyword (contract A.3/A.4, D-N1): «part def»,
        // never the expanded metaclass name «part definition».
        assert!(
            vehicle_node.stereotype.contains("part def")
                && !vehicle_node.stereotype.contains("part definition"),
            "Should use the declaration-text stereotype, got {:?}",
            vehicle_node.stereotype
        );
    }

    // ── Documentation compartment tests ──

    #[test]
    fn textual_representation_child_appears_in_documentation() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let part_id = graph.add_element(part);

        let tr = Element::new_with_kind(ElementKind::TextualRepresentation)
            .with_owner(part_id)
            .with_prop("body", "This is the SysML textual form");
        graph.add_element(tr);

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(
            json.contains("comp:documentation"),
            "Should have documentation compartment"
        );
        assert!(
            json.contains("This is the SysML textual form"),
            "TextualRepresentation body should appear in documentation"
        );
    }

    // ── N-ary connection tests ──

    #[test]
    fn nary_connection_generates_dot_node() {
        use sysml_core::Value;

        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("link")
            .with_owner(pkg_id.clone());
        let conn_id = graph.add_element(conn);

        // Create 3 end features (> 2 = n-ary)
        let e1 = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("end1")
            .with_owner(conn_id.clone())
            .with_prop("isEnd", Value::Bool(true));
        graph.add_element(e1);
        let e2 = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("end2")
            .with_owner(conn_id.clone())
            .with_prop("isEnd", Value::Bool(true));
        graph.add_element(e2);
        let e3 = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("end3")
            .with_owner(conn_id.clone())
            .with_prop("isEnd", Value::Bool(true));
        graph.add_element(e3);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // Check for nary-dot node
        let has_dot = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::NaryDot));
        assert!(has_dot, "N-ary connection should generate nary-dot node");

        // Check for nary-segment edges
        let seg_count = ir
            .edges
            .iter()
            .filter(|e| e.tags.contains(&EdgeTag::NarySegment))
            .count();
        assert_eq!(seg_count, 3, "Should have 3 nary-segment edges");
    }

    #[test]
    fn binary_connection_no_nary_dot() {
        use sysml_core::Value;

        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let conn = Element::new_with_kind(ElementKind::ConnectionUsage)
            .with_name("link")
            .with_owner(pkg_id.clone());
        let conn_id = graph.add_element(conn);

        let e1 = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("end1")
            .with_owner(conn_id.clone())
            .with_prop("isEnd", Value::Bool(true));
        graph.add_element(e1);
        let e2 = Element::new_with_kind(ElementKind::ReferenceUsage)
            .with_name("end2")
            .with_owner(conn_id.clone())
            .with_prop("isEnd", Value::Bool(true));
        graph.add_element(e2);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        let has_dot = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::NaryDot));
        assert!(
            !has_dot,
            "Binary connection should NOT generate nary-dot node"
        );
    }

    // ── Perform relationship tests ──

    #[test]
    fn action_with_perform_has_performed_by_compartment() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let action = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Drive")
            .with_owner(pkg_id.clone());
        let action_id = graph.add_element(action);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Driver")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let perform = Relationship::new(RelationshipKind::Perform, part_id, action_id);
        graph.add_relationship(perform);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(
            json.contains("comp:performedBy"),
            "ActionUsage with incoming Perform relationship should have performedBy compartment"
        );
        assert!(
            json.contains("Driver"),
            "performedBy compartment should list the performer name"
        );
    }

    #[test]
    fn action_without_perform_has_no_performed_by_compartment() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let action = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Drive")
            .with_owner(pkg_id.clone());
        graph.add_element(action);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(
            !json.contains("comp:performedBy"),
            "ActionUsage without Perform relationships should not have performedBy compartment"
        );
    }

    // ── Structural container embedding tests ──

    #[test]
    fn expanded_part_def_with_state_children_embeds_island() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let part_def = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Machine")
            .with_owner(pkg_id.clone());
        let pd_id = graph.add_element(part_def);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Idle")
            .with_owner(pd_id.clone());
        graph.add_element(s1);
        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Running")
            .with_owner(pd_id.clone());
        graph.add_element(s2);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        expanded.insert(pd_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // Machine is a child of the expanded Pkg, search recursively
        let machine_node = find_node_recursive(&ir.nodes, "Machine");
        assert!(machine_node.is_some(), "Should have Machine node");
    }

    #[test]
    fn expanded_package_with_actions_uses_bdd_not_embed() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let a1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("DoStuff")
            .with_owner(pkg_id.clone());
        graph.add_element(a1);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        // Package should NOT embed sub-diagrams, only structural types do
        assert!(
            json.contains("node:action"),
            "ActionUsage child should be rendered as BDD node"
        );
    }

    // ── Self-loop suppression test ──

    #[test]
    fn self_loop_on_collapsed_container_is_suppressed() {
        let mut graph = ModelGraph::new();
        let parent = Element::new_with_kind(ElementKind::PartDefinition).with_name("Parent");
        let parent_id = graph.add_element(parent);

        let a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(parent_id.clone());
        let a_id = graph.add_element(a);
        let b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(parent_id.clone());
        let b_id = graph.add_element(b);

        // Both A and B reroute to Parent when collapsed
        graph.add_relationship(Relationship::new(
            RelationshipKind::Dependency,
            a_id,
            b_id,
        ));

        let gen = GeneralViewGenerator;
        let expanded = HashSet::new();
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // With no expanded IDs, both A and B are text labels inside Parent.
        // The edge would reroute to Parent→Parent (self-loop) and should be suppressed.
        assert!(
            ir.edges.is_empty(),
            "Self-loop edges on collapsed containers should be suppressed"
        );
    }

    // ── Compartment text tests ──

    #[test]
    fn compartment_text_basic_format() {
        let elem = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
        let graph = ModelGraph::new();
        let text = compartment_text_for_element(&elem, &graph);
        assert_eq!(text, "part engine");
    }

    #[test]
    fn compartment_text_with_type() {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("engine")
            .with_prop("unresolved_type", "Engine");
        graph.add_element(elem.clone());
        let text = compartment_text_for_element(&elem, &graph);
        assert_eq!(text, "part engine : Engine");
    }

    #[test]
    fn compartment_text_enum_literal() {
        let owner = sysml_core::ElementId::from_string("owner");
        let elem = Element::new_with_kind(ElementKind::EnumerationUsage)
            .with_name("Red")
            .with_owner(owner);
        let graph = ModelGraph::new();
        let text = compartment_text_for_element(&elem, &graph);
        assert_eq!(text, "Red");
    }

    #[test]
    fn compartment_text_comment() {
        let elem = Element::new_with_kind(ElementKind::Comment)
            .with_name("note")
            .with_prop("body", "This is important");
        let graph = ModelGraph::new();
        let text = compartment_text_for_element(&elem, &graph);
        assert_eq!(text, "/* This is important */");
    }

    // ── generate_for_owner test ──

    #[test]
    fn generate_for_owner_returns_subtree() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let part_id = graph.add_element(part);
        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(part_id.clone());
        graph.add_element(child);

        let expanded = HashSet::new();
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate_for_owner(&ctx, &part_id.to_string());
        assert!(ir.is_some(), "generate_for_owner should return Some");
        let ir = ir.unwrap();
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "Vehicle");
    }

    #[test]
    fn generate_for_owner_nonexistent_returns_none() {
        let graph = ModelGraph::new();
        let expanded = HashSet::new();
        let gen = GeneralViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate_for_owner(&ctx, "nonexistent");
        assert!(ir.is_none());
    }
}
