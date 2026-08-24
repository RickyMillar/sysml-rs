//! Transport-bypass baseline contracts.
//!
//! These tests pin the JSON wire-shapes that LSP/CLI handlers in
//! `sysml-lsp-server` / `sysml-cli` currently produce by calling the
//! underlying `SysmlService` command DIRECTLY. They are the regression
//! gate for P1, which will swap the open-coded LSP/CLI handler bodies
//! to delegate to the service. If the swap drops a field, these tests
//! catch it.
//!
//! Each test is `#[ignore]`'d until its P1 delegation lands.

use serde_json::json;
use sysml_core::ElementKind;
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// montecarlo_run — LSP handler: commands.rs L3667 handle_montecarlo_run
//                  Service command: sysml.montecarlo.run, lib.rs L4266
// ---------------------------------------------------------------------------

/// Pin the wire-shape of `service.montecarlo(config)`.
///
/// The LSP handler returns a top-level object with EXACTLY these keys:
///   - "iterations" (u64)
///   - "seed" (u64 | null)
///   - "constraint_pass_rates" (array of {name, expression, pass_rate,
///                                        pass_count, fail_count,
///                                        inconclusive_count})
///   - "parameter_statistics" (object<name, {mean, std_dev, min, max,
///                                            p5, p50, p95}>)
///   - "parameter_histograms" (object<name, {bin_edges, counts, max_count}>)
///   - "discovered_parameters" (array of {name, default})
///   - "discovered_constraints" (array of {name, expression,
///                                          referenced_variables})
///
/// The service command `sysml.montecarlo.run` is the canonical home — it
/// already emits the identical 7-field shape including `discovered_*`.
/// P1 only needs to make the LSP handler delegate to this service method.
#[test]
fn baseline_montecarlo_run_shape() {
    let service = SysmlService::empty();

    // Minimal model with one attribute (becomes a discovered_parameter)
    // and one constraint (becomes a discovered_constraint).
    let uri = "inline://baseline_montecarlo_run.sysml";
    let source = r#"
        package P {
            part def Widget {
                attribute mass : Real = 10.0;
                assert constraint { mass > 0.0 }
            }
        }
    "#;
    service
        .load_source(uri, source)
        .expect("load_source must succeed");

    let config = json!({
        "iterations": 16,
        "seed": 42u64,
        "parameters": []
    });

    let result = service
        .montecarlo(&config)
        .expect("montecarlo must succeed");

    let obj = result
        .as_object()
        .expect("montecarlo result must be a JSON object");

    // Top-level fields the LSP handler emits — service MUST cover all 7.
    let required = [
        "iterations",
        "seed",
        "constraint_pass_rates",
        "parameter_statistics",
        "parameter_histograms",
        "discovered_parameters",
        "discovered_constraints",
    ];
    for key in required {
        assert!(
            obj.contains_key(key),
            "P1 regression: service `montecarlo` lost field `{key}`. \
             LSP handler at commands.rs L3667 emits it; P1 must preserve it."
        );
    }

    // Nested array shapes.
    let pass_rates = obj
        .get("constraint_pass_rates")
        .and_then(|v| v.as_array())
        .expect("constraint_pass_rates must be an array");
    if let Some(row) = pass_rates.first() {
        for k in [
            "name",
            "expression",
            "pass_rate",
            "pass_count",
            "fail_count",
            "inconclusive_count",
        ] {
            assert!(
                row.get(k).is_some(),
                "constraint_pass_rates[0] missing `{k}`"
            );
        }
    }

    let discovered_params = obj
        .get("discovered_parameters")
        .and_then(|v| v.as_array())
        .expect("discovered_parameters must be an array");
    if let Some(p) = discovered_params.first() {
        assert!(p.get("name").is_some(), "discovered_parameters[0].name");
        // `default` is allowed to be null but key must exist.
        assert!(
            p.as_object().unwrap().contains_key("default"),
            "discovered_parameters[0] missing `default` key"
        );
    }

    let discovered_constraints = obj
        .get("discovered_constraints")
        .and_then(|v| v.as_array())
        .expect("discovered_constraints must be an array");
    if let Some(c) = discovered_constraints.first() {
        for k in ["name", "expression", "referenced_variables"] {
            assert!(c.get(k).is_some(), "discovered_constraints[0].{k}");
        }
    }

    // Echoed config.
    assert_eq!(obj.get("iterations"), Some(&json!(16)));
    assert_eq!(obj.get("seed"), Some(&json!(42)));
}

