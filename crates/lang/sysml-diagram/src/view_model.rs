//! ViewModel — the renderer-agnostic, serializable wire artifact (Bucket 1.4).
//!
//! The [`ViewModel`] is the top-level artifact a frontend consumes: it composes
//! the [`DiagramIR`] scene (Bucket 1.2) with the renderer-agnostic addenda that
//! later Bucket-1 tasks attach (design tokens 1.3, interaction descriptors 1.5,
//! the `ElementId↔Span` text-map 1.6, simulation overlays 1.8, frame slots §F-10).
//! It is the value the `workspace_view_model_best` salsa query caches.
//!
//! The pure builder lives here (no salsa dependency); `sysml-ide-db` wraps it in
//! a tracked query. The retired graph-renderer `legacy graph` is *derived from* the same scene via the
//! `render` adapter — there is no parallel generate path: [`to_view_model`] and
//! `to_view_model` both build the scene through [`build_scene`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sysml_core::{ElementId, ModelGraph, ViewSummary};
use sysml_runtime::expressions::ExprIR;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::design_tokens::DesignTokens;
use crate::interaction::InteractionMap;
use crate::ir::{self, DiagramIR};
use crate::ViewType;
use crate::text_map::TextMap;
use crate::ViewRequest;

/// The renderer-agnostic, salsa-cacheable view artifact.
///
/// Composition over the promoted [`DiagramIR`] scene: `scene` carries the
/// structure/geometry; the addenda fields are added by later Bucket-1 tasks
/// (each a distinct concern, not a second scene). Cheap to clone — `scene` is an
/// `Arc`, so a clone is a pointer bump.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ViewModel {
    /// The diagram scene (promoted `DiagramIR`).
    pub scene: Arc<DiagramIR>,
    /// `ElementId↔Span` text-map (Bucket 1.6) — the bidirectional text↔diagram
    /// link. A shared `Arc` into the cheaper standalone text-map query; `None`
    /// when built by the pure scene-only builder (the salsa layer attaches it).
    pub text_map: Option<Arc<TextMap>>,
    /// Design tokens (Bucket 1.3) — the color palette, the single Rust source of
    /// truth for the CSS `:root` variables. A shared, process-wide `Arc` (tokens
    /// are constant), so every `ViewModel` carries the same pointer at no cost.
    /// Palette only for now; typography + geometry join in Bucket 3.
    pub tokens: Arc<DesignTokens>,
    /// Interaction descriptors (Bucket 1.5) — renderer-agnostic semantic
    /// affordances (e.g. go-to-definition target) joined to scene regions by
    /// `ElementId`. A shared `Arc` into the cheaper standalone interaction query
    /// (view-independent, like the text-map); `None` when built by the pure
    /// scene-only builder (the salsa layer attaches it). Command/label resolution
    /// is a 1.7 service-layer overlay, not stored here.
    pub interactions: Option<Arc<InteractionMap>>,
    /// View frame (§F-10 / spec §8.2.3.26) — the framed-view metadata: the name
    /// compartment (`«view» Name : kind`) plus up to three optional info
    /// compartments. `Some` only for a **declared** `View` (resolved from a
    /// `ViewSummary` at the salsa layer); `None` for an ad-hoc projection of a
    /// plain package, which is not a framed-view in the spec sense.
    pub frame: Option<ViewFrame>,
    /// The non-graph structured model (3.12) for the Grid/Browser/Geometry view
    /// families — `Some` only for those kinds (the renderer dispatches Table/Tree/
    /// Geometry from it instead of from a graph `scene`); `None` for graph views.
    /// This makes the ViewModel the SINGLE pipeline for ALL view families — the
    /// FE no longer hits the legacy legacy graph model `/render` payload for non-graph views.
    /// `skip_deserializing`: the wire flows Rust→JSON only, and the underlying
    /// models are `Serialize`-only, so the (derived) `Deserialize` for `ViewModel`
    /// defaults this to `None` rather than forcing `Deserialize` onto every model.
    #[cfg_attr(feature = "serde", serde(default, skip_deserializing))]
    pub non_graph: Option<crate::NonGraphModel>,
    // Addenda still to land with their tasks:
    //   overlays — NOT a field: per-tick sim deltas are session state, delivered
    //   as the `sim_overlay::SimOverlay` sibling artifact (1.8, steward option A).
}

