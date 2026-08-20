//! Tabular payload types for `GridView` and other table-shaped renderings.
//!
//! Produced by [`to_traceability_matrix`] and embedded in
//! `DiagramPayload::Table` for the wire format. Consumers (FE, MCP clients,
//! REST callers) work with rows × columns directly — no Sprotty SModel or
//! ELK layout involved.

use serde::Serialize;
use sysml_core::{ElementId, ElementKind, ModelGraph, RelationshipKind};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A complete tabular payload.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableModel {
    /// Optional descriptive title (e.g. "Traceability Matrix").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tag identifying which table generator produced this payload —
    /// useful for FE routing if multiple table flavours coexist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Column definitions in display order. The i-th column maps to
    /// each row's `cells[i]`.
    pub columns: Vec<TableColumn>,
    /// Rows in display order.
    pub rows: Vec<TableRow>,
    /// Legend for the cell symbols this table emits (e.g. "S" = Satisfied
    /// by). Only symbols that actually appear are listed; empty (and
    /// omitted from the wire format) when the table carries no marks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub legend: Vec<TableLegendEntry>,
}

/// One legend entry: a cell symbol and its human-readable meaning.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableLegendEntry {
    /// The symbol exactly as it appears in cells (e.g. "S", "V").
    pub symbol: String,
    /// Human-readable meaning, phrased row-relative ("Satisfied by").
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    /// Stable column identifier — often an `ElementId` for dynamic columns.
    pub id: String,
    /// Display label for the column header.
    pub label: String,
    /// Cell-content hint; clients can use it for alignment / sorting.
    pub kind: TableColumnKind,
}

/// Cell-content type hint.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TableColumnKind {
    Text,
    Number,
    Boolean,
    /// Short symbolic value (e.g. "S", "V") often paired with a CSS class.
    Symbol,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    /// Stable row identifier — often an `ElementId`.
    pub id: String,
    /// Cell values in column order. `cells.len()` must equal `columns.len()`.
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCell {
    /// Pre-formatted display string. Empty for visually empty cells.
    pub display: String,
    /// Optional CSS class hints (e.g. `"cell-s"`, `"cell-v"`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    /// Optional element id for cell-level navigation / selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

// ── Generators ───────────────────────────────────────────────────────────

/// The relationship kinds a traceability matrix draws marks from.
const MATRIX_RELATIONSHIP_KINDS: [RelationshipKind; 7] = [
    RelationshipKind::Satisfy,
    RelationshipKind::Verify,
    RelationshipKind::Allocate,
    RelationshipKind::Derive,
    RelationshipKind::Refine,
    RelationshipKind::Trace,
    RelationshipKind::Dependency,
];

/// Row-relative legend label for a matrix relationship kind.
fn legend_label(kind: &RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::Satisfy => "Satisfied by",
        RelationshipKind::Verify => "Verified by",
        RelationshipKind::Allocate => "Allocated to",
        RelationshipKind::Derive => "Derives from",
        RelationshipKind::Refine => "Refined by",
        RelationshipKind::Trace => "Traces to",
        RelationshipKind::Dependency => "Dependency",
        _ => "Related to",
    }
}

/// A requirement-proper kind: eligible as a matrix ROW. Verification cases
/// are deliberately NOT here — they are evidence (columns), never rows.
/// `SatisfyRequirementUsage` is an assertion artifact, also not a row.
fn is_row_requirement_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::RequirementDefinition
            | ElementKind::RequirementUsage
            | ElementKind::ConcernDefinition
            | ElementKind::ConcernUsage
    )
}