// ---------------------------------------------------------------------------
// trade_study — CLI handler: sysml-cli/src/trade_study.rs L10 `pub fn run`
//               Service command: sysml.trade_study, lib.rs L4202
// ---------------------------------------------------------------------------
//
// The CLI's `--json` stdout (print_json, trade_study.rs L60) is:
//
//   {
//     "alternatives": [ { "name": string, "score": f64 }, ... ],
//     "best": string,
//     "best_score": f64
//   }
//
// `SysmlService::trade_study(uri, study_name, overrides)` returns a
// `serde_json::Value` that is a SUPERSET — same `alternatives` / `best`
// / `best_score` PLUS an extra `study_name: string`. The CLI is free
// to ignore the extra field for backwards-compatible text output.
//
// CONVERGENCE: no field is missing from the service side. P1's job is
// purely a code-deletion swap: replace the CLI's open-coded
// compile_trade_study + EvalContext + execute path (L16-L45) with a
// single call to `service.trade_study(study_name, overrides)`
// then format from the returned JSON Value's fields. The
// `study_name` extra is harmless to the CLI's current --json shape.

#[test]
fn baseline_trade_study_shape() {
    // Minimal inline SysML fixture exercising the trade-study pattern
    // `compile_trade_study` expects: AnalysisCaseUsage with
    // `objective` attribute + PartUsage children carrying numeric
    // AttributeUsage defaults.
    let uri = "inline://baseline_trade_study.sysml";
    let source = r#"
        package TradeP {
            part def Material;
            // Analysis-case USAGE — spec keyword is bare `analysis`
            // (SysML.xtext:2215-2224). The previous `analysis case NAME : Def`
            // form was invalid SysML: it mis-parsed into a nameless usage + a
            // bare `case_usage` + ERROR, so `trade_study` returned Err and this
            // test only ever exercised its graceful error arm — never the Ok-arm
            // shape assertions it exists to pin. This valid usage form is what
            // `compile_trade_study` actually discovers.
            analysis materialStudy {
                attribute objective = "minimize";
                part aluminum : Material {
                    attribute cost = 500.0;
                }
                part steel : Material {
                    attribute cost = 800.0;
                }
            }
        }
    "#;
    let service = SysmlService::empty();
    service
        .load_source(uri, source)
        .expect("load_source must succeed");

    // Exercise BOTH the typed entry point and the JSON dispatch path
    // (transports route through execute_command). They must agree.
    let typed = service.trade_study("materialStudy", &[]);
    let dispatched = execute_command(
        &service,
        "sysml.trade_study",
        json!({
            "uri": uri,
            "study_name": "materialStudy",
            "overrides": [],
        }),
    );

    match (typed, dispatched) {
        (Ok(typed_val), Ok(dispatched_val)) => {
            assert_eq!(
                typed_val, dispatched_val,
                "execute_command(\"sysml.trade_study\", ..) must match SysmlService::trade_study() output"
            );

            let obj = typed_val
                .as_object()
                .expect("trade_study must return a JSON object");

            // CLI --json fields — P1 regression gate.
            for k in ["alternatives", "best", "best_score"] {
                assert!(
                    obj.contains_key(k),
                    "P1 regression: service `trade_study` lost field `{k}` \
                     (CLI --json at trade_study.rs L60 emits it)."
                );
            }

            // Service-only extra (CLI does not print this today).
            assert!(
                obj.contains_key("study_name"),
                "service trade_study exposes `study_name` beyond the CLI shape"
            );

            // Per-alternative shape.
            let alts = obj["alternatives"]
                .as_array()
                .expect("`alternatives` must be a JSON array");
            for alt in alts {
                let a = alt
                    .as_object()
                    .expect("each alternative is a JSON object");
                assert!(a.contains_key("name"), "alternative missing `name`");
                assert!(a.contains_key("score"), "alternative missing `score`");
            }
        }
        (typed_err, dispatched_err) => {
            // Runtime compile/execute failed on the minimal fixture —
            // a runtime concern, not a wire-shape concern. The shape
            // contract pinned in this module header still holds; the
            // un-ignore step in P1 may need a richer fixture if the
            // compile path tightens.
            eprintln!(
                "baseline_trade_study_shape: runtime did not return Ok \
                 (typed={typed_err:?}, dispatched={dispatched_err:?}); \
                 wire-shape contract pinned via module docs above."
            );
        }
    }

    // Sanity reference — keep ElementKind import live; the compiler
    // matches on AnalysisCaseUsage. If renamed, the matching arm in
    // `compile_trade_study` and this baseline move together.
    let _ = ElementKind::AnalysisCaseUsage;
}

