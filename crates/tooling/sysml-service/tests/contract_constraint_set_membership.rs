//! Contract gates for **session constraint-set membership and verdicts**.
//!
//! Purpose-built spec-faithful fixtures (NOT a corpus no-regression check).
//! They pin the two obligations that a session's constraint sweep was
//! violating — it swept 52 constraints for the espresso cell, 48 of them
//! reported `false`, and every one of those 48 was either a library
//! invariant or an uninstantiated `constraint def` evaluated against
//! unbound formals.
//!
//! ## 1. Membership — usages of the subject model, not imported types
//!
//! A `ConstraintDefinition` is a *type*, not an assertion. Per the SysML v2
//! vocabulary it is an `OccurrenceDefinition`/`Predicate` that
//!
//! > "defines a constraint that **may be asserted** to hold on a system or
//! > part of a system"
//! > — `references/sysmlv2/SysML-vocab.ttl:247`
//!
//! whereas a `ConstraintUsage` "is an `OccurrenceUsage` that is also a
//! `BooleanExpression`" (`SysML-vocab.ttl:254`) — the thing that is actually
//! evaluated. As a `Predicate` a definition is a kind of Behavior whose `in`
//! parameters are *formal* and unbound until something invokes it
//! (KerML §8.4.4.8.1); the standard library encodes the same asymmetry, with
//! `constraintChecks` declared as "the base feature of all ConstraintUsages"
//! (`sysml.library/Systems Library/Constraints.sysml:13-28`).
//!
//! And importing a package does not adopt its invariants. An `Import`
//!
//! > "is a Relationship ... which determines a set of Memberships that become
//! > importedMemberships of the importOwningNamespace"
//! > — `references/sysmlv2/Kerml-Vocab.ttl:231`
//!
//! — a name-visibility mechanism, silent on instantiation and obligation.
//!
//! ## 2. Verdicts — undecidable is `inconclusive`, never `fail`
//!
//! KerML has no three-valued Boolean and no coerce-to-false rule for a
//! missing binding. Its model of "no result" is the *empty* result:
//!
//! > "The result parameter of `NullEvaluation` has multiplicity 0..0, which
//! > means that a `NullExpression` always produces an empty result."
//! > — KerML §8.4.4.9.1, Null Expressions
//!
//! So a constraint the run could not evaluate must surface as
//! `VerdictKind::Inconclusive` (the standard library's own four-way split,
//! `sysml.library/Systems Library/VerificationCases.sysml:58-68`). Reporting
//! `fail` claims the model was checked and violated — a claim the run never
//! established.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;

use sysml_runtime::cases::VerdictKind;
use sysml_runtime::orchestrator::ConstraintEvalResult;
use sysml_service::SysmlService;
use tempfile::TempDir;

/// A model that (a) imports stdlib packages carrying their own
/// `assert constraint` invariants, (b) declares a `constraint def` it never
/// instantiates, and (c) declares constraint *usages* of its own — one
/// decidable-and-true, one decidable-and-false, one whose operand is never
/// bound.
///
/// `SpatialItems` / `MeasurementReferences` are the packages whose
/// invariants (`originPointConstraint`, `validOriginDimensions`,
/// `validateBasisDirections`, …) were being swept into sessions.
const FIXTURE: &str = r#"package ConstraintMembershipFixture {
    private import ScalarValues::*;
    private import Geometry::SpatialItems::*;
    private import Quantities::*;
    private import MeasurementReferences::*;

    // An uninstantiated constraint *definition*: a type with formal `in`
    // parameters. Nothing in this model uses it, so it asserts nothing.
    constraint def NeverInstantiated {
        in lo : Real;
        in hi : Real;
        lo <= hi
    }

    // A minimal subsystem so the workspace is orchestratable at all — the
    // session needs something to tick; it is not part of what is asserted.
    state def Idle {
        entry; then running;
        state running;
    }

    part def Widget {
        attribute pressure : Real = 12.0;

        // Decidable and true.
        assert constraint pressureInRange {
            pressure >= 9.0 and pressure <= 16.0
        }

        // Decidable and false — a genuine violation.
        assert constraint pressureImpossible {
            pressure >= 100.0
        }

        // Undecidable: `unboundOperand` is never bound anywhere in the model.
        assert constraint dependsOnUnbound {
            unboundOperand <= 1.0
        }
    }

    part widget : Widget;
}
"#;

fn session_constraints(content: &str) -> (TempDir, Vec<ConstraintEvalResult>) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("Fixture.sysml"), content).expect("write fixture");

    let service = SysmlService::empty();
    service
        .load_workspace(dir.path())
        .expect("load fixture workspace");

    let (_id, snapshot) = service
        .orchestrate_workspace_start("__workspace__", None, None, None)
        .expect("orchestrate workspace");

    (dir, snapshot.constraint_results)
}

