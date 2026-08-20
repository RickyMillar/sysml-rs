//! VVS — Verification verdict spec-conformance harness.
//!
//! Sibling to `constraint_spec_conformance.rs` (same convention). This file
//! gates the **STABLE, PURE-FUNCTION** verdict semantics of verification cases:
//! the `VerdictKind` enumeration, its worst-wins `aggregate`, `is_pass`, and the
//! lowercase `Display` rendering. These are pure functions on a `Copy` enum with
//! no I/O, no graph, no context — they cannot churn.
//!
//! Each test encodes ONE spec-defined obligation and asserts the engine's
//! CURRENT behavior against it, carrying a verdict marker on its own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — asserts what the engine ACTUALLY does
//!   today, which differs from the spec obligation.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — pins the absence of a surface.
//!
//! Each test names the obligation it gates with an `// OBL:` line whose id
//! matches the obligation tracker at
//! `crates/testing/sysml-spec-tests/spec-obligations/verification-analysis-cases.md`.
//! That tracker is the authority for spec citations; this file is the gate.
//!
//! Spec sources (cited per obligation in the tracker):
//! - SysML §7.24.1 *Verification cases* — verdict meanings, incl.
//!   *"Inconclusive indicates that a determination could not be made…"*
//!   (`SysML-spec-r2025-04_REF.html`).
//! - `sysml.library/Systems Library/VerificationCases.sysml:58-68`
//!   (`enum def VerdictKind { pass; fail; inconclusive; error; }`).
//!
//! ## OUT OF SCOPE HERE (deliberately not gated by this file)
//!
//! The verification-case EVALUATION pipeline — subject binding
//! (`cases/mod.rs::collect_subject_bindings`), objective/verified-requirement
//! discovery, simulation-coupled binding (`verify_with_simulation`), and the
//! `cases_pipeline` integration surface — is under ACTIVE construction
//! (Inc1/Inc2 of the verification-case build) and would churn if gated now.
//! Those obligations (`verification-subject-bound-to-objective-subject`,
//! `verified-requirements-derivation`,
//! `verification-case-result-is-verdict` as an end-to-end pipeline assertion,
//! and the sim-derived layer-3 bridge) are DEFERRED to the in-flight build per
//! the tracker. This file touches ONLY the stable pure-function verdict layer.
//!
//! The summary test (`vvs_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use sysml_runtime::cases::VerdictKind;

// ===========================================================================
// OBL — verdict-kind-enumeration
// `VerdictKind` has exactly pass / fail / inconclusive / error, and its
// rendered (Display) names are the lowercase spec identifiers.
// Spec: VerificationCases.sysml:58-68 `enum def VerdictKind { pass; fail;
// inconclusive; error; }` (LIBRARY = normative semantics).
// ===========================================================================

#[test]
fn verdict_kind_has_all_four_variants() {
    // OBL: verdict-kind-enumeration
    // VERDICT: CONFORMS
    // All four spec variants exist and are distinct.
    let all = [
        VerdictKind::Pass,
        VerdictKind::Fail,
        VerdictKind::Inconclusive,
        VerdictKind::Error,
    ];
    // Distinctness: no two variants compare equal.
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            assert_eq!(a == b, i == j, "variant equality must be identity-only");
        }
    }
}

#[test]
fn verdict_kind_display_uses_lowercase_spec_names() {
    // OBL: verdict-kind-enumeration
    // VERDICT: CONFORMS
    // Display strings exactly match the library enum identifiers.
    assert_eq!(VerdictKind::Pass.to_string(), "pass");
    assert_eq!(VerdictKind::Fail.to_string(), "fail");
    assert_eq!(VerdictKind::Inconclusive.to_string(), "inconclusive");
    assert_eq!(VerdictKind::Error.to_string(), "error");
}