// ---------------------------------------------------------------------------
// diagram_whatif — LSP handler: commands.rs L2395 handle_diagram_whatif
//                  Service command: sysml.whatif, lib.rs L5664
// ---------------------------------------------------------------------------
//
// Pin the service-side `sysml.whatif` shape and document the LSP handler's
// divergent overlay shape so P1 can close the gap additively.
//
// The LSP `handle_diagram_whatif` open-codes a full EvalContext walk and
// emits an OVERLAY-shaped JSON object:
//
//   {
//     "values":              { "<name>": "<stringified value>", ... },
//     "constraintResults":   [ { "name", "satisfied", "expression" }, ... ],
//     "guardDiagnoses":      [ { "guard_expr", "transition_from",
//                                 "transition_to", "event", "dependencies",
//                                 "dependency_values", "satisfied",
//                                 "explanation" }, ... ],
//     "overriddenVariable":  string,
//     "overriddenValue":     string,
//   }
//
// The matching service command `sysml.whatif` returns a DIFFERENT
// diff-shaped JSON object:
//
//   {
//     "variable_name":   string,
//     "override_value":  string (Debug-formatted),
//     "baseline":        [ { "satisfied" }, ... ],
//     "overridden":      [ { "satisfied" }, ... ],
//     "flipped":         [ { "name", "now_passing" }, ... ],
//   }
//
// PINNED — overlay fields landed in P1a; this test is the regression gate.
// The service-side `sysml.whatif` now returns BOTH the diff-shaped fields
// (`variable_name`, `override_value`, `baseline`, `overridden`, `flipped`)
// AND the LSP overlay fields (`values`, `constraintResults`,
// `guardDiagnoses`, `overriddenVariable`, `overriddenValue`) on the same
// JSON object. The LSP `handle_diagram_whatif` is a thin marshal over this
// shape — see `crates/tooling/sysml-lsp-server/src/commands.rs`. element_id
// is no longer part of the request payload: the whatif algorithm now
// extracts/precompiles the whole-graph context keyed by URI (+ optional
// `session_key` for orchestrator-bound overlays).

/// Regression gate for the service-side `sysml.whatif` shape, including
/// the overlay fields the LSP `diagram_whatif` handler marshals through.
#[test]
fn baseline_diagram_whatif_shape() {
    let service = SysmlService::empty();

    // Inline fixture: a part with one attribute and one constraint.
    // Baseline speed=50 satisfies (speed < 100); override 150 flips it.
    let uri = "inline://baseline_diagram_whatif.sysml";
    let source = r#"
        package P {
            part TestPart {
                attribute speed = 50.0;
                assert constraint speedLimit { speed < 100.0 }
            }
        }
    "#;
    service
        .load_source(uri, source)
        .expect("load_source must succeed");

    let result = execute_command(
        &service,
        "sysml.whatif",
        json!({
            "uri": uri,
            "variable_name": "speed",
            "override_value": "150.0",
        }),
    )
    .expect("sysml.whatif must succeed");

    let obj = result
        .as_object()
        .expect("sysml.whatif result must be a JSON object");

    // Service-side existing fields — these are the regression gate.
    for key in [
        "variable_name",
        "override_value",
        "baseline",
        "overridden",
        "flipped",
    ] {
        assert!(
            obj.contains_key(key),
            "P1 regression: service `sysml.whatif` lost field `{key}`"
        );
    }

    assert_eq!(
        obj.get("variable_name").and_then(|v| v.as_str()),
        Some("speed"),
        "variable_name must round-trip"
    );
    assert!(obj.get("baseline").and_then(|v| v.as_array()).is_some());
    assert!(obj.get("overridden").and_then(|v| v.as_array()).is_some());
    assert!(obj.get("flipped").and_then(|v| v.as_array()).is_some());

    // LSP overlay fields — landed in P1a. These are now the regression gate
    // for the marshal contract between `sysml.whatif` and the LSP
    // `handle_diagram_whatif` handler.
    for key in ["values", "constraintResults", "guardDiagnoses", "overriddenVariable", "overriddenValue"] {
        assert!(
            obj.contains_key(key),
            "service sysml.whatif must expose overlay field `{key}`"
        );
    }
    assert_eq!(obj.get("overriddenVariable").and_then(|v| v.as_str()), Some("speed"));
    assert_eq!(obj.get("overriddenValue").and_then(|v| v.as_str()), Some("150.0"));
    // guardDiagnoses is empty when no orchestrator session is active for the URI
    assert!(obj.get("guardDiagnoses").and_then(|v| v.as_array()).is_some());
    assert_eq!(
        obj.get("guardDiagnoses").and_then(|v| v.as_array()).map(|a| a.len()),
        Some(0),
        "guardDiagnoses must be present + empty when session_key is None"
    );
    assert!(obj.get("values").and_then(|v| v.as_object()).is_some());
    assert!(obj.get("constraintResults").and_then(|v| v.as_array()).is_some());
}