fn verdict_of<'a>(rows: &'a [ConstraintEvalResult], name: &str) -> Option<&'a ConstraintEvalResult> {
    rows.iter().find(|r| r.name == name)
}

/// **Membership, library half.** A model that imports library packages does
/// not adopt those packages' invariants. Import is name visibility
/// (`Kerml-Vocab.ttl:231`), not obligation — so no stdlib constraint may
/// appear in the session's constraint set, however many the imports make
/// visible.
#[test]
fn imported_library_constraints_are_not_in_the_session_constraint_set() {
    let (_dir, rows) = session_constraints(FIXTURE);

    // The exact stdlib invariants that were being swept in, by name.
    for leaked in [
        "originPointConstraint",
        "validOriginDimensions",
        "validSourceDimensions",
        "validSourceTargetDimensions",
        "validateBasisDirections",
        "orderSum",
        "boundMatch",
    ] {
        assert!(
            verdict_of(&rows, leaked).is_none(),
            "library invariant `{leaked}` must not be in the session constraint set; \
             importing a package makes its members visible, not asserted (Kerml-Vocab.ttl:231). \
             Set was: {:?}",
            rows.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }
}

/// **Membership, definition half.** A `constraint def` is a type whose `in`
/// parameters are formal and unbound (`SysML-vocab.ttl:247`, KerML
/// §8.4.4.8.1). It is checkable only through a usage that binds them, so an
/// uninstantiated definition contributes no row — while the model's own
/// usages all survive.
#[test]
fn uninstantiated_constraint_definition_is_excluded_but_usages_survive() {
    let (_dir, rows) = session_constraints(FIXTURE);

    assert!(
        verdict_of(&rows, "NeverInstantiated").is_none(),
        "an uninstantiated `constraint def` is a type, not an assertion — it must not \
         produce a verdict row. Set was: {:?}",
        rows.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // The real cell constraints must survive the filter — the point of the
    // sweep is to check the model's own usages, and all three are present.
    for kept in ["pressureInRange", "pressureImpossible", "dependsOnUnbound"] {
        assert!(
            verdict_of(&rows, kept).is_some(),
            "the model's own constraint usage `{kept}` must be in the session constraint \
             set. Set was: {:?}",
            rows.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }
}

/// **Verdicts.** The three usages exercise the three outcomes, and the
/// undecidable one is the load-bearing case: an unbound operand yields
/// `Inconclusive`, never `Fail`. `Fail` is reserved for a constraint the run
/// actually evaluated and found violated — KerML offers no coercion of a
/// missing binding to `false` (§8.4.4.9.1).
#[test]
fn unbound_constraint_is_inconclusive_not_fail() {
    let (_dir, rows) = session_constraints(FIXTURE);

    let unbound = verdict_of(&rows, "dependsOnUnbound").expect("dependsOnUnbound present");
    assert_eq!(
        unbound.verdict,
        VerdictKind::Inconclusive,
        "a constraint over an unbound operand was not evaluated, so it cannot be reported \
         as violated; got {} for `{}`",
        unbound.verdict,
        unbound.expression.clone().unwrap_or_default()
    );

    let violated = verdict_of(&rows, "pressureImpossible").expect("pressureImpossible present");
    assert_eq!(
        violated.verdict,
        VerdictKind::Fail,
        "`pressure >= 100.0` with `pressure = 12.0` is a decided violation"
    );

    let held = verdict_of(&rows, "pressureInRange").expect("pressureInRange present");
    assert_eq!(
        held.verdict,
        VerdictKind::Pass,
        "`pressure` in [9, 16] with `pressure = 12.0` holds"
    );

    // And the distinction is visible, not collapsed: Fail and Inconclusive
    // are different values, which is precisely what a `satisfied: bool` wire
    // field could not express.
    assert_ne!(
        unbound.verdict, violated.verdict,
        "undecidable and violated must not collapse to the same verdict"
    );
}

/// **The user-visible outcome.** The whole set is small and every row is a
/// constraint of the subject model. This is the assertion that would have
/// caught "48 failing · 4/52": the set may not be dominated by rows the
/// model never authored.
#[test]
fn session_constraint_set_contains_only_subject_model_usages() {
    let (_dir, rows) = session_constraints(FIXTURE);

    let names: Vec<&String> = rows.iter().map(|r| &r.name).collect();
    let expected = ["pressureInRange", "pressureImpossible", "dependsOnUnbound"];

    assert_eq!(
        rows.len(),
        expected.len(),
        "the session constraint set must be exactly this model's constraint usages; got {names:?}"
    );
    for name in expected {
        assert!(names.iter().any(|n| *n == name), "missing `{name}` in {names:?}");
    }
}
