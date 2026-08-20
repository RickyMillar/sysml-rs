//! Workspace-tier id-stability pin (ADR-009 / workspace-scope plan §W6b
//! follow-up).
//!
//! With the content-true `ModelGraph::fingerprint` (ids included), the
//! ElaboratedWorkspace tier only benefits from salsa backdating if
//! elaborating identical input yields identical ids — one fresh-UUID
//! mint on the workspace elaboration path (implicit-generalization
//! memberships, typing mirrors, …) makes every re-elaboration compare
//! unequal and cascades recomputes through every downstream query.
//!
//! This pins the STDLIB-backed workspace path (the library enables
//! implicit generalization against base types — the exact family of
//! mints the library-less per-file probe in
//! `sysml-ide-db/tests/id_stability.rs` cannot reach). If stdlib is
//! unavailable in the environment, the elaboration still runs library-
//! less and the pin still holds for that shape.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use sysml_service::SysmlService;
use tempfile::TempDir;

const MODEL: &str = r#"
package Fixture {
    part def Vehicle {
        attribute mass : ScalarValues::Real = 1500.0;
    }
    part car : Vehicle {
        doc /* the family car */
    }
    requirement def R1 {
        doc /* must carry five people */
        require constraint { car.mass < 2000.0 }
    }
}
"#;

/// Two independent services loading the same workspace produce
/// fingerprint-identical elaborated workspace graphs.
#[test]
fn workspace_elaboration_is_id_stable_across_services() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("fixture.sysml"), MODEL).unwrap();

    let fp = |_: ()| {
        let service = SysmlService::empty();
        service.load_workspace(dir.path()).unwrap();
        service
            .workspace_aware_graph()
            .expect("workspace graph")
            .fingerprint()
    };
    let fp1 = fp(());
    let fp2 = fp(());
    assert_eq!(
        fp1, fp2,
        "workspace elaboration of identical input must produce identical \
         ids/content — a fresh-UUID mint on this path defeats salsa \
         backdating for every downstream query"
    );
}

/// Same-service edit round-trip: A → B → back to A must reproduce A's
/// fingerprint exactly. This is the in-db shape salsa backdating
/// actually exercises (re-elaboration after a real input change).
#[test]
fn workspace_elaboration_round_trip_reproduces_fingerprint() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("fixture.sysml");
    fs::write(&file, MODEL).unwrap();

    let service = SysmlService::empty();
    service.load_workspace(dir.path()).unwrap();
    let fp_a1 = service.workspace_aware_graph().unwrap().fingerprint();

    fs::write(&file, MODEL.replace("family car", "company car")).unwrap();
    service.load_workspace(dir.path()).unwrap();
    let fp_b = service.workspace_aware_graph().unwrap().fingerprint();
    assert_ne!(fp_a1, fp_b, "doc edit must change the fingerprint");

    fs::write(&file, MODEL).unwrap();
    service.load_workspace(dir.path()).unwrap();
    let fp_a2 = service.workspace_aware_graph().unwrap().fingerprint();
    assert_eq!(
        fp_a1, fp_a2,
        "restoring the original text must reproduce the original \
         fingerprint exactly (deterministic ids + content)"
    );
}