/// The diagram view frame (spec §8.2.3.26 / §F-10) — the outer framed-view.
///
/// Carries the name compartment as **typed** data (`view_kind` + `name`); the
/// renderer formats the `«view» Name : kind` keyword line (the short-code label
/// is a renderer concern, never baked into the IR). The three info compartments
/// are genuinely optional per the spec ("up to three optional info
/// compartments") — `None` means "no compartment", not "unimplemented".
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ViewFrame {
    /// View kind, for the name compartment. Typed `ViewType`, not a label string.
    pub view_kind: ViewType,
    /// The declared view's name (the name-compartment text after the keyword).
    pub name: String,
    /// The view's **immediate, literally-declared** type/supertype name (R7,
    /// spec §8.2.3.26): the heading suffix `«view» Name : Type`. This is the raw
    /// first entry of `supertype_names` — NOT canonicalized to the nearest
    /// standard view (that would smuggle semantics). `None` when the view
    /// declares no type (heading is then just `«view» Name`, never a synthesized
    /// render-word like `: General`).
    pub type_name: Option<String>,
    /// Top-right info compartment — the Expose/Filter summary (spec-derivable).
    pub top_right: Option<FrameSlot>,
    /// Bottom-left info compartment — annotations. SPEC-SILENT content; `None`
    /// until a Bucket-3 renderer signal defines it.
    pub bottom_left: Option<FrameSlot>,
    /// Bottom-right info compartment — view metadata. SPEC-SILENT content; `None`
    /// until a Bucket-3 renderer signal defines it.
    pub bottom_right: Option<FrameSlot>,
}

/// One frame info-compartment's content (renderer-agnostic text lines).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FrameSlot {
    pub text: String,
}

/// Build a [`ViewFrame`] from a declared view's [`ViewSummary`] and the resolved
/// view kind. Populates the name compartment and, when the view declares exposes
/// or filters, the top-right info compartment with their summary. The other two
/// info compartments are SPEC-SILENT and left `None` (Bucket 3).
pub fn view_frame_from_summary(
    graph: &ModelGraph,
    summary: &ViewSummary,
    view_kind: ViewType,
) -> ViewFrame {
    let name = summary
        .name
        .clone()
        .unwrap_or_else(|| "<unnamed>".to_owned());

    // R7 (§8.2.3.26): the heading suffix is the view's LITERALLY-declared
    // immediate type/supertype — `supertype_names().first()`, NOT the
    // canonicalizing `walk_supertypes_for_view_type` BFS. `view def X :>
    // StateTransition` → `: StateTransition` verbatim; no declared type → no
    // suffix (never a synthesized render-word like `: General`).
    let type_name = graph
        .get_element(&summary.id)
        .map(|el| crate::visual_kind::supertype_names(graph, el))
        .and_then(|mut names| names.drain(..).next());

    let mut lines: Vec<String> = summary
        .exposed
        .iter()
        .filter_map(|e| e.qualified_name.clone().map(|q| format!("expose {q}")))
        .collect();
    if !summary.filters.is_empty() {
        lines.push(format!("[filtered: {} criteria]", summary.filters.len()));
    }
    let top_right = (!lines.is_empty()).then(|| FrameSlot {
        text: lines.join("\n"),
    });

    ViewFrame {
        view_kind,
        name,
        type_name,
        top_right,
        bottom_left: None,
        bottom_right: None,
    }
}

impl ViewModel {
    /// Wrap a scene as a `ViewModel`. The design tokens (1.3) are always attached
    /// (a shared constant `Arc`); other addenda are attached by later tasks.
    pub fn new(scene: DiagramIR) -> Self {
        Self {
            scene: Arc::new(scene),
            text_map: None,
            tokens: DesignTokens::shared(),
            interactions: None,
            frame: None,
            non_graph: None,
        }
    }

    /// Attach the non-graph structured model (3.12) for a Grid/Browser/Geometry
    /// view. The renderer dispatches on this when present.
    pub fn with_non_graph(mut self, non_graph: crate::NonGraphModel) -> Self {
        self.non_graph = Some(non_graph);
        self
    }

    /// Borrow the non-graph model, if this is a non-graph view.
    pub fn non_graph(&self) -> Option<&crate::NonGraphModel> {
        self.non_graph.as_ref()
    }

    /// Attach the view frame (§F-10). The salsa layer calls this with the frame
    /// derived from the matching declared-view `ViewSummary`, or `None` for an
    /// ad-hoc projection.
    pub fn with_frame(mut self, frame: Option<ViewFrame>) -> Self {
        self.frame = frame;
        self
    }

