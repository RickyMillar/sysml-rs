//! Conformance gate for per-instance constraint evaluation.
//!
//! Spec basis: `Constraints.sysml:23` (`constraintChecks :> booleanEvaluations`)
//! read with `Performances.kerml:94-102` — a ConstraintUsage is a Predicate
//! evaluated per occurrence. A constraint must be evaluated against the bound
//! values of its owning instance; a value-less reference is INCONCLUSIVE, not a
//! definitive failure.
//!
//! These cases mirror the empirical sweep that motivated the
//! constraint-evaluation conformance wave (c1/c5/c6/c8 are single-instance;
//! c3 is the multi-instance N-verdict case handled in a later increment).

use sysml_service::{SysmlService, VerdictKind};

/// Returns (name, verdict, message) for each constraint occurrence, in order.
fn check(src: &str) -> Vec<(String, VerdictKind, Option<String>)> {
    let service = SysmlService::empty();
    service.load_source("conf.sysml", src).unwrap();
    let results = service.check_constraints("conf.sysml", &[]).unwrap();
    results
        .into_iter()
        .map(|r| (r.name, r.verdict, r.message))
        .collect()
}

// c1: def declares a value-less attribute + constraint; a single instance binds
// a concrete satisfying value. The instance value (50) must win — the
// constraint must PASS, not be clobbered to a value-less reference.
#[test]
fn c1_single_instance_concrete_value_passes() {
    let src = "package C1 {\n\
        part def Tank { attribute level; constraint c { level < 100 } }\n\
        part tank : Tank { attribute level = 50; }\n\
    }";
    let r = check(src);
    eprintln!("c1 = {r:?}");
    assert!(!r.is_empty(), "c1: expected at least one constraint result");
    assert_eq!(
        r[0].1,
        VerdictKind::Pass,
        "c1: constraint should PASS (level=50 < 100), got {r:?}"
    );
}

// c5: def declares a value-less attribute + constraint; NO instance binds it.
// With nothing to evaluate against, the verdict must be INCONCLUSIVE, never a
// definitive failure.
#[test]
fn c5_no_instance_is_inconclusive() {
    let src = "package C5 {\n\
        part def Tank { attribute level; constraint c { level < 100 } }\n\
    }";
    let r = check(src);
    eprintln!("c5 = {r:?}");
    // A constraint on a definition with NO usages is omitted (no occurrence);
    // if a verdict is produced at all it must be Inconclusive, never Fail.
    for (_, verdict, _) in &r {
        assert_eq!(
            *verdict,
            VerdictKind::Inconclusive,
            "c5: value-less constraint must be Inconclusive (never Fail), got {r:?}"
        );
    }
}

// c6: constraint references a sibling attribute on the same usage instance.
#[test]
fn c6_sibling_on_usage() {
    let src = "package C6 {\n\
        part def Tank { attribute level; attribute cap; constraint c { level < cap } }\n\
        part tank : Tank { attribute level = 30; attribute cap = 100; }\n\
    }";
    let r = check(src);
    eprintln!("c6 = {r:?}");
    assert!(!r.is_empty(), "c6: expected a constraint result");
    assert_eq!(
        r[0].1,
        VerdictKind::Pass,
        "c6: should PASS (30 < 100), got {r:?}"
    );
}

// c8: nested — constraint in a nested part definition, single instance binds.
#[test]
fn c8_nested_single_instance() {
    let src = "package C8 {\n\
        part def Inner { attribute v; constraint c { v > 0 } }\n\
        part def Outer { part inner : Inner { attribute v = 5; } }\n\
        part outer : Outer;\n\
    }";
    let r = check(src);
    eprintln!("c8 = {r:?}");
    assert!(!r.is_empty(), "c8: expected a constraint result");
    assert_eq!(
        r[0].1,
        VerdictKind::Pass,
        "c8: should PASS (v=5 > 0), got {r:?}"
    );
}
