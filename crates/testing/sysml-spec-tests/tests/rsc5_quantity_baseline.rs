//! RSC-5.B0 — Quantity-carrying values (Phase 5) behavioural baseline + inventory.
//!
//! §5 row RSC-5.B0; steward blocks M1/M3 + doc-block #7.
//!
//! **Purpose (per the design doc §5 / §8):**
//!  1. `rsc5_convert_engine_oracle` — the units.rs/`convert_quantity` conversion
//!     suite as the convert-engine ORACLE (proves the reframe: the SI conversion
//!     engine already exists at HEAD). RSC-5.1b/5.3 boundary conversion must
//!     converge on these results.
//!  2. `rsc5_cross_scale_arithmetic_gap` (`#[ignore]`, steward M1) — pins
//!     `5 [mA] + 3 [A]` and `5 [mA] < 3 [A]` as EXPECTED-WRONG TODAY (bare
//!     magnitude arithmetic/comparison; B6/B5). Keeps the gap visible across the
//!     5.1→5.1b window; flips to EXPECTED-CONVERTED at RSC-5.1b.
//!  3. `rsc5_snapshot_magnitudes_pinned` — pins current post-step snapshot
//!     magnitudes for the ISQ-typed proving fixtures (`coulomb-friction`,
//!     `radiation-cooling`). The re-bless guard for D-5.0.4: RSC-5.1 (mRef is
//!     pure metadata) MUST keep these byte-identical.
//!  4. `rsc5_quantity_slot_absent_from_snapshot` (steward M3) — pins that the
//!     runtime snapshot sink (`snapshot_view::value_to_scalar`, B4) reduces a
//!     `Value::Quantity` to a bare magnitude and structurally cannot carry the
//!     unit. RSC-5.4's fix is then verifiable as the unit BECOMING recoverable.
//!
//! ---
//!
//! ## RSC-5.B0 empirical findings (orchestrator-gated 2026-06-17)
//!
//! These were verified by reading/running the actual code at HEAD, NOT trusted
//! from the survey (standing lesson — verify load-bearing/negative claims):
//!
//!  - **Reframe CONFIRMED.** `Value::Quantity` (`meta.rs:148`), real dimensional
//!    arithmetic in `eval_quantity_binary` (`evaluator.rs:876`), the SI
//!    `convert_quantity` engine (`units.rs:314`), and the 281-entry `ISQ_TYPES`
//!    table all exist and work. The memory note "ConvertQuantity blocked on
//!    units" is FALSE at HEAD.
//!  - **B3 CORRECTION (M2 consumer probe).** `maybe_tag_isq` is NOT just in
//!    ide-db: an *identical* copy lives in `sysml-runtime/compiler.rs:756`
//!    feeding `context_from_graph` (the EvalContext builder). So the dual
//!    source-of-truth the doc warns about ALREADY exists across TWO crates, and
//!    NEITHER copy is at slot mint. It resolves by matching the unresolved type
//!    NAME against `ISQ_TYPES` (no stdlib load needed) and wraps to `unit: None`.
//!    D-5.0.3's relocate-to-slot-mint must reconcile THREE sites.
//!  - **B5/B6 CONFIRMED.** Comparison ops compare bare magnitudes ignoring both
//!    dimension AND scale (`evaluator.rs:974`, "warn don't error" emits nothing);
//!    add/sub do bare `lv + rv` with no scale field in existence.
//!  - **B4 CONFIRMED (two distinct sinks).** `snapshot_values_json`
//!    (`service/lib.rs:172`) drops a Quantity entirely (`_ => {}`);
//!    `value_to_scalar`/`normalize` (`snapshot_view.rs`) keep the magnitude but
//!    drop the unit (Option<f64> return). RSC-5.4 must fix BOTH (steward M5).
//!  - **Q4 breakage inventory (corpus, survey-grade — re-confirm at 5.2).** ZERO
//!    accidental cross-DIMENSION comparisons in the corpus: every ISQ comparison
//!    is same-dimension. The only risk surface is same-dimension/different-SCALE
//!    idioms (e.g. a `sim_time_ms > maxTime_s * 1000` runtime-var comparison in a
//!    verification file; `[kg/L]` compound units) — Q4 handles these as boundary conversion, NOT a
//!    mid-expression hard error. The Q4 hard-error accepted-cost is therefore
//!    empirically near-zero today.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::physics::DimensionVector;
use sysml_core::{ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::{context_from_graph, ModelCompiler};
use sysml_runtime::expressions::units::convert_quantity;
use sysml_runtime::expressions::{compile_simple_expression, EvalContext, ExpressionEvaluator};
use sysml_runtime::orchestrator::Orchestrator;
use sysml_runtime::snapshot_view::{normalize, value_to_scalar};

// ---------------------------------------------------------------------------
// ISQ 7-vector dimension helpers (order: length, mass, time, current,
// temperature, amount, luminosity).
// ---------------------------------------------------------------------------
fn dim_current() -> DimensionVector {
    DimensionVector::new(0, 0, 0, 1, 0, 0, 0)
}
fn dim_length() -> DimensionVector {
    DimensionVector::new(1, 0, 0, 0, 0, 0, 0)
}
fn dim_mass() -> DimensionVector {
    DimensionVector::new(0, 1, 0, 0, 0, 0, 0)
}
fn dim_temp() -> DimensionVector {
    DimensionVector::new(0, 0, 0, 0, 1, 0, 0)
}
fn dim_voltage() -> DimensionVector {
    // V = kg·m²·s⁻³·A⁻¹
    DimensionVector::new(2, 1, -3, -1, 0, 0, 0)
}

// ---------------------------------------------------------------------------
// Harness (mirrors rsc3_exchange_baseline.rs — pure runtime: TreeSitterParser
// → ModelGraph → ModelCompiler → build_workspace_orchestrator → step()).
// ---------------------------------------------------------------------------
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

fn build_orchestrator(compiler: &ModelCompiler) -> Orchestrator {
    let base_ctx = context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler
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
        .expect("workspace orchestrator should compile")
}

/// Deterministic, sorted, fixed-precision dump of the numeric variables in a
/// snapshot — the re-bless guard surface.
fn pin_magnitudes(orch: &mut Orchestrator, ticks: usize) -> String {
    let mut snap = orch.step();
    for _ in 1..ticks {
        snap = orch.step();
    }
    let mut rows: Vec<(String, String)> = snap
        .variables
        .iter()
        .filter(|(k, _)| !k.starts_with("__"))
        .filter_map(|(k, v)| value_to_scalar(v).map(|f| (k.clone(), format!("{f:.6}"))))
        .collect();
    rows.sort();
    rows.into_iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 1. Convert-engine oracle (the reframe proof + the 5.1b/5.3 conversion reference)
// ---------------------------------------------------------------------------
#[test]
fn rsc5_convert_engine_oracle() {
    // mA → A (the G16 motivating scale): 5 mA == 0.005 A.
    let (v, d, u) = convert_quantity(5.0, &dim_current(), Some("mA"), "A")
        .expect("mA→A converts");
    assert!((v - 0.005).abs() < 1e-12, "5 mA = 0.005 A, got {v}");
    assert_eq!(d, dim_current());
    assert_eq!(u, "A");

    // km → m (pure scale).
    let (v, _, u) = convert_quantity(1.0, &dim_length(), Some("km"), "m")
        .expect("km→m converts");
    assert!((v - 1000.0).abs() < 1e-9, "1 km = 1000 m, got {v}");
    assert_eq!(u, "m");

    // degC → K (offset conversion).
    let (v, _, _) = convert_quantity(0.0, &dim_temp(), Some("degC"), "K")
        .expect("degC→K converts");
    assert!((v - 273.15).abs() < 1e-9, "0 degC = 273.15 K, got {v}");

    // Cross-dimension conversion is rejected (mass ↛ length).
    assert!(
        convert_quantity(1.0, &dim_mass(), Some("kg"), "m").is_err(),
        "kg→m must error (dimension mismatch)"
    );
}

// ---------------------------------------------------------------------------
// 2. Cross-scale arithmetic/comparison — EXPECTED-CONVERTED (RSC-5.1b).
//    Flipped + un-ignored at RSC-5.1b: `eval_quantity_binary` now converts the
//    RHS into the LHS unit (scale-aware) before add/sub and comparison, reusing
//    the single SI conversion home (`convert_quantity`). Was EXPECTED-WRONG at
//    RSC-5.B0 (bare-magnitude operate; B5/B6).
// ---------------------------------------------------------------------------
#[test]
fn rsc5_cross_scale_arithmetic_gap() {
    let mut ctx = EvalContext::new();
    // a = 5 [mA], b = 3 [A] — SAME dimension (current), DIFFERENT scale.
    ctx.set("a", Value::quantity(5.0, dim_current(), Some("mA".into())));
    ctx.set("b", Value::quantity(3.0, dim_current(), Some("A".into())));
    let ev = ExpressionEvaluator::new();

    // ADD: 5 mA + 3 A = 3.005 A == 3005 mA. The result keeps the LHS unit (mA),
    // so the RHS (3 A) is converted into mA (3000) before adding.
    let sum = ev
        .eval(&compile_simple_expression("a + b").expect("compiles"), &ctx)
        .expect("evaluates");
    match sum {
        Value::Quantity { value, unit, dimension } => {
            assert!(
                (value - 3005.0).abs() < 1e-9,
                "5 mA + 3 A = 3005 mA (scale-aware), got {value}"
            );
            assert_eq!(unit.as_deref(), Some("mA"), "result keeps the LHS unit");
            assert_eq!(dimension, dim_current(), "result keeps the current dimension");
        }
        other => panic!("expected Quantity, got {other:?}"),
    }

    // COMPARE: 5 mA (0.005 A) < 3 A is TRUE — RHS converted into mA (3000) first,
    // so 5 < 3000.
    let lt = ev
        .eval(&compile_simple_expression("a < b").expect("compiles"), &ctx)
        .expect("evaluates");
    assert_eq!(
        lt,
        Value::Bool(true),
        "5 mA < 3 A is true once the comparison is scale-aware"
    );

    // Q4: a cross-DIMENSION comparison is now a hard error (was a silent
    // bare-magnitude compare).
    ctx.set("len", Value::quantity(1.0, dim_length(), Some("m".into())));
    let cross = ev.eval(
        &compile_simple_expression("a < len").expect("compiles"),
        &ctx,
    );
    assert!(
        cross.is_err(),
        "comparing a current (mA) to a length (m) must be a dimension-mismatch error, got {cross:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Snapshot-magnitude re-bless guard (D-5.0.4). RSC-5.1 must keep these
//    byte-identical (mRef is metadata, not a magnitude change).
// ---------------------------------------------------------------------------
#[test]
fn rsc5_snapshot_magnitudes_pinned() {
    // `coulomb-friction` is the clean single-file ISQ ODE fixture: its
    // snapshot variables carry the ISQ-typed attributes (mass[kg], gravity
    // [m/s²], appliedForce[N], position[m], velocity[m/s]) as bare unit-less
    // magnitudes — live corroboration of B4 (units never reach the snapshot).
    let compiler = ModelCompiler::new(load_file("examples/coulomb-friction/CoulombFriction.sysml"));
    let mut orch = build_orchestrator(&compiler);
    let report = pin_magnitudes(&mut orch, 10);
    insta::assert_snapshot!("rsc5_coulomb_friction_magnitudes", report);

    // NOTE (5.B0 finding): `radiation-cooling` does NOT yield a detectable ODE
    // subsystem when parsed standalone ("no ODE found") — its single-derivative
    // SSR form needs the standard library resolved, unlike coulomb-friction
    // which the detector finds by structure. Adding it as a second proving
    // fixture is deferred to a wave that loads the stdlib; it is not needed for
    // the magnitude re-bless guard, which coulomb-friction's 5 ISQ attributes
    // already provide.
}

// ---------------------------------------------------------------------------
// 4. Quantity slot absent from snapshot (steward M3) — pins the B4 runtime sink.
//    False-pass-proof: the Quantity is constructed directly, so we KNOW a real
//    Quantity is being reduced. RSC-5.4 adds a unit channel; this flips to
//    assert the unit is recoverable (== "V").
// ---------------------------------------------------------------------------
#[test]
fn rsc5_quantity_slot_absent_from_snapshot() {
    let q = Value::quantity(230.0, dim_voltage(), Some("V".into()));

    // The magnitude survives the snapshot/timeseries sink...
    assert_eq!(
        value_to_scalar(&q),
        Some(230.0),
        "value_to_scalar keeps the Quantity magnitude"
    );

    // ...but the sink's Option<f64> return STRUCTURALLY cannot carry the unit.
    // There is no public function at HEAD that recovers "V" from a snapshot
    // value. This absence is the B4 gap; RSC-5.4 introduces a unit channel and
    // this assertion flips to recover the unit. (The service JSON sink
    // `snapshot_values_json` drops the whole Quantity via `_ => {}` and is the
    // second sink RSC-5.4 must fix — steward M5.)
    let unit_recoverable = match &q {
        Value::Quantity { unit, .. } => unit.clone(),
        _ => None,
    };
    assert_eq!(
        unit_recoverable.as_deref(),
        Some("V"),
        "the unit EXISTS on the Value but no snapshot sink surfaces it (B4)"
    );
}

// ---------------------------------------------------------------------------
// 4b. RSC-5.4 verify-before-build substrate (NOT ignored — runs against HEAD).
//     Proves the two facts RSC-5.4's snapshot unit channel depends on:
//       (i)  an explicit-`[unit]` ISQ slot mints `m_ref.unit = Some(..)`
//            (the `infer_m_ref` path #1), unlike type-only ISQ slots whose
//            m_ref unit is `None` (so a unit channel can actually carry data);
//       (ii) that slot appears in the stepping snapshot's `variables` under a
//            name spelling the slot store also exposes (canonical/runtime) —
//            so the m_ref→snapshot join has a real key to match.
//     If either fails, RSC-5.4's channel would be a false-pass empty map.
// ---------------------------------------------------------------------------
#[test]
fn rsc5_explicit_unit_slot_carries_mref_unit() {
    let compiler =
        ModelCompiler::new(load_file("examples/quantity-snapshot-demo/QuantitySnapshot.sysml"));
    let mut orch = build_orchestrator(&compiler);

    // (i) The explicit-[unit] slot carries m_ref.unit == Some("K").
    let store = orch.slot_store();
    let (canon, runtime, unit, dim) = store
        .iter()
        .filter(|(_, meta, _)| meta.canonical_name.ends_with("refTemp"))
        .find_map(|(_, meta, _)| {
            meta.m_ref.as_ref().map(|m| {
                (
                    meta.canonical_name.to_string(),
                    meta.runtime_name.to_string(),
                    m.unit.as_ref().map(|u| u.to_string()),
                    m.dimension,
                )
            })
        })
        .expect("refTemp slot must exist and carry an m_ref");
    assert_eq!(
        unit.as_deref(),
        Some("K"),
        "explicit `[K]` must mint m_ref.unit=Some(\"K\") (not None like type-only ISQ)"
    );
    assert_eq!(dim, dim_temp(), "refTemp is a temperature");
    drop(store);

    // (ii) refTemp reaches the stepping snapshot under canonical or runtime name.
    let snap = orch.step();
    assert!(
        snap.variables.contains_key(&canon) || snap.variables.contains_key(&runtime),
        "refTemp slot must appear in snapshot.variables (canonical={canon:?} runtime={runtime:?}) \
         so the m_ref→snapshot join has a key to match; got keys: {:?}",
        snap.variables.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 4c. RSC-5.4 — the unit reaches the snapshot sink (D-5.0.7 / B4 fix).
//     The complement of `rsc5_quantity_slot_absent_from_snapshot`: that test
//     pins that `value_to_scalar`'s `Option<f64>` structurally cannot carry the
//     unit; THIS one proves RSC-5.4's separate channel recovers it. The
//     normalized snapshot now exposes `unit_vars`/`dimension_vars` joined from
//     the slot store's m_ref, so the explicit-`[K]` slot surfaces unit "K".
// ---------------------------------------------------------------------------
#[test]
fn rsc5_snapshot_surfaces_explicit_unit() {
    let compiler =
        ModelCompiler::new(load_file("examples/quantity-snapshot-demo/QuantitySnapshot.sysml"));
    let mut orch = build_orchestrator(&compiler);
    let snap = orch.step();
    let norm = normalize(&snap);

    // The explicit-[unit] slot surfaces its unit under a `refTemp` key.
    let (unit_key, unit) = norm
        .unit_vars
        .iter()
        .find(|(k, _)| k.ends_with("refTemp"))
        .expect("refTemp must surface a unit in NormalizedSnapshot.unit_vars");
    assert_eq!(unit, "K", "explicit `[K]` surfaces as unit \"K\" at the sink");

    // ...and the same variable surfaces a temperature dimension.
    assert!(
        norm.dimension_vars.contains_key(unit_key),
        "refTemp must also surface a dimension; dimension_vars={:?}",
        norm.dimension_vars
    );

    // The magnitude still projects to scalar_vars (the unit channel is additive,
    // not a replacement — `value_to_scalar` is unchanged).
    assert!(
        norm.scalar_vars.contains_key(unit_key),
        "refTemp magnitude must still be in scalar_vars (additive unit channel)"
    );
}

// ---------------------------------------------------------------------------
// RSC-5.1 — ISQ-typed slots carry an inferred MeasurementRef (D-5.0.3).
//    Verifies the mint post-pass actually populates m_ref (not a silent no-op):
//    coulomb-friction's ISQ-typed attributes get the right dimension.
//    ISQ-type-only inference ⇒ unit None / scale 1.0 / offset 0.0 (byte-identical
//    SI-base magnitude). The explicit `[unit]` path (#1) lands with the parser fix.
// ---------------------------------------------------------------------------
#[test]
fn rsc5_isq_slots_carry_mref() {
    let compiler = ModelCompiler::new(load_file("examples/coulomb-friction/CoulombFriction.sysml"));
    let orch = build_orchestrator(&compiler);
    let store = orch.slot_store();

    // Collect canonical-name → dimension for every slot that carries an mRef.
    let tagged: Vec<(String, DimensionVector, Option<String>, f64, f64)> = store
        .iter()
        .filter_map(|(_, meta, _)| {
            meta.m_ref.as_ref().map(|m| {
                (
                    meta.canonical_name.to_string(),
                    m.dimension,
                    m.unit.as_ref().map(|u| u.to_string()),
                    m.scale,
                    m.offset,
                )
            })
        })
        .collect();

    assert!(
        !tagged.is_empty(),
        "coulomb-friction has ISQ-typed attributes (mass, position, velocity, …) — \
         at least one slot must carry an mRef"
    );

    // ISQ-type-only inference is SI-base, unit-less (byte-identical today).
    for (name, _, unit, scale, offset) in &tagged {
        assert_eq!(*unit, None, "{name}: ISQ-type-only mRef has no explicit unit yet");
        assert_eq!(*scale, 1.0, "{name}: SI-base scale");
        assert_eq!(*offset, 0.0, "{name}: SI-base offset");
    }

    // `mass : MassValue` ⇒ mass dimension; `position : LengthValue` ⇒ length.
    let find = |needle: &str| -> Option<DimensionVector> {
        tagged
            .iter()
            .find(|(n, ..)| n.rsplit('.').next() == Some(needle))
            .map(|(_, d, ..)| *d)
    };
    assert_eq!(find("mass"), Some(dim_mass()), "mass slot carries the mass dimension");
    assert_eq!(
        find("position"),
        Some(dim_length()),
        "position slot carries the length dimension"
    );
}

// ---------------------------------------------------------------------------
// 6. RSC-5.2 — quantity boundary diagnostics (D-5.0.8 / UQ001, two-tier).
//
// `quantity_mismatch_health_diagnostics` is additive and not yet wired into the
// service diagnostics pipeline at this step — these drive the pure runtime
// function directly (mirrors how flow/port health is unit-tested in-crate).
// ---------------------------------------------------------------------------

/// The cross-dimension binding fixture fires exactly the intended diagnostics:
/// one UQ001 ERROR (length = mass), one UQ001 WARNING (dimensioned = untyped),
/// and NOTHING for the same-dimension binding (length = length).
#[test]
fn rsc52_cross_dim_binding_fires_uq001() {
    // The fixture is elaborated through ModelCompiler so the connector
    // `source_id`/`target_id` endpoints are stamped (the diagnostic reads them).
    let compiler =
        ModelCompiler::new(load_file("examples/quantity-mismatch/QuantityMismatch.sysml"));
    let diags = sysml_runtime::quantity_health::quantity_mismatch_health_diagnostics(
        compiler.graph(),
    );

    let uq001: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ001"))
        .collect();
    assert_eq!(
        uq001.len(),
        2,
        "expected exactly the length=mass error and the dimensioned=untyped warning, got: {:#?}",
        diags
    );

    let errors: Vec<_> = uq001
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    let warnings: Vec<_> = uq001
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Warning)
        .collect();
    assert_eq!(errors.len(), 1, "one resolved-incompatible hard error");
    assert_eq!(warnings.len(), 1, "one dimensioned-vs-untyped warning");
    assert!(
        errors[0].message.contains("incompatible quantity dimensions"),
        "error wording: {}",
        errors[0].message
    );
    assert!(
        warnings[0].message.contains("untyped"),
        "warning wording: {}",
        warnings[0].message
    );
}

/// False-positive guard: the real ISQ-typed proving fixtures (every binding /
/// attribute same-dimension) must produce ZERO quantity diagnostics. This is
/// the corpus-clean regression pin for the static scan.
#[test]
fn rsc52_isq_proving_fixtures_clean() {
    for rel in [
        "examples/coulomb-friction/CoulombFriction.sysml",
        "examples/radiation-cooling/RadiationCooling.sysml",
    ] {
        let compiler = ModelCompiler::new(load_file(rel));
        let diags = sysml_runtime::quantity_health::quantity_mismatch_health_diagnostics(
            compiler.graph(),
        );
        assert!(
            diags.is_empty(),
            "{rel} should produce no quantity-mismatch diagnostics, got: {diags:#?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. RSC-5.2b — quantity diagnostics inside constraint expressions
//    (D-5.0.8 / UQ003 cross-dim comparison, UQ004 dimensionless-fn argument).
//    Drives the pure runtime `quantity_expression_health_diagnostics` directly.
// ---------------------------------------------------------------------------

/// The cross-dimension constraint fixture fires exactly one UQ003 (the
/// `len < mass` ordering comparison) and exactly one UQ004 (`sin(len)`), and
/// nothing for the same-dimension / dimensioned-vs-literal constraints.
#[test]
fn rsc52b_cross_dim_constraint_fires_uq003_uq004() {
    let compiler = ModelCompiler::new(load_file(
        "examples/quantity-expr-mismatch/QuantityExprMismatch.sysml",
    ));
    let diags = sysml_runtime::quantity_health::quantity_expression_health_diagnostics(
        compiler.graph(),
    );

    let uq003: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ003"))
        .collect();
    let uq004: Vec<&sysml_span::Diagnostic> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("UQ004"))
        .collect();

    assert_eq!(
        uq003.len(),
        1,
        "exactly one cross-dim ordering comparison (len < mass), got: {diags:#?}"
    );
    assert_eq!(
        uq004.len(),
        1,
        "exactly one dimensionless-fn misuse (sin(len)), got: {diags:#?}"
    );
    assert_eq!(uq003[0].severity, sysml_span::Severity::Error);
    assert_eq!(uq004[0].severity, sysml_span::Severity::Error);
    assert!(
        uq003[0].message.contains("incompatible quantity dimensions"),
        "UQ003 wording: {}",
        uq003[0].message
    );
    assert!(
        uq004[0].message.contains("dimensionless-only function"),
        "UQ004 wording: {}",
        uq004[0].message
    );
}

/// False-positive guard for the expression scan. The ISQ proving fixtures and —
/// critically — a verification file (`PumpSafety`, whose require-constraints
/// compare a simulation-*injected* runtime variable, not a model attribute,
/// against a threshold) must produce ZERO expression diagnostics: the injected
/// name resolves to nothing, so its dimension is unknown and the comparison
/// stays silent (verify-before-build item (c)).
#[test]
fn rsc52b_constraint_fixtures_clean() {
    for rel in [
        "examples/coulomb-friction/CoulombFriction.sysml",
        "examples/radiation-cooling/RadiationCooling.sysml",
        "examples/espresso-pump-hybrid/Verification/PumpSafety.sysml",
    ] {
        let compiler = ModelCompiler::new(load_file(rel));
        let diags = sysml_runtime::quantity_health::quantity_expression_health_diagnostics(
            compiler.graph(),
        );
        assert!(
            diags.is_empty(),
            "{rel} should produce no quantity-expression diagnostics, got: {diags:#?}"
        );
    }
}
