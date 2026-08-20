//! Phase 0 — Behavioral baseline for resolution-dependent LSP/service features.
//!
//! Captures the CURRENT runtime behavior of every resolution-dependent feature
//! against the coffee-machine fixture, regardless of what the static audit said
//! about the code. Asserts the INTENDED (correct) behavior, so tests pass once
//! the feature is fixed and fail today where the audit's "works" claim diverges
//! from real behavior. This file is the regression target for the
//!
//! Every test is `#[ignore]`-tagged so they stay opt-in. Run with:
//!
//!   cargo test --release -p sysml-service --test contract_resolution_features_baseline -- --ignored
//!
//! Lifts the audit's test_matrix into executable form. Each test maps to one
//! feature is fixed, drop the `#[ignore]` and the test guards against regression.
//!
//! Patterns followed (from `crates/tooling/sysml-service/CLAUDE.md`):
//! - `SysmlService::empty()` constructor.
//! - `service.open_context(OpenTarget::Folder(...))` for workspace loading
//!   (single source of truth — every transport routes through this).
//! - Service commands as the test boundary — CLI/LSP/MCP are thin shims so
//!   service-level parity = transport-level parity.

use std::path::PathBuf;

use sysml_project::discovery::OpenTarget;
use sysml_service::SysmlService;

// ---------------------------------------------------------------------------
// Fixture: coffee-machine workspace
// ---------------------------------------------------------------------------

/// Path to the coffee-machine book fixture under the workspace root.
///
/// `CARGO_MANIFEST_DIR` is `crates/tooling/sysml-service/`. The fixture lives
/// at `tests/fixtures/book-examples/coffee-machine/` under the workspace root,
/// so we walk up three directories.
fn coffee_machine_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/book-examples/coffee-machine")
        .canonicalize()
        .expect("coffee-machine fixture must exist")
}

fn coffee_machine_file(name: &str) -> String {
    coffee_machine_root()
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Load the coffee-machine workspace and return the service.
///
/// Uses `open_context(Folder)` — the canonical workspace-loading path — so all
/// per-file diagnostics, resolution graphs, and salsa-tracked queries are
/// keyed against the same `ProjectFileSet` that the LSP/MCP/REST transports
/// use in production.
fn load_coffee_machine() -> SysmlService {
    let svc = SysmlService::empty();
    let ctx = svc
        .open_context(OpenTarget::Folder(coffee_machine_root()))
        .expect("coffee-machine open_context should succeed");
    assert!(
        ctx.loaded_uris.len() > 1,
        "open_context should load multiple coffee-machine files, got {:?}",
        ctx.loaded_uris
    );
    svc
}

// ---------------------------------------------------------------------------
// Test matrix row 1 — Goto from bare cross-file usage to definition
// ---------------------------------------------------------------------------
// File: connections.sysml line 11 (`        end source : WaterPort;`)
// Cursor: on `WaterPort` (cross-file type, defined in ports-and-interfaces.sysml).
// Intent: target span lives in ports-and-interfaces.sysml at `port def WaterPort`.

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn goto_def_water_port_bare_cross_file_jumps_to_port_def() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // `end source : WaterPort;` — cursor on the `W` of WaterPort (line 11
    // 1-indexed → 10 LSP; column 21 0-indexed = first char of `WaterPort`).
    let target = svc
        .goto_definition(&uri, 10, 25)
        .expect("goto_definition should not error")
        .expect("cursor on WaterPort should yield a target");

    assert!(
        target.uri.ends_with("ports-and-interfaces.sysml"),
        "expected target in ports-and-interfaces.sysml, got uri={}",
        target.uri
    );
    // `port def WaterPort {` is around line 8 (0-indexed 7) of the file.
    assert!(
        target.line_start <= 10,
        "expected target near top of ports-and-interfaces.sysml (line ~7-8), got line_start={}",
        target.line_start
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 2 — Goto through FeatureTyping + Redefinition
// ---------------------------------------------------------------------------
// File: connections.sysml line 21
//   `        part waterTank : WaterTankWithPorts :>> CoffeeMachine::waterTank;`
//
// Three cursor positions probe three different relationship paths:
//   a) On `waterTank` (decl name) — should goto the redefined feature in
//      definitions.sysml (the `:>> CoffeeMachine::waterTank` target).
//   b) On `WaterTankWithPorts` (cross-file type) — should goto the
//      `part def WaterTankWithPorts` in ports-and-interfaces.sysml.
//   c) On `CoffeeMachine::waterTank` (qualified-name cross-file ref) — should
//      goto definitions.sysml at the `waterTank` member.
//
// These are the THREE positions the user reported as broken.

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn goto_def_water_tank_with_ports_cross_file_type() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // Line 21 (1-indexed) → 20 LSP. `WaterTankWithPorts` starts at col 25
    // (0-indexed). Cursor at col 30 lands mid-identifier.
    let target = svc
        .goto_definition(&uri, 20, 30)
        .expect("goto_definition should not error")
        .expect("cursor on WaterTankWithPorts should yield a target");

    assert!(
        target.uri.ends_with("ports-and-interfaces.sysml"),
        "expected target in ports-and-interfaces.sysml, got uri={}",
        target.uri
    );
}

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn goto_def_qualified_name_coffee_machine_water_tank() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // `CoffeeMachine::waterTank` starts at col 48 on line 21. Cursor at col 55
    // lands inside the `CoffeeMachine` segment (or near the `::`).
    let target = svc
        .goto_definition(&uri, 20, 55)
        .expect("goto_definition should not error")
        .expect("cursor on CoffeeMachine::waterTank should yield a target");

    // `waterTank` is a member of `CoffeeMachine` declared in definitions.sysml.
    assert!(
        target.uri.ends_with("definitions.sysml")
            || target.uri.ends_with("connections.sysml"),
        "expected target in definitions.sysml (or in-file if CoffeeMachine is local), got uri={}",
        target.uri
    );
}

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn goto_def_decl_name_water_tank_jumps_to_redefined_feature() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // `waterTank` (decl name) starts at col 13 on line 21.
    let target = svc
        .goto_definition(&uri, 20, 15)
        .expect("goto_definition should not error");

    // INTENT: declaration-name goto-def is ambiguous in spec (a feature's own
    // decl can either no-op or jump to its redefined parent). Either is
    // acceptable; what we DO NOT want is "jump to other unrelated `waterTank`
    // identifiers." If a target is returned, it must point at the redefined
    // member (`CoffeeMachine::waterTank` in definitions.sysml) — not, say,
    // `actions.sysml` or `requirements.sysml`.
    if let Some(t) = target {
        assert!(
            t.uri.ends_with("connections.sysml") || t.uri.ends_with("definitions.sysml"),
            "decl-name goto must not jump to unrelated `waterTank` in another file; got uri={}",
            t.uri
        );
    }
}

