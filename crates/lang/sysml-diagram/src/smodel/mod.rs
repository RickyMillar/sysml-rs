// ── Diagram view generators ──────────────────────────────────────────────
//
// **Decoupling contract:**
// • Each view generator (general, sequence, interconnection, …) produces
//   an independent SGraph. Adding/changing one generator must NEVER break
//   another.
// • Shared infrastructure (types, builders, classify) is view-agnostic.
//   View-specific fields on SModel types MUST be `Option`/`Default` so
//   existing constructors keep compiling when a new field is added.
// • On the TypeScript side, dispatch is by CSS class and node/edge type,
//   never by view name.
// ─────────────────────────────────────────────────────────────────────────

pub(crate) mod builders;
pub mod types;

pub use types::*;

use std::collections::HashMap;
use std::sync::Arc;

use sysml_core::{ElementId, ModelGraph};
use sysml_runtime::expressions::ExprIR;
use sysml_runtime::LinkGraph;
use tracing::instrument;

/// The type of diagram view to generate.
///
/// The 8 variants map directly to the spec's standard `ViewDefinition`
/// set (`General`, `Interconnection`, `StateTransition`, `ActionFlow`,
/// `Sequence`, `Geometry`, `Grid`, `Browser`).
///
/// There are no other view kinds: a "requirement view" is a `General`
/// view with a requirement-shaped filter (declared on the view def
/// itself or inherited through its `:>` chain), and constraint /
/// binding ("parametric") notation renders in `Interconnection`.
/// Requirement and constraint notation is gated on element kind in the
/// shared render path, not on a dedicated view kind. See
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ViewType {
    General,
    Interconnection,
    StateTransition,
    ActionFlow,
    Browser,
    Sequence,
    Grid,
    Geometry,
}

impl ViewType {
    /// Map a caller-supplied wire/request string to a `ViewType`,
    /// tolerantly: the canonical standard-library def name
    /// (`"InterconnectionView"`), the lowercase kind token
    /// (`"interconnection"`), and the short CLI aliases (`"state"`,
    /// `"action"`) all parse.
    ///
    /// This is a TRANSPORT convenience for deserialising request
    /// parameters in tool crates (LSP commands, HTTP/MCP params). It must
    /// never be used to classify names found in a model or graph — model
    /// resolution recognises only the canonical `*View` standard defs (see
    /// `view_request::resolve_view_kind`).
    pub fn from_request_str(s: &str) -> Option<Self> {
        match s {
            "GeneralView" | "general" => Some(Self::General),
            "InterconnectionView" | "interconnection" => Some(Self::Interconnection),
            "StateTransitionView" | "statetransition" | "state" => Some(Self::StateTransition),
            "ActionFlowView" | "actionflow" | "action" => Some(Self::ActionFlow),
            "BrowserView" | "browser" => Some(Self::Browser),
            "SequenceView" | "sequence" => Some(Self::Sequence),
            "GridView" | "grid" => Some(Self::Grid),
            "GeometryView" | "geometry" => Some(Self::Geometry),
            _ => None,
        }
    }
}

/// Generate an SModel graph honoring a structured [`crate::ViewRequest`].
///
/// This is the canonical generator entry point for Phase 4.5d. The
/// `request.filter` and `request.hints` fields flow into
/// [`crate::ir::GeneratorContext`], so a preset (e.g. Parametric) renders
/// through its underlying view kind (Interconnection) with the preset's
/// filter and hints applied. After the generator produces the IR,
/// `request.overlays` run in order to add visual fidelity (solver value
/// badges, requirement compartments, etc.) before render emits SGraph.
#[instrument(skip_all, fields(view_type = ?request.view_type))]
pub fn to_smodel_with(graph: &ModelGraph, request: &crate::ViewRequest) -> SGraph {
    let start = std::time::Instant::now();
    tracing::info!(
        view_type = ?request.view_type,
        expanded_count = request.expanded_ids.len(),
        has_filter = request.filter.is_some(),
        has_hints = request.hints.is_some(),
        overlay_count = request.overlays.len(),
        "generating SModel",
    );

    // Build the scene via the shared `build_scene` (the full IR-generator path
    // for ALL view families, incl. Grid/Browser/Geometry) and render it to
    // SGraph. NOTE: this deliberately does NOT go through `to_view_model` —
    // that path (3.12) short-circuits the non-graph families to a minimal scene
    // + a structured `non_graph` model for the new renderer; the legacy SGraph
    // export (LSP `diagram.*` / CLI `export smodel`) still needs the real IR
    // scene for those views.
    let scene = crate::view_model::build_scene(graph, request, None);
    let sgraph = crate::ir::render_with(&scene, request.hints.as_ref());

    let elapsed = start.elapsed();
    tracing::info!(
        view_type = ?request.view_type,
        children_count = sgraph.children.len(),
        elapsed_ms = elapsed.as_secs_f64() * 1000.0,
        "SModel generation complete"
    );
    sgraph
}

