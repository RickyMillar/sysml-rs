//! RSC-5.2b — the value-binding (`=` / `:=`) form of UQ001 (D-5.0.8).
//!
//! An attribute whose declared ISQ dimension contradicts the dimension of its
//! default *value expression* is a UQ001 hard error, emitted by
//! `quantity_mismatch_health_diagnostics` alongside the binding-connector form.
//!
//! This suite lives in its own file (not `rsc5_quantity_baseline.rs`) so its
//! committed surface stays disjoint from the parallel RSC-5.4 lane working in
//! that file.
//!
//! Verify-before-build facts proven here empirically:
//!  - a non-literal default value is reachable as `ExprIR` via
//!    `compile_expression(attr, graph)` (the parser emits the value subtree as
//!    an attribute child), and `infer_m_ref` resolves to the declared ISQ
//!    *type* (path #2) because no `[unit]` prop is folded onto an expression;
//!  - the conservative `expr_dimension` keeps the real ISQ proving fixtures
//!    clean (no false positives across attribute default values).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{context_from_graph, ModelCompiler};
use sysml_runtime::quantity_health::quantity_mismatch_health_diagnostics;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn load_file(rel: &str) -> ModelGraph {
    let path = workspace_root().join(rel);
    assert!(path.exists(), "model file not found: {}", path.display());
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let name = Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .expect("file name")
        .to_owned();
    let parser = TreeSitterParser::new();
    parser.parse(&[SysmlFile::new(name, source)]).graph
}

/// The cross-dimension value-binding fixture fires exactly one UQ001 error (the
/// `badLen : LengthValue = mass * 2` declaration), and nothing for the two
/// dimensionally-consistent default expressions (`area`, `scaledLen`).
#[test]
fn rsc52b_value_binding_fires_uq001() {
    // Elaborated through ModelCompiler so the value subtree + types are present.
    let compiler = ModelCompiler::new(load_file(
        "examples/quantity-value-mismatch/QuantityValueMismatch.sysml",
    ));
    let diags = quantity_mismatch_health_diagnostics(compiler.graph());

    let uq001: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ001"))
        .collect();

    assert_eq!(
        uq001.len(),
        1,
        "exactly one value-binding mismatch (badLen length = mass*2), got: {diags:#?}"
    );
    assert_eq!(uq001[0].severity, sysml_span::Severity::Error);
    assert!(
        uq001[0].message.contains("default value"),
        "UQ001 value-binding wording: {}",
        uq001[0].message
    );
}

/// Load a single-file fixture parsed AND name-resolved — required for the
/// port-feature/signal path (`resolve_references` populates the FeatureTyping
/// `type` refs that `find_feature_type` + `compile_ports` need; the
/// `ModelCompiler::new` path elaborates but never resolves). See ledger L36.
fn load_resolved(rel: &str) -> ModelGraph {
    let mut graph = load_file(rel);
    let _ = sysml_core::resolution::resolve_references(&mut graph);
    graph
}

/// Build the workspace orchestrator and return its compile diagnostics — UQ002
/// (cross-dimension signal link) is emitted by `compile_signal_propagation`
/// into the signal-prop diagnostics, surfaced via `push_compile_warnings`.
fn compile_warnings_for(rel: &str) -> Vec<sysml_span::Diagnostic> {
    let compiler = ModelCompiler::new(load_resolved(rel));
    let base_ctx = context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    let orch = compiler
        .build_workspace_orchestrator(
            base_ctx,
            Some(precompiled),
            None,
            None,
            None,
            &[],
            Some(10.0),
            Some(60_000.0),
        )
        .expect("workspace orchestrator should compile");
    orch.compile_warnings().to_vec()
}

/// RSC-5.2 UQ002 — a SignalLink whose source slot (`reading : ElectricCurrentValue`,
/// dim I) and target slot (`reading : ThermodynamicTemperatureValue`, dim Θ) carry
/// incompatible non-zero dimensions fires exactly one UQ002 hard error. The
/// flow+connect doubling collapses to one logical path (L29/L30 dedup), so the
/// error is emitted once, not twice.
#[test]
fn rsc52_cross_dim_signal_fires_uq002() {
    let diags = compile_warnings_for(
        "examples/quantity-signal-mismatch/QuantitySignalMismatch.sysml",
    );
    let uq002: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ002"))
        .collect();
    assert_eq!(
        uq002.len(),
        1,
        "exactly one cross-dim signal mismatch (current vs temperature), got: {diags:#?}"
    );
    assert_eq!(uq002[0].severity, sysml_span::Severity::Error);
    assert!(
        uq002[0].message.contains("incompatible quantity dimensions"),
        "UQ002 wording: {}",
        uq002[0].message
    );
}

/// False-positive guard: the exchange-plane fixture HAS live signal links, but
/// their `reading` feature is a plain `Reading` item (dimensionless — no mRef),
/// so UQ002 must stay silent. This pins that a same-/no-dimension signal link
/// never false-fires (the cross-dim check requires both endpoints dimensioned).
#[test]
fn rsc52_signal_fixtures_clean_of_uq002() {
    let diags = compile_warnings_for("examples/exchange-plane-fixture/ExchangePlane.sysml");
    let uq002: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ002"))
        .collect();
    assert!(
        uq002.is_empty(),
        "dimensionless/item signal links must not fire UQ002, got: {uq002:#?}"
    );
}

/// False-positive guard: the real ISQ proving fixtures (whose attribute default
/// values, where present, are dimensionally consistent) must produce ZERO
/// UQ001 diagnostics from the value-binding scan. This is the corpus-clean
/// regression pin for the conservative `expr_dimension` over default values.
#[test]
fn rsc52b_value_binding_fixtures_clean() {
    for rel in [
        "examples/coulomb-friction/CoulombFriction.sysml",
        "examples/radiation-cooling/RadiationCooling.sysml",
        "examples/espresso-pump-hybrid/Verification/PumpSafety.sysml",
    ] {
        let compiler = ModelCompiler::new(load_file(rel));
        let diags = quantity_mismatch_health_diagnostics(compiler.graph());
        assert!(
            diags.is_empty(),
            "{rel} should produce no UQ001 quantity diagnostics, got: {diags:#?}"
        );
    }
}