    /// Borrow the view frame, if this is a framed (declared) view.
    pub fn frame(&self) -> Option<&ViewFrame> {
        self.frame.as_ref()
    }

    /// Borrow the design tokens (always present).
    pub fn tokens(&self) -> &DesignTokens {
        &self.tokens
    }

    /// Attach a shared interaction map (Bucket 1.5). The salsa layer calls this
    /// with the cached `Arc` from the standalone interaction query.
    pub fn with_interactions(mut self, interactions: Arc<InteractionMap>) -> Self {
        self.interactions = Some(interactions);
        self
    }

    /// Borrow the interaction map, if attached.
    pub fn interactions(&self) -> Option<&InteractionMap> {
        self.interactions.as_deref()
    }

    /// Attach a shared text-map (Bucket 1.6). The salsa layer calls this with the
    /// cached `Arc` from the standalone text-map query.
    pub fn with_text_map(mut self, text_map: Arc<TextMap>) -> Self {
        self.text_map = Some(text_map);
        self
    }

    /// Borrow the text-map, if attached.
    pub fn text_map(&self) -> Option<&TextMap> {
        self.text_map.as_deref()
    }

    /// Borrow the scene.
    pub fn scene(&self) -> &DiagramIR {
        &self.scene
    }

    /// Clone the scene `Arc` (cheap pointer bump).
    pub fn scene_arc(&self) -> Arc<DiagramIR> {
        Arc::clone(&self.scene)
    }

    /// Every id the scene or the non-graph payload references: node / port /
    /// compartment-item / edge-endpoint ids (recursively, islands included) plus
    /// tree-node / table row-column-cell / geometry-primitive ids. This is the
    /// join domain of the `text_map` / `interactions` sidecars for THIS view.
    pub fn referenced_element_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        collect_scene_ids(&self.scene, &mut ids);
        if let Some(non_graph) = self.non_graph.as_ref() {
            collect_non_graph_ids(non_graph, &mut ids);
        }
        ids
    }

    /// A copy of this `ViewModel` with the `text_map` / `interactions` sidecars
    /// scoped to [`Self::referenced_element_ids`].
    ///
    /// The sidecars are whole-graph maps (one salsa query per graph, shared by
    /// every view), so a serialized `ViewModel` carries a span for every element
    /// in the workspace — megabytes for a view whose scene holds a handful of
    /// nodes. Export paths (CLI `sysml export viewmodel`, fixture baking) call
    /// this before serializing; the live service keeps the unpruned shared
    /// `Arc`s, where the duplication is free.
    pub fn pruned_to_referenced(&self) -> ViewModel {
        let keep = self.referenced_element_ids();
        let mut pruned = self.clone();
        pruned.text_map = self.text_map.as_deref().map(|tm| Arc::new(tm.retained(&keep)));
        pruned.interactions = self
            .interactions
            .as_deref()
            .map(|im| Arc::new(im.retained(&keep)));
        pruned
    }

    /// Rewrite every text-map span file URI that lives under `root` to a
    /// root-relative path, so a serialized `ViewModel` (a baked fixture, a
    /// downloaded export) carries no absolute machine paths. A file outside
    /// `root` keeps its URI verbatim — an honest fallback, matching the
    /// provenance-manifest normalization in the service layer.
    pub fn with_relative_file_uris(mut self, root: &std::path::Path) -> ViewModel {
        if let Some(tm) = self.text_map.as_deref() {
            let mut rewritten = tm.clone();
            rewritten.relativize_files(root);
            self.text_map = Some(Arc::new(rewritten));
        }
        self
    }
}

fn collect_scene_ids(scene: &DiagramIR, out: &mut HashSet<String>) {
    for node in &scene.nodes {
        collect_node_ids(node, out);
    }
    for edge in &scene.edges {
        collect_edge_ids(edge, out);
    }
}

fn collect_node_ids(node: &ir::DiagramNode, out: &mut HashSet<String>) {
    out.insert(node.element_id.clone());
    for port in &node.ports {
        collect_port_ids(port, out);
    }
    for child in &node.children {
        collect_child_ids(child, out);
    }
}

fn collect_port_ids(port: &ir::DiagramPort, out: &mut HashSet<String>) {
    out.insert(port.element_id.clone());
    for sub in &port.sub_ports {
        collect_port_ids(sub, out);
    }
}

