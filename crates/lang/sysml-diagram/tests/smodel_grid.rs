//! GridView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;
use sysml_diagram::tmodel::{to_traceability_matrix, TableModel};

/// Parse + elaborate `source`, then build the traceability matrix,
/// optionally scoped to the element named `expose_name`.
fn matrix(source: &str, expose_name: Option<&str>) -> TableModel {
    let mut graph = parse_sysml(source);
    sysml_core::elaborate::elaborate(&mut graph);
    let expose = expose_name.map(|n| {
        graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(n))
            .unwrap_or_else(|| panic!("expose element {n:?} not found"))
            .id
            .clone()
    });
    to_traceability_matrix(&graph, expose.as_ref())
}

/// Row header names (leading "Requirement" cell of each row), in order.
fn row_names(table: &TableModel) -> Vec<String> {
    table
        .rows
        .iter()
        .map(|r| r.cells[0].display.clone())
        .collect()
}

/// Dynamic column labels (everything after the leading "Requirement" column).
fn col_labels(table: &TableModel) -> Vec<String> {
    table.columns[1..].iter().map(|c| c.label.clone()).collect()
}

/// The display of the cell at (row named `row`, column labelled `col`).
fn cell_at(table: &TableModel, row: &str, col: &str) -> String {
    let ci = table
        .columns
        .iter()
        .position(|c| c.label == col)
        .unwrap_or_else(|| panic!("no column {col:?}"));
    let r = table
        .rows
        .iter()
        .find(|r| r.cells[0].display == row)
        .unwrap_or_else(|| panic!("no row {row:?}"));
    r.cells[ci].display.clone()
}

/// Mini analogue of a compliance-traceability tile: requirement defs
/// verified through declaration-form `verify requirement chk : Def;` inside
/// verification-case objectives.
const COMPLIANCE_MODEL: &str = "package P {
    requirement def NoTrip { }
    requirement def FastTrip { }
    verification def NoTripCase {
        objective o1 { verify requirement chk1 : NoTrip; }
    }
    verification def FastTripCase {
        objective o2 { verify requirement chk2 : FastTrip; }
    }
}";

#[test]
fn matrix_declaration_form_verify_lands_on_requirement_defs() {
    let table = matrix(COMPLIANCE_MODEL, None);

    // Rows: exactly the requirement definitions — no verification cases, no
    // membership-owned check-usages (name-sorted).
    assert_eq!(row_names(&table), vec!["FastTrip", "NoTrip"]);

    // Columns: the verification cases (the evidence axis), never the
    // check-usage names.
    let cols = col_labels(&table);
    assert!(cols.contains(&"NoTripCase".to_owned()), "cols: {cols:?}");
    assert!(cols.contains(&"FastTripCase".to_owned()), "cols: {cols:?}");
    assert!(!cols.iter().any(|c| c.starts_with("chk")), "cols: {cols:?}");

    // Coverage: each def row carries a V under its verifying case — the
    // Verify edge targets the check-usage, which rolls up to its typing def.
    assert_eq!(cell_at(&table, "NoTrip", "NoTripCase"), "V");
    assert_eq!(cell_at(&table, "FastTrip", "FastTripCase"), "V");
    assert_eq!(cell_at(&table, "NoTrip", "FastTripCase"), "");

    // Legend describes the emitted symbol.
    assert!(
        table
            .legend
            .iter()
            .any(|e| e.symbol == "V" && e.label == "Verified by"),
        "legend: {:?}",
        table.legend
    );
}

#[test]
fn matrix_axes_are_disjoint() {
    let table = matrix(COMPLIANCE_MODEL, None);
    let row_ids: Vec<&str> = table.rows.iter().map(|r| r.id.as_str()).collect();
    for col in &table.columns[1..] {
        assert!(
            !row_ids.contains(&col.id.as_str()),
            "element {} ({}) appears on both axes",
            col.label,
            col.id
        );
    }
}

const EXPOSE_MODEL: &str = "package M {
    requirement def SafetyReq { }
    requirement def OtherReq { }
    part def Vehicle { }
    part vehicle : Vehicle { satisfy SafetyReq; }
}";