/// Generate an SModel graph honoring a structured request **and** a
/// precompiled-filter-expression cache.
///
/// Same shape as [`to_smodel_with`] but routes the
/// `filter_cache` map into [`crate::ir::GeneratorContext`] so per-
/// element filter evaluation reads cached `ExprIR` instead of
/// re-compiling on each call. Closes ADR-011 §3 / S3.T6b.
///
/// Output is byte-identical to `to_smodel_with` for the same
/// `(graph, request)` — the cache only changes execution cost. Cache
/// misses still fall through to the on-demand path with the same
/// safe fall-through-to-true semantics as before.
#[instrument(skip_all, fields(view_type = ?request.view_type))]
pub fn to_smodel_with_filter_cache(
    graph: &ModelGraph,
    request: &crate::ViewRequest,
    filter_cache: &HashMap<ElementId, Arc<ExprIR>>,
) -> SGraph {
    // See `to_smodel_with`: the SGraph path uses the full IR-generator scene for
    // every view family, NOT the 3.12 non-graph short-circuit in `to_view_model`.
    let scene = crate::view_model::build_scene(graph, request, Some(filter_cache));
    crate::ir::render_with(&scene, request.hints.as_ref())
}

/// Generate a tagged `DiagramPayload` honoring a structured request
/// **and** a precompiled-filter-expression cache. This is the canonical
/// payload entry point: it dispatches by view family (Table/Geometry/Tree)
/// and renders the graph branch through [`to_smodel_with_filter_cache`].
pub fn to_payload_with_filter_cache(
    graph: &ModelGraph,
    request: &crate::ViewRequest,
    filter_cache: &HashMap<ElementId, Arc<ExprIR>>,
) -> crate::DiagramPayload {
    let expose = request.exposes.first();
    match request.view_type {
        ViewType::Grid => crate::DiagramPayload::NonGraph(crate::NonGraphModel::Table(
            crate::tmodel::to_traceability_matrix(graph, expose),
        )),
        ViewType::Geometry => crate::DiagramPayload::NonGraph(crate::NonGraphModel::Geometry(
            crate::gmodel::to_geometry_model(graph, expose),
        )),
        ViewType::Browser => crate::DiagramPayload::NonGraph(crate::NonGraphModel::Tree(
            crate::tree::to_tree_model(graph, expose),
        )),
        _ => crate::DiagramPayload::Graph(to_smodel_with_filter_cache(
            graph,
            request,
            filter_cache,
        )),
    }
}

/// Generate an ActionFlowView SGraph for a specific named action.
///
/// This is used by the LSP server when targeting a single ActionDefinition
/// by name (e.g. from a code lens or command).
pub fn generate_action_named(graph: &ModelGraph, action_name: &str) -> SGraph {
    let ir = crate::ir::generators::action::generate_named_ir(graph, action_name);
    crate::ir::render(&ir)
}

/// Generate a SequenceView SGraph from a classified [`LinkGraph`].
///
/// Used by the service `flow_visualize` command. Connectors surface and
/// PowerBonds drop (RSC-3.5c.2b).
pub fn generate_sequence_from_flows(
    lg: &LinkGraph,
    graph: Option<&ModelGraph>,
) -> SGraph {
    let edges = crate::ir::generators::sequence::seq_edges_from_link_graph(lg);
    let ir = crate::ir::generators::sequence::generate_from_flows(&edges, graph);
    crate::ir::render(&ir)
}