fn collect_child_ids(child: &ir::DiagramChild, out: &mut HashSet<String>) {
    match child {
        ir::DiagramChild::Node(node) => collect_node_ids(node, out),
        ir::DiagramChild::Text { element_id, .. } => {
            out.insert(element_id.clone());
        }
        ir::DiagramChild::Compartment { children, .. } => {
            for c in children {
                collect_child_ids(c, out);
            }
        }
        ir::DiagramChild::Island { subtree, .. } => collect_scene_ids(subtree, out),
        ir::DiagramChild::Edge(edge) => collect_edge_ids(edge, out),
    }
}

fn collect_edge_ids(edge: &ir::DiagramEdge, out: &mut HashSet<String>) {
    out.insert(edge.id.clone());
    out.insert(edge.source_id.clone());
    out.insert(edge.target_id.clone());
    if let Some(id) = edge.source_port_id.as_ref() {
        out.insert(id.clone());
    }
    if let Some(id) = edge.target_port_id.as_ref() {
        out.insert(id.clone());
    }
}

fn collect_non_graph_ids(non_graph: &crate::NonGraphModel, out: &mut HashSet<String>) {
    match non_graph {
        crate::NonGraphModel::Table(table) => {
            for col in &table.columns {
                out.insert(col.id.clone());
            }
            for row in &table.rows {
                out.insert(row.id.clone());
                for cell in &row.cells {
                    if let Some(id) = cell.element_id.as_ref() {
                        out.insert(id.clone());
                    }
                }
            }
        }
        crate::NonGraphModel::Geometry(geometry) => {
            for primitive in &geometry.primitives {
                match primitive {
                    crate::GeometryPrimitive::Rect { id, element_id, .. } => {
                        out.insert(id.clone());
                        if let Some(eid) = element_id.as_ref() {
                            out.insert(eid.clone());
                        }
                    }
                }
            }
        }
        crate::NonGraphModel::Tree(tree) => {
            for root in &tree.roots {
                collect_tree_node_ids(root, out);
            }
        }
    }
}

fn collect_tree_node_ids(node: &crate::TreeNode, out: &mut HashSet<String>) {
    out.insert(node.id.clone());
    if let Some(id) = node.element_id.as_ref() {
        out.insert(id.clone());
    }
    for child in &node.children {
        collect_tree_node_ids(child, out);
    }
}

/// Build the diagram scene for a request. This is the single generate path
/// shared by [`to_view_model`] and the retired graph-renderer `to_view_model` adapter — the
/// generator runs once, overlays apply once, ELK/CSS rendering is downstream.
pub(crate) fn build_scene(
    graph: &ModelGraph,
    request: &ViewRequest,
    filter_cache: Option<&HashMap<ElementId, Arc<ExprIR>>>,
) -> DiagramIR {
    let gen = ir::get_generator(request.view_type);
    let mut ctx = ir::GeneratorContext::new(graph, &request.expanded_ids);
    if let Some(cache) = filter_cache {
        ctx = ctx.with_filter_cache(cache);
    }
    if let Some(f) = request.filter.as_ref() {
        ctx = ctx.with_filter(f);
    }
    if !request.exposes.is_empty() {
        ctx = ctx.with_exposes(&request.exposes);
    }
    let mut scene = gen.generate(&ctx);
    if !request.overlays.is_empty() {
        ir::apply_overlays(&request.overlays, &mut scene, graph);
    }
    // Structural guarantee (D-B2): no edge may reference a shape the renderer
    // won't lay out — one stray endpoint is a hard elk import error that
    // blanks the whole view. Reroute to the nearest laid-out ancestor or drop.
    ir::consistency::enforce_scene_consistency(&mut scene, graph);
    // Spec-silent port-side fallback (#71): a direction-less port carries no
    // side, so elk lays it out FREE and stacks same-node ports into overlapping
    // labels (G3 clash). Deterministically place every side-less port (declared
    // direction → W/E; bare → W/E alternation) — a rendering freedom per
    // §8.2.3.12, never written back to the model. See `ir::port_placement`.
    ir::port_placement::assign_port_sides(&mut scene);
    scene
}

