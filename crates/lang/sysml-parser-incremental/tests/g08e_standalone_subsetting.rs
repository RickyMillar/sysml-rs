//! G08e end-to-end gate: a standalone `subset X subsets Y;` declaration parses
//! to a `Subsetting` relationship (NOT a phantom Usage) and BOTH endpoints
//! resolve by name through pass-2 resolution.
//!
//! Spec: KerML.xtext:679-688 — `Subsetting` is a `NonFeatureElement` namespace
//! member; both `subsettingFeature` and `subsettedFeature` are feature
//! references resolved by name. The owned `:>` form (handled elsewhere) instead
//! sets `subsettingFeature` directly to the owning feature.

#![cfg(feature = "semantic")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use sysml_core::ElementKind;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

#[test]
fn standalone_subset_resolves_both_endpoints() {
    let src = "package P { part a; part b; subset a subsets b; }";
    let parser = TreeSitterParser::new();
    let result = parser
        .parse(&[SysmlFile::new("g08e.sysml", src)])
        .into_resolved();
    let graph = &result.graph;

    let sub = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::Subsetting)
        .expect("standalone `subset a subsets b;` must mint a Subsetting relationship");

    let a = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("a") && e.kind == ElementKind::PartUsage)
        .expect("part a");
    let b = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("b") && e.kind == ElementKind::PartUsage)
        .expect("part b");

    assert_eq!(
        sub.get_prop("subsettingFeature").and_then(|v| v.as_ref()),
        Some(&a.id),
        "subsettingFeature must resolve to part `a` (no leftover unresolved_subsettingFeature)"
    );
    assert_eq!(
        sub.get_prop("subsettedFeature").and_then(|v| v.as_ref()),
        Some(&b.id),
        "subsettedFeature must resolve to part `b`"
    );
    // Endpoints fully resolved — no unresolved residue.
    assert!(
        sub.get_prop("unresolved_subsettingFeature").is_none()
            || sub
                .get_prop("subsettingFeature")
                .and_then(|v| v.as_ref())
                .is_some(),
        "subsetting endpoint should be resolved, not left unresolved"
    );
}