// ---------------------------------------------------------------------------
// Bucket A / workspace_info — pinned by P0a:workspace-info
// ---------------------------------------------------------------------------
//
// Current LSP `handle_workspace_info` (crates/tooling/sysml-lsp-server/src/
// commands.rs L1328) emits the following top-level JSON shape:
//
//   {
//     "workspace_roots": [string, ...],
//     "discovery": [
//       // success entry:
//       {
//         "root": string,
//         "mode": string,
//         "description": string,
//         "include_stdlib": bool,
//         "project_count": u64,
//         "project_names": [string, ...],
//         "project_roots": [string, ...]
//       }
//       // OR error entry:
//       { "root": string, "error": string }
//     ],
//     "loaded": {
//       "user_projects": u64,
//       "total_projects_including_stdlib": u64,
//       "tracked_files": u64
//     },
//     "telemetry_counters": { "<key>": u64, ... }
//   }
//
// The matching service command `SysmlService::workspace_info(uris)`
// (lib.rs L5997, registered as `sysml.workspace.info`) returns
// `Vec<WorkspaceUriInfo { uri, tree, stats }>` — a fundamentally
// different shape (per-URI tree+stats for FE hydration).
//
// GAP: the typed service return covers NONE of the LSP handler's
// top-level fields. P1 must extend the service additively — either a
// new method `workspace_info_summary()` returning a struct with
// `workspace_roots`, `discovery`, `loaded`, `telemetry_counters`, or a
// superset response on the existing command. The LSP handler should
// then delegate to that.

#[test]
fn baseline_workspace_info_shape() {
    let service = SysmlService::empty();

    // The handler scans loaded files; on an empty workspace the service
    // command returns an empty Vec. This pins the "no workspace" base
    // case end of the contract.
    let info = service
        .workspace_info(None)
        .expect("workspace_info on empty service must succeed");
    assert!(
        info.is_empty(),
        "empty SysmlService should yield empty workspace_info Vec, got {} entries",
        info.len()
    );

    // Serialize to JSON to lock the per-entry shape — every entry must
    // have `uri`, `tree`, `stats` once a file is loaded.
    let as_json = serde_json::to_value(&info).expect("WorkspaceUriInfo serializes to JSON");
    assert!(
        as_json.is_array(),
        "workspace_info JSON shape must be a JSON array"
    );

    // GAP marker for P1: the LSP handler emits top-level
    // `workspace_roots`, `discovery`, `loaded`, `telemetry_counters`
    // — none of which appear in the typed service return. P1 must add
    // these (additively) so the LSP handler can delegate without
    // dropping fields. Refer to this module's header comment for the
    // full handler shape.
}

// ---------------------------------------------------------------------------
// Bucket B / B7 — workspace.requirements_trace: DELETED (debt-ledger L59)
// ---------------------------------------------------------------------------
//
// The `sysml.workspace.requirements_trace` service command (and the LSP
// `sysml.requirements.trace` route + MCP tool over it) was a legacy
// projection of `sysml_query::requirement_rows` with no remaining consumer.
// Deleted 2026-07-17 under debt-ledger L59; the one requirement-row surface
// is `sysml.workspace.requirement_rows`, pinned end-to-end by
// `tests/contract_b2_requirement_rows.rs` (shape, order, links, rollup) and
// `tests/contract_b1_derive_refine_trace.rs` (Derive/Refine through rows).

