//! GridView generator (DiagramIR) — **legacy legacy graph emitter**.
//!
//! Produces a traceability matrix: rows are requirements, columns are
//! verification/design elements, cells mark Satisfy/Verify/Allocate/Derive/Trace
//! intersections. Uses fixed positioning — no ELK layout needed.
//!
//! ## Status
//!
//! The canonical wire path for `view=grid` now goes through
//! [`crate::tmodel::to_traceability_matrix`] → `tagged payload::Table(TableModel)`,
//! which is what REST and MCP serve to the simulation-app FE. This retired graph-renderer
//! generator is retained for LSP push notifications (`sysml/diagram/setModel`)
//! and the retired CLI graph export path, both of which still
//! expect raw `legacy graph`. Delete once those consumers migrate to typed
//! payloads.

use std::collections::{BTreeMap, HashMap, HashSet};

use sysml_core::{ElementId, ElementKind, RelationshipKind};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramNode, HeaderStyle, NodeLayout, NodeTag};
use crate::view_text;
use crate::ViewType;
use crate::visual_kind::{self as classify, VisualKind};

// ── Layout constants ─────────────────────────────────────────────────────

/// Horizontal spacing between column origins.
const COL_SPACING: f64 = 150.0;

/// Vertical spacing between row origins.
const ROW_SPACING: f64 = 60.0;

/// Width of a column header.
const COL_HEADER_WIDTH: f64 = 130.0;

/// Height of a column/row header.
const HEADER_HEIGHT: f64 = 40.0;

/// Width of a row header.
const ROW_HEADER_WIDTH: f64 = 140.0;

/// Width/height of a cell marker.
const CELL_SIZE: f64 = 40.0;

/// Horizontal offset to center a cell within a column.
/// The old generator used `(ci + 1) * 150 + 45`, meaning 45px inset into
/// the 130px-wide column, which centers a 40px cell: (130 - 40) / 2 = 45.
const CELL_X_OFFSET: f64 = 45.0;

// ── Generator ────────────────────────────────────────────────────────────

pub struct GridViewGenerator;