// ---------------------------------------------------------------------------
// Test matrix row 3 — References on a cross-file port def
// ---------------------------------------------------------------------------
// File: ports-and-interfaces.sysml line 8 (`port def WaterPort {`)
// Cursor: on `WaterPort` (definition site).
// Intent: refs = def itself + every `WaterPort` mention in connections.sysml
// and any other typing site. NOT every element named `WaterPort` regardless
// of resolved identity.

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn references_water_port_def_collects_cross_file_usages() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("ports-and-interfaces.sysml");

    // `port def WaterPort {` — `WaterPort` starts around col 13 on line 8.
    let hits = svc
        .references(&uri, 7, 16)
        .expect("references should not error");

    assert!(
        !hits.is_empty(),
        "references on WaterPort def must include the def itself at minimum"
    );

    let def_hits = hits.iter().filter(|h| h.is_def).count();
    let usage_hits = hits.iter().filter(|h| !h.is_def).count();

    // Exactly one def site (the `port def WaterPort` declaration).
    assert_eq!(
        def_hits, 1,
        "expected exactly one definition hit, got {def_hits} (every cross-file hit being is_def=true is a known references.rs bug)"
    );
    // At least one usage in connections.sysml (`end source : WaterPort;`,
    // `end sink : ~WaterPort;`, etc.).
    assert!(
        usage_hits >= 2,
        "expected ≥2 usage hits across files (connections.sysml uses WaterPort multiple times), got {usage_hits}"
    );

    let has_connections_use = hits
        .iter()
        .any(|h| h.uri.ends_with("connections.sysml") && !h.is_def);
    assert!(
        has_connections_use,
        "expected a usage hit in connections.sysml, hits: {hits:?}"
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 3.5 (anti-test) — References on `waterTank` decl-name
// MUST NOT return unrelated parts named `waterTank` in distant files.
// This is the user-reported bug: name-based matching pollutes results.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn references_water_tank_decl_does_not_match_unrelated_homonyms() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // Cursor on `waterTank` (decl name) on line 21.
    let hits = svc
        .references(&uri, 20, 15)
        .expect("references should not error");

    // INTENT: hits should be element-identity-based. The `waterTank` on
    // line 21 of connections.sysml is a NEW PartUsage that REDEFINES
    // CoffeeMachine::waterTank. References to THIS element are in-file only
    // (no cross-file usages exist of the ConnectedCoffeeMachine.waterTank
    // specifically). Returning every `waterTank` across the workspace is
    // wrong — those resolve to other elements.
    //
    // A correct impl returns either:
    //   - just this in-file decl + any in-file uses (`waterTank.waterOut`, etc.),
    //   - OR an empty list (since no other element identifies as THIS waterTank).
    //
    // What we DO NOT accept: hits in actions.sysml, requirements.sysml, or
    // any file where `waterTank` is a different PartUsage.
    let cross_file_hits: Vec<_> = hits
        .iter()
        .filter(|h| !h.uri.ends_with("connections.sysml"))
        .collect();

    assert!(
        cross_file_hits.is_empty(),
        "references on a decl-name must not return unrelated cross-file homonyms; got {} cross-file hits: {:?}",
        cross_file_hits.len(),
        cross_file_hits
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 4 — Hover on a cross-file port def usage
// ---------------------------------------------------------------------------

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn hover_on_cross_file_water_port_returns_port_def_signature() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // `WaterPort` on line 11 col ~25.
    let hover = svc
        .hover(&uri, 10, 25)
        .expect("hover should not error")
        .expect("hover on WaterPort must return content");

    let markdown = &hover.markdown;
    assert!(
        markdown.contains("WaterPort") || markdown.contains("port def"),
        "hover content should mention WaterPort or its kind, got: {markdown}"
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 4.5 — Hover on the typed-def cross-file ref
// ---------------------------------------------------------------------------

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn hover_on_water_tank_with_ports_resolves_through_typing() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("connections.sysml");

    // `WaterTankWithPorts` on line 21 col ~30.
    let hover = svc
        .hover(&uri, 20, 30)
        .expect("hover should not error")
        .expect("hover on WaterTankWithPorts must return content");

    assert!(
        hover.markdown.contains("WaterTankWithPorts") || hover.markdown.contains("part def"),
        "hover content should mention WaterTankWithPorts or part def, got: {}",
        hover.markdown
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 5 — Rename safety
// ---------------------------------------------------------------------------
// PrepareRename on the cross-file port-def name must succeed with a
// narrowly-scoped range (the identifier only).

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn prepare_rename_water_port_returns_identifier_range() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("ports-and-interfaces.sysml");

    // `port def WaterPort` on line 8.
    let resp = svc
        .rename(&uri, 7, 16, None)
        .expect("prepare_rename should not error");

    let prep = resp.prepare.expect("prepare_rename should return a range");
    assert_eq!(
        prep.placeholder, "WaterPort",
        "placeholder must be the identifier text, got {:?}",
        prep.placeholder
    );
}

// ---------------------------------------------------------------------------
// Test matrix row 5.5 — Rename SAFETY anti-test
// ---------------------------------------------------------------------------
// Renaming `WaterPort` must NOT clobber unrelated identifiers in other files.
// The audit flagged rename as "broken" because name-only matching causes both
// over-edit (homonyms) and under-edit (redefinition chains). This test locks
// in the over-edit safety boundary.

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn rename_water_port_edits_target_uses_but_not_unrelated_names() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("ports-and-interfaces.sysml");

    // Apply-rename: WaterPort → H2OPort.
    let resp = svc
        .rename(&uri, 7, 16, Some("H2OPort"))
        .expect("apply_rename should not error");

    let edit = resp.apply.expect("apply_rename should return a workspace edit");

    // Inspect the workspace edit shape. We don't make sharp per-file count
    // claims — those depend on the fixture's exact use sites — but we do
    // assert: (a) at least one edit lands in ports-and-interfaces.sysml,
    // (b) at least one lands in connections.sysml (since WaterPort is used
    // there), (c) NO edits land in requirements.sysml, calculations.sysml, or
    // metadata.sysml, where the `WaterPort` identifier does not appear.
    let edit_json = serde_json::to_value(&edit).expect("edit must be serializable");
    let s = edit_json.to_string();

    assert!(
        s.contains("ports-and-interfaces.sysml"),
        "workspace edit must touch the def file, edit: {s}"
    );
    assert!(
        s.contains("connections.sysml"),
        "workspace edit must touch connections.sysml (uses WaterPort), edit: {s}"
    );
    // Safety boundary — these files don't use `WaterPort`:
    for unrelated in ["requirements.sysml", "calculations.sysml", "metadata.sysml"] {
        assert!(
            !s.contains(unrelated),
            "workspace edit must NOT touch {unrelated} (doesn't reference WaterPort), edit: {s}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test matrix row 6 — Auto-import code action for unresolved cross-file name
// ---------------------------------------------------------------------------
// This is the audit's "broken" code_action_list claim: auto-import suggestions
// only see the library, not user-authored packages. We construct a scratch
// scenario by copying connections.sysml and removing the `import Ports::*;`
// line, then cursor on the now-unresolved `WaterPort`.

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn code_actions_offer_auto_import_for_unresolved_user_package_name() {
    use std::fs;
    use tempfile::TempDir;

    // Copy the coffee-machine workspace into a temp dir, strip the
    // `import Ports::*;` line from connections.sysml, then probe code actions
    // on the now-unresolved `WaterPort`.
    let dir = TempDir::new().expect("tempdir");
    let src = coffee_machine_root();
    for entry in fs::read_dir(&src).expect("read coffee-machine") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().expect("filename");
            let dst = dir.path().join(name);
            let content = fs::read_to_string(&path).expect("read");
            let stripped = if name == "connections.sysml" {
                content
                    .lines()
                    // Replace (don't drop) the `import Ports::*;` line so
                    // every other line keeps its original line number — the
                    // diag below points to fixed (line, col) coords.
                    .map(|l| {
                        if l.trim_start().starts_with("import Ports::") {
                            "    // import removed by test"
                        } else {
                            l
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                content
            };
            fs::write(&dst, stripped).expect("write");
        }
    }

    let svc = SysmlService::empty();
    let _ctx = svc
        .open_context(OpenTarget::Folder(dir.path().to_path_buf()))
        .expect("open scratch workspace");
    let uri = dir
        .path()
        .join("connections.sysml")
        .to_string_lossy()
        .into_owned();

    // Hand-construct the E200-style "unresolved name" diagnostic at the
    // `WaterPort` site. The LSP transport derives this shape from
    // `lsp_types::Diagnostic`; we build the equivalent service shape here so
    // the test is independent of the diagnostic-computation pipeline (which
    // is exercised separately by contract_strict_mode_diagnostics).
    // Match the production E200 message shape: name in single quotes.
    // `code_actions::extract_quoted_name` requires the single-quoted form to
    // recover the unresolved identifier without re-parsing the file.
    let diag_json = serde_json::json!([{
        "line_start": 10,
        "col_start": 21,
        "line_end": 10,
        "col_end": 30,
        "code": "E200",
        "message": "no definition 'WaterPort' found in scope",
    }]);

    // Cursor on `WaterPort` (line 11, 0-indexed line 10, col 25).
    let actions = svc
        .code_action_list(&uri, 10, 25, 10, 25, &diag_json)
        .expect("code_action_list should not error");

    let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();

    // INTENT: at least one action should suggest importing WaterPort from
    // Ports — either `import Ports::*;` or `import Ports::WaterPort;`.
    let has_auto_import = titles.iter().any(|t| {
        let lower = t.to_lowercase();
        lower.contains("import")
            && (lower.contains("ports") || lower.contains("waterport"))
    });

    assert!(
        has_auto_import,
        "expected an auto-import action for cross-file user-package name `WaterPort`; got titles: {titles:?}"
    );
}

// ---------------------------------------------------------------------------
// Single-file confidence test — Outline on definitions.sysml
// ---------------------------------------------------------------------------
// Outline is resolution-independent by spec, but included so the baseline
// surfaces a sanity green (proves the test harness wiring works against the
// coffee-machine fixture and the test isn't just universally red).

#[test]
#[ignore = "phase 0 baseline — opt-in via --ignored"]
fn outline_definitions_returns_part_definitions() {
    let svc = load_coffee_machine();
    let uri = coffee_machine_file("definitions.sysml");

    let outline = svc.outline(&uri).expect("outline should not error");
    let json = serde_json::to_value(&outline).expect("serialize outline");
    let s = json.to_string();

    // definitions.sysml contains `part def CoffeeMachine` plus member parts
    // like `WaterTank`, `Brewer`, etc.
    assert!(
        s.contains("CoffeeMachine"),
        "outline should mention CoffeeMachine, got: {s}"
    );
}