/// Build the scoped non-graph structured model (3.12) for a Grid/Browser/Geometry
/// request, or `None` for a graph view. Pure function of `(graph, request)`.
fn build_non_graph(graph: &ModelGraph, request: &ViewRequest) -> Option<crate::NonGraphModel> {
    let expose = request.exposes.first();
    match request.view_type {
        ViewType::Grid => Some(crate::NonGraphModel::Table(
            crate::tmodel::to_traceability_matrix(graph, expose),
        )),
        ViewType::Geometry => Some(crate::NonGraphModel::Geometry(
            crate::gmodel::to_geometry_model(graph, expose),
        )),
        ViewType::Browser => Some(crate::NonGraphModel::Tree(crate::tree::to_tree_model(
            graph, expose,
        ))),
        _ => None,
    }
}

fn build_view_model_impl(
    graph: &ModelGraph,
    request: &ViewRequest,
    filter_cache: Option<&HashMap<ElementId, Arc<ExprIR>>>,
) -> ViewModel {
    if let Some(non_graph) = build_non_graph(graph, request) {
        // Non-graph family: the renderer consumes `non_graph`, not a graph scene.
        // Emit only a minimal scene that carries `view_type` (the dispatch
        // carrier) — running the full Grid/Browser/Geometry IR generators here
        // would scan the whole graph to produce a scene the renderer ignores
        // (steward Q2). `frame`/`text_map`/`interactions` join by id, independent
        // of scene node content. The legacy IR generators stay on the
        // `to_view_model` (LSP/CLI export) path.
        ViewModel::new(DiagramIR::new_fixed(request.view_type)).with_non_graph(non_graph)
    } else {
        ViewModel::new(build_scene(graph, request, filter_cache))
    }
}

/// Build the [`ViewModel`] for a structured [`ViewRequest`]. Pure function of
/// `(graph, request)` — salsa-cacheable.
pub fn to_view_model(graph: &ModelGraph, request: &ViewRequest) -> ViewModel {
    build_view_model_impl(graph, request, None)
}