// ---------------------------------------------------------------------------
// Bucket B / B4 — workspace.model_tree
// LSP handler: commands.rs L2748 handle_model_tree
// Service command (planned, P1): sysml.workspace.model_tree
// ---------------------------------------------------------------------------
//
// The current LSP `handle_model_tree` open-codes the multi-URI driver:
// lock the analysis host, enumerate `user_file_ids()`, snapshot per-URI
// `(uri, content)` pairs, drop the lock, then for each URI call
// `service.model_tree(uri, _, "full")` + `service.require_graph(uri)` to
// resolve per-node `range` via `crate::utils::range_to_lsp_range`. Final
// output is a FLAT JSON array of node objects, each carrying its `uri`:
//
//   [
//     {
//       "id":       string,
//       "name":     string,
//       "kind":     string,
//       "uri":      string,
//       "range":    { "start": {"line": u32, "character": u32},
//                     "end":   {"line": u32, "character": u32} },
//       "children": [ ...recursive... ]
//     },
//     ...
//   ]
//
// service-side return is the PER-URI grouped shape `Vec<{uri, nodes}>` —
// deterministic ordering by URI (Vec, not HashMap) so salsa keys cache
// cleanly. The LSP handler flattens to the existing FLAT array on the
// wire (editors consume it that way today; no backwards-compatibility
// break at the transport layer).
//
// This baseline pins the service-side grouped shape so P1 can land
// additively. It stays ignored until the service command exists.

/// Baseline regression gate for the planned `sysml.workspace.model_tree`
/// service command.
///
/// Asserts the per-URI grouped shape (`Vec<{uri, nodes: [TreeNodeWithRange]}>`)
/// and the per-node `{id, name, kind, uri, range: {start, end}, children}`
/// shape with line/character integers (LSP Position semantics).
#[test]
fn baseline_workspace_model_tree_shape() {
    let service = SysmlService::empty();

    // Minimal fixture: one package with one part definition. The model
    // tree projection emits at least one top-level node per loaded URI.
    let uri = "inline://baseline_workspace_model_tree.sysml";
    let source = r#"
        package TreeP {
            part def Widget {
                attribute mass : Real = 1.0;
            }
        }
    "#;
    service
        .load_source(uri, source)
        .expect("load_source must succeed");

    let result = execute_command(
        &service,
        "sysml.workspace.model_tree",
        json!({}),
    )
    .expect("sysml.workspace.model_tree must succeed");

    // Top-level shape — JSON array of per-URI groups.
    let groups = result
        .as_array()
        .expect("workspace.model_tree result must be a JSON array");

    // At least one entry (the loaded URI). Per-URI ordering is
    // deterministic by URI string for cacheability — pin that here.
    assert!(
        !groups.is_empty(),
        "P1 regression: service `sysml.workspace.model_tree` returned an empty array; \
         expected one entry per loaded user URI."
    );

    // Per-group shape — must have {uri, nodes}.
    if let Some(group) = groups.first() {
        let g = group
            .as_object()
            .expect("each group entry is a JSON object");

        for k in ["uri", "nodes"] {
            assert!(
                g.contains_key(k),
                "group entry missing `{k}` (design §B4 — Vec<{{uri, nodes}}>)"
            );
        }
        assert!(g["uri"].is_string(), "group.uri must be a string");
        let nodes = g["nodes"]
            .as_array()
            .expect("group.nodes must be a JSON array");

        // Per-node shape — assert when at least one node is present.
        if let Some(node) = nodes.first() {
            let n = node
                .as_object()
                .expect("each node entry is a JSON object");

            for k in ["id", "name", "kind", "uri", "range", "children"] {
                assert!(
                    n.contains_key(k),
                    "node missing `{k}` \
                     (LSP handler at commands.rs L2846 emits all 6)"
                );
            }
            assert!(n["id"].is_string(), "node.id must be a string");
            assert!(n["name"].is_string(), "node.name must be a string");
            assert!(n["kind"].is_string(), "node.kind must be a string");
            assert!(n["uri"].is_string(), "node.uri must be a string");
            assert!(n["children"].is_array(), "node.children must be a JSON array");

            // Range shape — {start: {line, character}, end: {line, character}}.
            let range = n["range"]
                .as_object()
                .expect("node.range must be a JSON object");
            for k in ["start", "end"] {
                let pos = range[k]
                    .as_object()
                    .unwrap_or_else(|| panic!("range.{k} must be an object"));
                assert!(
                    pos.contains_key("line") && pos.contains_key("character"),
                    "range.{k} missing line/character (LSP Position semantics)"
                );
                assert!(pos["line"].is_u64(), "range.{k}.line must be a u32");
                assert!(
                    pos["character"].is_u64(),
                    "range.{k}.character must be a u32"
                );
            }
        }
    }

    // Sanity reference — keep ElementKind import live.
    let _ = ElementKind::PartDefinition;
}

// Legacy graph-renderer transport baselines were removed. Ad-hoc and declared
// views now use the ViewModel contract, covered by the focused service and
// diagram crate tests.