/// Build a traceability matrix from the model graph.
///
/// Rows: requirement definitions plus standalone requirement usages —
/// requirements-proper only. Verification cases are never rows (they are
/// evidence), membership-owned check-usages (`verify requirement chk : R;`)
/// fold into the requirement definition they are typed by, and a usage typed
/// by an in-scope definition folds into that definition's row. Rows and
/// columns are disjoint by construction (except a requirement-to-requirement
/// Derive/Refine/Trace, where the target requirement legitimately appears as
/// a column too).
///
/// Columns: a leading "Requirement" name column plus one per element on the
/// other end of a Satisfy / Verify / Allocate / Derive / Refine / Trace /
/// Dependency relationship (parts, verification cases, …). Cells carry the
/// relationship-kind symbol(s); `legend` maps each emitted symbol to a
/// human-readable meaning.
pub fn to_traceability_matrix(graph: &ModelGraph, expose: Option<&ElementId>) -> TableModel {
    // ── Scope (3.12 / R2-10) ──
    // When the view exposes an element, the row set restricts to requirements
    // within that element's subtree (the root itself included, so
    // `expose SafetyReq;` rows exactly SafetyReq). A non-requirement,
    // non-package expose root (e.g. a part def) scopes the same way — it must
    // NOT fall through to "every requirement in the model"; if nothing under
    // it is a requirement the matrix is honestly empty. The standard
    // library's own requirement defs are always excluded.
    let scope_root: Option<&ElementId> = expose.filter(|id| graph.get_element(id).is_some());
    let in_scope = |e: &sysml_core::Element| -> bool {
        match scope_root {
            Some(root_id) => &e.id == root_id || graph.is_descendant_of(&e.id, root_id),
            None => true,
        }
    };

    // ── Candidate requirements ──
    let candidates: Vec<&sysml_core::Element> = graph
        .elements
        .values()
        .filter(|e| is_row_requirement_kind(&e.kind))
        .filter(|e| !graph.is_library_element(&e.id))
        .filter(|e| in_scope(e))
        // Membership-owned verification check-usages are bookkeeping, not
        // requirements: their coverage rolls up to the typing definition.
        .filter(|e| !sysml_core::query::is_verification_check_usage(graph, &e.id))
        .collect();

    // ── Fold typed usages into their definition row ──
    // A usage typed by an in-scope requirement definition (`requirement
    // r : R;`) does not get its own row — marks on it land on R's row. A
    // usage whose definition is out of scope (or library) keeps its own row.
    let def_ids: HashSet<ElementId> = candidates
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                ElementKind::RequirementDefinition | ElementKind::ConcernDefinition
            )
        })
        .map(|e| e.id.clone())
        .collect();
    let mut requirements: Vec<&sysml_core::Element> = candidates
        .into_iter()
        .filter(|e| {
            if matches!(
                e.kind,
                ElementKind::RequirementUsage | ElementKind::ConcernUsage
            ) {
                if let Some(def) =
                    sysml_core::query::resolve_requirement_typing_target(e, graph)
                {
                    if def_ids.contains(&def.id) {
                        return false; // folds into the definition's row
                    }
                }
            }
            true
        })
        .collect();
    requirements.sort_by(|a, b| (a.name.as_deref(), &a.id).cmp(&(b.name.as_deref(), &b.id)));

    let row_id_set: HashSet<ElementId> = requirements.iter().map(|r| r.id.clone()).collect();

    // Resolve a relationship endpoint to the row it lands on: the endpoint
    // itself when it IS a row, else — for a requirement usage (including the
    // check-usages a declaration-form Verify targets) — the requirement
    // definition it is typed by, when that definition is a row. Single
    // typing hop, mirroring `sysml_core::query`'s def-rollup seam.
    let resolve_row = |id: &ElementId| -> Option<ElementId> {
        if row_id_set.contains(id) {
            return Some(id.clone());
        }
        let e = graph.get_element(id)?;
        if matches!(
            e.kind,
            ElementKind::RequirementUsage | ElementKind::ConcernUsage
        ) {
            let def = sysml_core::query::resolve_requirement_typing_target(e, graph)?;
            if row_id_set.contains(&def.id) {
                return Some(def.id.clone());
            }
        }
        None
    };

    // ── Collect Satisfy / Verify / Allocate / Derive / Refine / Trace /
    //    Dependency marks ──
    let mut cell_map: HashMap<(ElementId, ElementId), Vec<String>> = HashMap::new();
    let mut col_id_set: HashSet<ElementId> = HashSet::new();
    let mut legend_kinds: Vec<RelationshipKind> = Vec::new();

    for rel in graph.relationships.values() {
        if !MATRIX_RELATIONSHIP_KINDS.contains(&rel.kind) {
            continue;
        }
        // Row = the endpoint that resolves into the row set; the other
        // endpoint is the column. Source wins when both ends resolve
        // (Derive: source = derived requirement, the natural row).
        let (row, col) = if let Some(row) = resolve_row(&rel.source) {
            (row, rel.target.clone())
        } else if let Some(row) = resolve_row(&rel.target) {
            (row, rel.source.clone())
        } else {
            continue;
        };
        // A declaration-form Verify targets the check-usage that folded into
        // `row` itself; never mark a row against itself.
        if col == row || resolve_row(&col).is_some_and(|c| c == row) {
            continue;
        }
        col_id_set.insert(col.clone());
        let symbols = cell_map.entry((row, col)).or_default();
        let symbol = rel.kind.matrix_symbol();
        if !symbols.iter().any(|s| s == symbol) {
            symbols.push(symbol.to_owned());
        }
        if !legend_kinds.contains(&rel.kind) {
            legend_kinds.push(rel.kind.clone());
        }
    }

    // Sort dynamic columns deterministically (label first for readability,
    // id as tiebreak).
    let mut col_ids_sorted: Vec<ElementId> = col_id_set.into_iter().collect();
    col_ids_sorted.sort_by(|a, b| {
        let label = |id: &ElementId| {
            graph
                .get_element(id)
                .and_then(|e| e.name.clone())
                .unwrap_or_default()
        };
        (label(a), a.clone()).cmp(&(label(b), b.clone()))
    });

    let col_index: BTreeMap<&ElementId, usize> = col_ids_sorted
        .iter()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();

    // ── Columns: leading "Requirement" + one per target element ──
    let mut columns = Vec::with_capacity(col_ids_sorted.len() + 1);
    columns.push(TableColumn {
        id: "__requirement__".to_owned(),
        label: "Requirement".to_owned(),
        kind: TableColumnKind::Text,
    });
    for col_id in &col_ids_sorted {
        let element = graph.elements.get(col_id);
        let label = element
            .and_then(|e| e.name.as_deref())
            .unwrap_or("unnamed")
            .to_owned();
        columns.push(TableColumn {
            id: col_id.to_string(),
            label,
            kind: TableColumnKind::Symbol,
        });
    }

    // ── Rows ──
    let rows = requirements
        .iter()
        .map(|req| {
            let mut cells = Vec::with_capacity(columns.len());

            // Leading Requirement name cell.
            cells.push(TableCell {
                display: req.name.as_deref().unwrap_or("unnamed").to_owned(),
                css_classes: vec!["table-row-header".to_owned()],
                element_id: Some(req.id.to_string()),
            });

            // Symbol cells.
            let mut empty_cells = vec![TableCell::default(); col_ids_sorted.len()];
            for (col_id, ci) in &col_index {
                let key = (req.id.clone(), (*col_id).clone());
                if let Some(symbols) = cell_map.get(&key) {
                    let display = symbols.join(", ");
                    let mut css = vec!["table-cell".to_owned()];
                    for sym in symbols {
                        css.push(format!("cell-{}", sym.to_lowercase()));
                    }
                    empty_cells[*ci] = TableCell {
                        display,
                        css_classes: css,
                        element_id: Some((*col_id).to_string()),
                    };
                }
            }
            cells.extend(empty_cells);

            TableRow {
                id: req.id.to_string(),
                cells,
            }
        })
        .collect();

    // ── Legend: one entry per relationship kind that actually marked a
    //    cell, in canonical kind order ──
    let legend = MATRIX_RELATIONSHIP_KINDS
        .iter()
        .filter(|k| legend_kinds.contains(k))
        .map(|k| TableLegendEntry {
            symbol: k.matrix_symbol().to_owned(),
            label: legend_label(k).to_owned(),
        })
        .collect();

    TableModel {
        title: Some("Traceability Matrix".to_owned()),
        kind: Some("traceability_matrix".to_owned()),
        columns,
        rows,
        legend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, Relationship};

    #[test]
    fn empty_graph_produces_only_requirement_column() {
        let graph = ModelGraph::new();
        let table = to_traceability_matrix(&graph, None);
        assert_eq!(table.columns.len(), 1, "leading Requirement column only");
        assert_eq!(table.columns[0].label, "Requirement");
        assert!(table.rows.is_empty());
    }

    #[test]
    fn requirement_without_relationships_renders_as_lone_row() {
        let mut graph = ModelGraph::new();
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Req-A");
        graph.add_element(req);

        let table = to_traceability_matrix(&graph, None);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells[0].display, "Req-A");
        // No dynamic columns since no relationships exist.
        assert_eq!(table.columns.len(), 1);
    }

    #[test]
    fn satisfy_and_verify_form_combined_matrix() {
        let mut graph = ModelGraph::new();
        let req1 = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Speed");
        let req1_id = graph.add_element(req1);
        let req2 = Element::new_with_kind(ElementKind::RequirementUsage).with_name("Safety");
        let req2_id = graph.add_element(req2);
        let part1 = Element::new_with_kind(ElementKind::PartUsage).with_name("Engine");
        let part1_id = graph.add_element(part1);
        let part2 = Element::new_with_kind(ElementKind::PartUsage).with_name("Brakes");
        let part2_id = graph.add_element(part2);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Satisfy,
            req1_id,
            part1_id,
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Verify,
            req2_id,
            part2_id,
        ));

        let table = to_traceability_matrix(&graph, None);

        // Leading + 2 dynamic columns.
        assert_eq!(table.columns.len(), 3);
        let labels: Vec<&str> = table.columns.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Requirement"));
        assert!(labels.contains(&"Engine"));
        assert!(labels.contains(&"Brakes"));

        // 2 rows, one per requirement.
        assert_eq!(table.rows.len(), 2);
        // Each row has 3 cells matching column count.
        for row in &table.rows {
            assert_eq!(row.cells.len(), 3);
        }

        // Find the cell with "S" symbol.
        let has_s = table
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .any(|c| c.display == "S" && c.css_classes.iter().any(|cls| cls == "cell-s"));
        assert!(has_s, "Satisfy cell should display 'S' with cell-s class");

        let has_v = table
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .any(|c| c.display == "V" && c.css_classes.iter().any(|cls| cls == "cell-v"));
        assert!(has_v, "Verify cell should display 'V' with cell-v class");
    }

    #[test]
    fn combined_relationship_kinds_join_symbols_in_one_cell() {
        let mut graph = ModelGraph::new();
        let req = Element::new_with_kind(ElementKind::RequirementUsage).with_name("R1");
        let req_id = graph.add_element(req);
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Component");
        let part_id = graph.add_element(part);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Allocate,
            req_id.clone(),
            part_id.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Derive,
            req_id.clone(),
            part_id.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Trace,
            req_id,
            part_id,
        ));

        let table = to_traceability_matrix(&graph, None);
        let cell = table
            .rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .find(|c| c.css_classes.iter().any(|cls| cls == "table-cell"))
            .expect("at least one populated cell");

        // Symbols joined with ", ".
        assert!(
            cell.display.contains("A") && cell.display.contains("D") && cell.display.contains("T"),
            "expected combined A/D/T symbols, got {:?}",
            cell.display
        );
        // Per-kind CSS classes.
        for cls in &["cell-a", "cell-d", "cell-t"] {
            assert!(
                cell.css_classes.iter().any(|c| c == cls),
                "expected {} CSS class, got {:?}",
                cls,
                cell.css_classes
            );
        }
    }

    #[test]
    fn serialised_kind_field_is_present() {
        let table = TableModel {
            kind: Some("traceability_matrix".to_owned()),
            ..Default::default()
        };
        let value = serde_json::to_value(&table).unwrap();
        assert_eq!(
            value.get("kind").and_then(|v| v.as_str()),
            Some("traceability_matrix")
        );
    }
}