/// Like [`to_view_model`], but routes a precompiled filter-expression cache into
/// the generator (mirrors `to_view_model_with_filter_cache`).
pub fn to_view_model_with_filter_cache(
    graph: &ModelGraph,
    request: &ViewRequest,
    filter_cache: &HashMap<ElementId, Arc<ExprIR>>,
) -> ViewModel {
    build_view_model_impl(graph, request, Some(filter_cache))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::view_index::ExposeRef;
    use sysml_core::ElementKind;

    fn expose(qname: &str) -> ExposeRef {
        ExposeRef {
            id: ElementId::from_string(format!("expose-{qname}")),
            is_namespace: false,
            qualified_name: Some(qname.to_owned()),
            exposed_element_id: Some(ElementId::from_string(qname)),
        }
    }

    fn summary(name: Option<&str>, exposed: Vec<ExposeRef>, filters: usize) -> ViewSummary {
        ViewSummary {
            id: ElementId::from_string("view-1"),
            name: name.map(str::to_owned),
            kind: ElementKind::ViewUsage,
            exposed,
            renderings: Vec::new(),
            filters: (0..filters)
                .map(|i| ElementId::from_string(format!("filter-{i}")))
                .collect(),
            source_span: None,
        }
    }

    #[test]
    fn frame_populates_name_and_expose_compartment() {
        let s = summary(Some("EngineView"), vec![expose("P::Engine")], 0);
        let frame = view_frame_from_summary(&ModelGraph::new(), &s, ViewType::General);

        assert_eq!(frame.name, "EngineView");
        assert_eq!(frame.view_kind, ViewType::General);
        // No element in the (empty) graph declares a supertype → no heading suffix.
        assert_eq!(frame.type_name, None);
        // Expose/Filter summary lands in the top-right info compartment.
        assert_eq!(
            frame.top_right.as_ref().map(|s| s.text.as_str()),
            Some("expose P::Engine")
        );
        // SPEC-SILENT compartments stay None.
        assert!(frame.bottom_left.is_none() && frame.bottom_right.is_none());
    }

    #[test]
    fn frame_with_filters_notes_them() {
        let s = summary(Some("V"), vec![expose("A")], 2);
        let frame = view_frame_from_summary(&ModelGraph::new(), &s, ViewType::Interconnection);
        let tr = frame.top_right.expect("expose+filter summary present");
        assert!(tr.text.contains("expose A"));
        assert!(tr.text.contains("2 criteria"));
    }

    #[test]
    fn frame_with_no_exposes_or_filters_has_empty_info_compartment() {
        let s = summary(Some("Bare"), vec![], 0);
        let frame = view_frame_from_summary(&ModelGraph::new(), &s, ViewType::General);
        assert_eq!(frame.name, "Bare");
        assert!(frame.top_right.is_none(), "no exposes/filters → no info compartment");
    }

    #[test]
    fn frame_type_name_is_the_literal_declared_supertype() {
        // R7 (§8.2.3.26): the heading suffix is the view's LITERALLY-declared
        // immediate supertype, read from `supertype_names().first()` — NOT
        // canonicalized to a standard view kind.
        use sysml_core::Element;
        let mut graph = ModelGraph::new();
        let view_id = ElementId::from_string("view-1");
        let mut view = Element::new(view_id.clone(), ElementKind::ViewDefinition);
        view.set_prop("unresolved_type", "StateTransition");
        graph.add_element(view);
        let s = summary(Some("DriveModesView"), vec![], 0);
        let frame = view_frame_from_summary(&graph, &s, ViewType::StateTransition);
        assert_eq!(frame.name, "DriveModesView");
        assert_eq!(frame.type_name.as_deref(), Some("StateTransition"));
    }

    #[test]
    fn pruned_to_referenced_scopes_text_map_to_scene_ids() {
        use crate::ir::{DiagramChild, DiagramNode};
        use crate::VisualKind;
        use crate::text_map::build_text_map;
        use sysml_core::Element;
        use sysml_span::Span;

        // Graph with three spanned elements; the scene references only two.
        // Scene ids are `ElementId::to_string()` — mint them the same way.
        let [a, b, offscene] =
            ["a", "b", "offscene"].map(|n| ElementId::from_string(n).to_string());
        let mut graph = ModelGraph::new();
        for id in [&a, &b, &offscene] {
            let mut el = Element::new(ElementId::from_string(id.clone()), ElementKind::PartUsage);
            el.spans.push(Span::new("m.sysml", 0, 1));
            graph.add_element(el);
        }
        let text_map = build_text_map(&graph);
        assert_eq!(text_map.len(), 3);

        let mut node = DiagramNode::new(a.clone(), VisualKind::Part, "a");
        node.children.push(DiagramChild::Text {
            compartment: crate::CompartmentKind::Attributes,
            text: "b".into(),
            element_id: b.clone(),
            source: crate::ir::types::CompartmentItemSource::Owned,
        });
        let vm = ViewModel::new(DiagramIR {
            view_type: ViewType::General,
            nodes: vec![node],
            edges: Vec::new(),
            buttons: Vec::new(),
        })
        .with_text_map(Arc::new(text_map));

        let ids = vm.referenced_element_ids();
        assert!(ids.contains(&a) && ids.contains(&b) && !ids.contains(&offscene));

        let pruned = vm.pruned_to_referenced();
        let tm = pruned.text_map().expect("text_map survives pruning");
        assert_eq!(tm.len(), 2);
        assert!(tm.span_for(&a).is_some() && tm.span_for(&b).is_some());
        assert!(tm.span_for(&offscene).is_none(), "off-scene span pruned");
        // Every retained id is a referenced id (pruned map ⊆ scene ids).
        assert!(tm.iter().all(|(id, _)| ids.contains(id)));
    }

    #[test]
    fn referenced_ids_cover_non_graph_tree_payload() {
        use crate::{NonGraphModel, TreeModel, TreeNode};

        let tree = TreeModel {
            title: None,
            kind: Some("containment_tree".into()),
            roots: vec![TreeNode {
                id: "root".into(),
                element_id: Some("root".into()),
                label: "Root".into(),
                kind_label: None,
                stereotype: None,
                css_classes: Vec::new(),
                children: vec![TreeNode {
                    id: "leaf".into(),
                    element_id: Some("leaf".into()),
                    label: "Leaf".into(),
                    kind_label: None,
                    stereotype: None,
                    css_classes: Vec::new(),
                    children: Vec::new(),
                }],
            }],
        };
        let vm = ViewModel::new(DiagramIR::new_fixed(ViewType::Browser))
            .with_non_graph(NonGraphModel::Tree(tree));

        let ids = vm.referenced_element_ids();
        assert!(ids.contains("root") && ids.contains("leaf"));
    }

    #[test]
    fn ad_hoc_view_has_no_frame() {
        // A scene-only ViewModel (the pure builder) is unframed until the salsa
        // layer attaches a declared-view frame.
        let vm = ViewModel::new(DiagramIR {
            view_type: ViewType::General,
            nodes: Vec::new(),
            edges: Vec::new(),
            buttons: Vec::new(),
        });
        assert!(vm.frame().is_none());
    }
}