impl ViewGenerator for GridViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::Grid
    }

    fn elk_algorithm(&self) -> &str {
        "fixed"
    }

    fn elk_direction(&self) -> Option<&str> {
        None
    }

    #[instrument(skip_all)]
    #[allow(clippy::indexing_slicing)] // Column indices derived from enumeration of known-length vecs
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!("GridView IR generate");

        let graph = ctx.graph;
        let mut ir = DiagramIR::new_fixed(ViewType::Grid);

        // --- Collect requirements (rows) ---
        // A declared view scopes its rows to the `expose` target — but a trace
        // matrix is inherently multi-element, and only one expose reaches the
        // generator (the first-resolved). So scope by the expose subtree ONLY
        // when that single target can meaningfully anchor a requirement set:
        //   • a requirement  → that requirement + any nested under it
        //   • a Package/Namespace → every requirement under it
        // Otherwise (no expose, or the expose is a part / other leaf) keep every
        // requirement as a row. `passes_filter` applies in all cases. This stops
        // a real workspace from dumping the standard library's requirements
        // without breaking a matrix whose first expose happens to be a part.
        let scope_root = ctx.expose_ids.first().and_then(|id| graph.get_element(id).map(|e| (id, e)));
        let in_scope = |e: &sysml_core::Element| -> bool {
            match &scope_root {
                Some((root_id, root)) if classify::is_requirement_kind(&root.kind) => {
                    &e.id == *root_id || graph.is_descendant_of(&e.id, root_id)
                }
                Some((root_id, root))
                    if matches!(
                        root.kind,
                        ElementKind::Package | ElementKind::LibraryPackage
                    ) =>
                {
                    graph.is_descendant_of(&e.id, root_id)
                }
                _ => true,
            }
        };
        let requirements: Vec<_> = graph
            .elements
            .values()
            .filter(|e| classify::is_requirement_kind(&e.kind))
            // The merged workspace graph carries the standard library's own
            // requirement definitions (RequirementCheck, verification cases, …).
            // A user's trace matrix must not list them as rows.
            .filter(|e| !graph.is_library_element(&e.id))
            .filter(|e| in_scope(e))
            .filter(|e| ctx.passes_filter(e))
            .collect();

        // Build a set of requirement IDs for fast lookup
        let req_id_set: HashSet<ElementId> =
            requirements.iter().map(|r| r.id.clone()).collect();

        // --- Collect Satisfy/Verify/Allocate/Derive/Trace/Dependency relationships ---
        // Map: (requirement_id, target_id) -> relationship kind symbols
        let mut cell_map: HashMap<(ElementId, ElementId), Vec<String>> = HashMap::new();
        let mut col_id_set: HashSet<ElementId> = HashSet::new();

        for rel in graph.relationships.values() {
            if matches!(
                rel.kind,
                RelationshipKind::Satisfy
                    | RelationshipKind::Verify
                    | RelationshipKind::Allocate
                    | RelationshipKind::Derive
                    | RelationshipKind::Refine
                    | RelationshipKind::Trace
                    | RelationshipKind::Dependency
            ) {
                // Row = the requirement endpoint; the other endpoint is the
                // column. Source wins when both ends are requirements
                // (Derive: source = derived requirement = the natural row).
                let (row, col) = if req_id_set.contains(&rel.source) {
                    (rel.source.clone(), rel.target.clone())
                } else if req_id_set.contains(&rel.target) {
                    (rel.target.clone(), rel.source.clone())
                } else {
                    continue;
                };
                col_id_set.insert(col.clone());
                cell_map
                    .entry((row, col))
                    .or_default()
                    .push(rel.kind.matrix_symbol().to_owned());
            }
        }

        // Sort columns for deterministic layout
        let mut col_ids_sorted: Vec<ElementId> = col_id_set.into_iter().collect();
        col_ids_sorted.sort();

        // Build column index: col_id -> col_index
        let col_index: BTreeMap<&ElementId, usize> = col_ids_sorted
            .iter()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();

        // --- Column header nodes ---
        for (ci, col_id) in col_ids_sorted.iter().enumerate() {
            let element = graph.elements.get(col_id);
            let name = element
                .and_then(|e| e.name.as_deref())
                .unwrap_or("unnamed");

            let col_id_str = col_id.to_string();
            let mut node = DiagramNode::new(
                format!("col-header:{}", col_id_str),
                VisualKind::Generic,
                name,
            )
            .with_header_style(HeaderStyle::Inline)
            .with_position(((ci + 1) as f64) * COL_SPACING, 0.0)
            .with_size(COL_HEADER_WIDTH, HEADER_HEIGHT)
            .with_layout(NodeLayout::Free)
            .with_tag(NodeTag::GridColumn);

            // Source location from the element
            if let Some(e) = element {
                super::container::apply_source_metadata(&mut node, e, graph);
            }

            ir.nodes.push(node);
        }

        // --- Row header + cell nodes ---
        for (ri, req) in requirements.iter().enumerate() {
            let req_id_str = req.id.to_string();
            let name = req.name.as_deref().unwrap_or("unnamed");

            // Row header
            let mut row_node = DiagramNode::new(
                format!("row-header:{}", req_id_str),
                VisualKind::Requirement,
                name,
            )
            .with_header_style(HeaderStyle::Inline)
            .with_position(0.0, ((ri + 1) as f64) * ROW_SPACING)
            .with_size(ROW_HEADER_WIDTH, HEADER_HEIGHT)
            .with_layout(NodeLayout::Free)
            .with_tag(NodeTag::GridRow);

            row_node.tooltip = view_text::tooltip_text(req, graph);

            ir.nodes.push(row_node);

            // Cell nodes at intersections
            for col_id in &col_ids_sorted {
                let ci = col_index[col_id];
                let key = (req.id.clone(), col_id.clone());

                let symbols = match cell_map.get(&key) {
                    Some(kinds) => kinds.clone(),
                    None => continue, // Skip empty cells
                };

                let label_text = symbols.join(", ");

                let col_id_str = col_id.to_string();
                // The per-symbol `cell-{s|v|a|d|t}` classes are dropped — the
                // matrix symbols remain in the cell's label text; the cell is
                // tagged only with `NodeTag::GridCell`.
                let cell_node = DiagramNode::new(
                    format!("cell:{}:{}", req_id_str, col_id_str),
                    VisualKind::Generic,
                    label_text,
                )
                .with_header_style(HeaderStyle::Inline)
                .with_position(
                    ((ci + 1) as f64) * COL_SPACING + CELL_X_OFFSET,
                    ((ri + 1) as f64) * ROW_SPACING,
                )
                .with_size(CELL_SIZE, CELL_SIZE)
                .with_layout(NodeLayout::Free)
                .with_tag(NodeTag::GridCell);

                ir.nodes.push(cell_node);
            }
        }

        ir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph, Relationship};
    use std::collections::HashSet;

    static EMPTY_SET: std::sync::LazyLock<HashSet<String>> =
        std::sync::LazyLock::new(HashSet::new);

    fn make_ctx(graph: &ModelGraph) -> GeneratorContext {
        GeneratorContext::new(graph, &EMPTY_SET)
    }

    #[test]
    fn grid_excludes_library_requirements() {
        // A user requirement and a library requirement (inside a registered
        // library package). The trace matrix must list only the user one.
        let mut graph = ModelGraph::new();
        let user_req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("UserReq");
        graph.add_element(user_req);

        let lib_pkg = Element::new_with_kind(ElementKind::LibraryPackage).with_name("StdLib");
        let lib_pkg_id = lib_pkg.id.clone();
        graph.add_element(lib_pkg);
        graph.register_library_package(lib_pkg_id.clone());
        let mut lib_req =
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("RequirementCheck");
        lib_req.owner = Some(lib_pkg_id);
        graph.add_element(lib_req);

        let ir = GridViewGenerator.generate(&make_ctx(&graph));
        let names: Vec<&str> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"UserReq"));
        assert!(!names.contains(&"RequirementCheck"), "library requirement must not be a row");
    }

    #[test]
    fn grid_scopes_rows_to_exposed_requirement() {
        // Two user requirements; exposing one scopes the rows to it.
        let mut graph = ModelGraph::new();
        let a = Element::new_with_kind(ElementKind::RequirementUsage).with_name("ReqA");
        let a_id = graph.add_element(a);
        let b = Element::new_with_kind(ElementKind::RequirementUsage).with_name("ReqB");
        graph.add_element(b);

        let ctx = GeneratorContext::new(&graph, &EMPTY_SET).with_expose(&a_id);
        let ir = GridViewGenerator.generate(&ctx);
        let names: Vec<&str> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"ReqA"));
        assert!(!names.contains(&"ReqB"), "non-exposed requirement must be out of scope");
    }

    #[test]
    fn grid_ir_basic_satisfy() {
        let mut graph = ModelGraph::new();

        let req1 = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req-Speed");
        let req1_id = graph.add_element(req1);

        let req2 =
            Element::new_with_kind(ElementKind::RequirementDefinition).with_name("Req-Safety");
        let req2_id = graph.add_element(req2);

        let part1 = Element::new_with_kind(ElementKind::PartUsage).with_name("Engine");
        let part1_id = graph.add_element(part1);

        let part2 = Element::new_with_kind(ElementKind::PartUsage).with_name("Brakes");
        let part2_id = graph.add_element(part2);

        let satisfy =
            Relationship::new(RelationshipKind::Satisfy, req1_id.clone(), part1_id.clone());
        graph.add_relationship(satisfy);

        let verify = Relationship::new(RelationshipKind::Verify, req2_id.clone(), part2_id);
        graph.add_relationship(verify);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        // View type (fixed layout is adapter-derived from `view_type`)
        assert_eq!(ir.view_type, ViewType::Grid);

        // No edges in grid view
        assert!(ir.edges.is_empty());

        // Count nodes: 2 col headers + 2 row headers + 2 cells = 6
        assert_eq!(ir.nodes.len(), 6);

        // Check node names
        let names: Vec<&str> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Req-Speed"));
        assert!(names.contains(&"Req-Safety"));
        assert!(names.contains(&"Engine"));
        assert!(names.contains(&"Brakes"));

        // Check typed node tags
        let has_grid_col = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::GridColumn));
        assert!(has_grid_col, "should have a GridColumn-tagged node");

        let has_grid_row = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::GridRow));
        assert!(has_grid_row, "should have a GridRow-tagged node");

        let has_grid_cell = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::GridCell));
        assert!(has_grid_cell, "should have a GridCell-tagged node");

        // Satisfy cell carries the "S" symbol in its label.
        let has_cell_s = ir
            .nodes
            .iter()
            .any(|n| n.tags.contains(&NodeTag::GridCell) && n.name == "S");
        assert!(has_cell_s, "satisfy cell should show the S symbol");
    }

    #[test]
    fn grid_ir_no_relationships() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req-A");
        graph.add_element(req);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        // 1 row header, 0 col headers, 0 cells
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "Req-A");
        assert!(ir.nodes[0].tags.contains(&NodeTag::GridRow));
    }

    #[test]
    fn grid_ir_empty_graph() {
        let graph = ModelGraph::new();
        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());
    }

    #[test]
    fn grid_ir_verify_relationship() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req-1");
        let req_id = graph.add_element(req);

        let vc =
            Element::new_with_kind(ElementKind::VerificationCaseUsage).with_name("TestCase-1");
        let vc_id = graph.add_element(vc);

        let verify = Relationship::new(RelationshipKind::Verify, req_id, vc_id);
        graph.add_relationship(verify);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        // Find cell node with "V" label
        let cell = ir.nodes.iter().find(|n| n.name == "V");
        assert!(cell.is_some(), "verify cell should have V label");

        let cell = cell.unwrap();
        assert!(
            cell.tags.contains(&NodeTag::GridCell),
            "verify cell should be tagged GridCell"
        );
    }

    #[test]
    fn grid_ir_extended_relationship_types() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req-1");
        let req_id = graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Component");
        let part_id = graph.add_element(part);

        let allocate =
            Relationship::new(RelationshipKind::Allocate, req_id.clone(), part_id.clone());
        graph.add_relationship(allocate);

        let derive =
            Relationship::new(RelationshipKind::Derive, req_id.clone(), part_id.clone());
        graph.add_relationship(derive);

        let trace = Relationship::new(RelationshipKind::Trace, req_id, part_id);
        graph.add_relationship(trace);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        // Find the cell node (should have combined symbols)
        let cell = ir.nodes.iter().find(|n| n.tags.contains(&NodeTag::GridCell));
        assert!(cell.is_some(), "should have a cell node");

        // The combined matrix symbols live in the cell's label text.
        let cell = cell.unwrap();
        assert!(cell.name.contains('A'), "allocate symbol A in label");
        assert!(cell.name.contains('D'), "derive symbol D in label");
        assert!(cell.name.contains('T'), "trace symbol T in label");
    }

    #[test]
    fn grid_ir_cell_symbols() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let req_id = graph.add_element(req);

        let p1 = Element::new_with_kind(ElementKind::PartUsage).with_name("P1");
        let p1_id = graph.add_element(p1);

        let p2 = Element::new_with_kind(ElementKind::PartUsage).with_name("P2");
        let p2_id = graph.add_element(p2);

        // Satisfy -> S
        let sat = Relationship::new(RelationshipKind::Satisfy, req_id.clone(), p1_id);
        graph.add_relationship(sat);

        // Verify -> V
        let ver = Relationship::new(RelationshipKind::Verify, req_id, p2_id);
        graph.add_relationship(ver);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        let cell_names: Vec<&str> = ir
            .nodes
            .iter()
            .filter(|n| n.tags.contains(&NodeTag::GridCell))
            .map(|n| n.name.as_str())
            .collect();

        assert!(cell_names.contains(&"S"), "satisfy cell should show S symbol");
        assert!(cell_names.contains(&"V"), "verify cell should show V symbol");
    }

    #[test]
    fn grid_ir_fixed_positions() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let req_id = graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("P1");
        let part_id = graph.add_element(part);

        let sat = Relationship::new(RelationshipKind::Satisfy, req_id, part_id);
        graph.add_relationship(sat);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        // Column header at (1*150, 0) = (150, 0)
        let col_header = ir.nodes.iter().find(|n| n.name == "P1").unwrap();
        assert_eq!(col_header.position, Some((150.0, 0.0)));
        assert_eq!(col_header.size, Some((130.0, 40.0)));

        // Row header at (0, 1*60) = (0, 60)
        let row_header = ir.nodes.iter().find(|n| n.name == "R1").unwrap();
        assert_eq!(row_header.position, Some((0.0, 60.0)));
        assert_eq!(row_header.size, Some((140.0, 40.0)));

        // Cell at (1*150 + 45, 1*60) = (195, 60)
        let cell = ir
            .nodes
            .iter()
            .find(|n| n.tags.contains(&NodeTag::GridCell))
            .unwrap();
        assert_eq!(cell.position, Some((195.0, 60.0)));
        assert_eq!(cell.size, Some((40.0, 40.0)));
    }

    // (removed `grid_ir_source_locations` — node source_uri/source_range was
    //  dropped in 3.15; source spans now live only in the ViewModel text-map.)

    #[test]
    fn grid_ir_all_nodes_have_inline_headers() {
        let mut graph = ModelGraph::new();

        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let req_id = graph.add_element(req);

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("P1");
        let part_id = graph.add_element(part);

        let sat = Relationship::new(RelationshipKind::Satisfy, req_id, part_id);
        graph.add_relationship(sat);

        let gen = GridViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        for node in &ir.nodes {
            assert_eq!(
                node.header_style,
                HeaderStyle::Inline,
                "all grid nodes should use inline headers, but {:?} uses {:?}",
                node.element_id,
                node.header_style
            );
        }
    }



    #[test]
    fn grid_ir_view_type_and_algorithm() {
        let gen = GridViewGenerator;
        assert_eq!(gen.view_type(), ViewType::Grid);
        assert_eq!(gen.elk_algorithm(), "fixed");
        assert_eq!(gen.elk_direction(), None);
    }
}