#[test]
fn matrix_expose_requirement_restricts_rows() {
    // Unscoped: both requirement defs row.
    let all = matrix(EXPOSE_MODEL, None);
    assert_eq!(row_names(&all), vec!["OtherReq", "SafetyReq"]);

    // `expose SafetyReq;` — rows restrict to the exposed requirement, and
    // its satisfy coverage still lands.
    let scoped = matrix(EXPOSE_MODEL, Some("SafetyReq"));
    assert_eq!(row_names(&scoped), vec!["SafetyReq"]);
    assert_eq!(cell_at(&scoped, "SafetyReq", "vehicle"), "S");
    assert!(
        scoped
            .legend
            .iter()
            .any(|e| e.symbol == "S" && e.label == "Satisfied by"),
        "legend: {:?}",
        scoped.legend
    );
}

/// Two requirement-owning packages — the shape a view would scope with
/// `expose A; expose B;`.
const MULTI_EXPOSE_MODEL: &str = "package Root {
    package A { requirement def AlphaReq { } }
    package B { requirement def BetaReq { } }
}";

#[test]
fn matrix_multi_expose_union_is_not_yet_expressible_known_nonconformance() {
    // COVERAGE PIN for a tracked NONCONFORMANCE — see
    // truncation, view_model.rs / smodel/mod.rs `exposes.first()`).
    //
    // The spec derivation `deriveViewUsageExposedElement` flat-maps over ALL
    // Expose relationships on a view:
    //     exposedElement = ownedImport->selectByKind(Expose)
    //         .importedMemberships(Set{}).memberElement->...
    // (SysML-spec-r2025-04 `exposedElement`; SysML-vocab.ttl:1479-1482 —
    // "memberElements of the imported Memberships from ALL the Expose
    // Relationships"). So `expose A; expose B;` MUST scope to the UNION.
    //
    // Today it cannot: `to_traceability_matrix` takes a SINGULAR
    // `Option<&ElementId>`, so the non-graph call sites pass `exposes.first()`
    // and silently drop every expose after the first. Fixing this needs the
    // signature widened to a slice, not just a patched call site.
    //
    // This test exists because task #72 rewrote the only corpus view that
    // exercised multi-expose on the non-graph path (`TraceMatrixView` traded
    // `expose SafetyReq; expose Vehicle;` for `expose Showcase::*;`), which
    // would otherwise have let this bug fall off the radar entirely.
    let alpha_only = matrix(MULTI_EXPOSE_MODEL, Some("A"));
    assert_eq!(row_names(&alpha_only), vec!["AlphaReq"]);

    let beta_only = matrix(MULTI_EXPOSE_MODEL, Some("B"));
    assert_eq!(row_names(&beta_only), vec!["BetaReq"]);

    // The union IS reachable when a single expose root owns both — proving the
    // matrix builder itself is fine and the defect is purely the singular
    // scope parameter plus the `.first()` call sites above it.
    let both = matrix(MULTI_EXPOSE_MODEL, Some("Root"));
    assert_eq!(row_names(&both), vec!["AlphaReq", "BetaReq"]);

    // When the truncation is fixed, `to_traceability_matrix` should accept a
    // slice and `[A, B]` must yield the same rows as the `Root` case above.
}

#[test]
fn matrix_non_requirement_expose_does_not_dump_all_requirements() {
    // `expose Vehicle;` — a part def holds no requirements, so the matrix is
    // honestly empty rather than falling back to every requirement in the
    // model (the old behaviour the R2-10 review flagged on TraceMatrixView).
    let table = matrix(EXPOSE_MODEL, Some("Vehicle"));
    assert!(
        table.rows.is_empty(),
        "expected no rows for a requirement-less expose root, got {:?}",
        row_names(&table)
    );
}

#[test]
fn matrix_package_expose_scopes_to_subtree() {
    let two_packages = "package A {
        requirement def InScope { }
    }
    package B {
        requirement def OutOfScope { }
    }";
    let table = matrix(two_packages, Some("A"));
    assert_eq!(row_names(&table), vec!["InScope"]);
}

#[test]
fn matrix_typed_usage_folds_into_definition_row() {
    // `requirement r : R;` folds into R's row — its satisfy mark lands on R,
    // and r does not appear as a separate (empty-looking) row or column.
    let table = matrix(
        "package Q { requirement def R { } requirement r : R; part s; satisfy r by s; }",
        None,
    );
    assert_eq!(row_names(&table), vec!["R"]);
    assert_eq!(cell_at(&table, "R", "s"), "S");
}

#[test]
fn grid_with_satisfy_relationships() {
    let sg = generate(
        "package P { requirement def R; requirement r : R; part s; satisfy r by s; }",
        ViewType::Grid,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn grid_empty_model_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::Grid, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn grid_serializes() {
    let sg = generate(
        "package P { requirement def R; requirement r : R; part s; satisfy r by s; }",
        ViewType::Grid,
        false,
    );
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}