#[test]
fn verdict_kind_is_pass_only_for_pass() {
    // OBL: verdict-kind-enumeration
    // VERDICT: CONFORMS
    // `is_pass()` distinguishes the single passing verdict from every other.
    assert!(VerdictKind::Pass.is_pass());
    assert!(!VerdictKind::Fail.is_pass());
    assert!(!VerdictKind::Inconclusive.is_pass());
    assert!(!VerdictKind::Error.is_pass());
}

// ===========================================================================
// OBL — verdict-semantics  (aggregate = worst-wins)
// Aggregation of two verdicts yields the more severe one, with priority
// Error > Fail > Inconclusive > Pass. This encodes the spec verdict meanings:
// any non-pass dominates a pass, an undetermined result (Inconclusive,
// §7.24.1 "a determination could not be made") dominates a pass but yields to
// a decided Fail, and an Error (a fault during verification) dominates all.
// Spec: §7.24.1 verdict meanings (GOSPEL).
// ===========================================================================

#[test]
fn aggregate_pass_with_fail_is_fail() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // A single failure dominates a pass (order-independent).
    assert_eq!(VerdictKind::Pass.aggregate(VerdictKind::Fail), VerdictKind::Fail);
    assert_eq!(VerdictKind::Fail.aggregate(VerdictKind::Pass), VerdictKind::Fail);
}

#[test]
fn aggregate_fail_with_error_is_error() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // An error (fault during verification) dominates even a decided failure.
    assert_eq!(VerdictKind::Fail.aggregate(VerdictKind::Error), VerdictKind::Error);
    assert_eq!(VerdictKind::Error.aggregate(VerdictKind::Fail), VerdictKind::Error);
}

#[test]
fn aggregate_pass_with_inconclusive_is_inconclusive() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // An undetermined result dominates a pass: "could not determine" is not a pass.
    assert_eq!(
        VerdictKind::Pass.aggregate(VerdictKind::Inconclusive),
        VerdictKind::Inconclusive
    );
    assert_eq!(
        VerdictKind::Inconclusive.aggregate(VerdictKind::Pass),
        VerdictKind::Inconclusive
    );
}

#[test]
fn aggregate_inconclusive_with_fail_is_fail() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // A decided failure dominates an undetermined result (Fail > Inconclusive).
    assert_eq!(
        VerdictKind::Inconclusive.aggregate(VerdictKind::Fail),
        VerdictKind::Fail
    );
    assert_eq!(
        VerdictKind::Fail.aggregate(VerdictKind::Inconclusive),
        VerdictKind::Fail
    );
}

#[test]
fn aggregate_pass_with_pass_is_pass() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // Only all-pass aggregates to pass — the sole way to reach the passing verdict.
    assert_eq!(VerdictKind::Pass.aggregate(VerdictKind::Pass), VerdictKind::Pass);
}

#[test]
fn aggregate_full_priority_order_error_fail_inconclusive_pass() {
    // OBL: verdict-semantics
    // VERDICT: CONFORMS
    // Exhaustive worst-wins lattice check: Error > Fail > Inconclusive > Pass,
    // verified commutatively across every ordered pair.
    use VerdictKind::*;
    // Severity rank: higher = more severe (dominates in aggregate).
    let rank = |v: VerdictKind| match v {
        Pass => 0,
        Inconclusive => 1,
        Fail => 2,
        Error => 3,
    };
    let all = [Pass, Inconclusive, Fail, Error];
    for &a in &all {
        for &b in &all {
            let expected = if rank(a) >= rank(b) { a } else { b };
            assert_eq!(
                a.aggregate(b),
                expected,
                "aggregate({a}, {b}) must be the more-severe verdict"
            );
            // Commutativity: order of arguments never changes the result.
            assert_eq!(a.aggregate(b), b.aggregate(a), "aggregate must be commutative");
        }
    }
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn vvs_matrix_summary() {
    let src = include_str!("verification_verdict_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    println!(
        "VVS verification-verdict matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED \
         (case-evaluation/subject-binding/sim pipeline deferred to in-flight build)",
        verdicts.len()
    );
    assert!(
        verdicts.len() >= 8,
        "expected ≥8 verdict-marked gates in the verification-verdict pilot"
    );
}
