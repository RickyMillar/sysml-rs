//! Phase B.1 gate: `FeatureReferenceExpression` (FRE) resolution.
//!
//! FREs — the identifiers inside constraint / calc / binding expressions — are
//! minted outside the `unresolved_*` prop machinery, so the two-pass resolver
//! never selected them. The `pass_refs` pass now selects them by kind and, for
//! bare single-segment names, writes an ADDITIVE `resolved_props::FEATURE_REFERENCE`
//! (`Value::Ref`) pointing at the resolved target. This test gates that the ref
//! is minted, points at the right sibling feature, and that a genuinely
//! unresolvable ref is left untouched (no wrong ref, no panic).
//!

use sysml_core::resolution::{resolve_references, resolved_props};
use sysml_core::{ElementKind, ModelGraph, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Parse inline SysML source into a (raw, un-elaborated) `ModelGraph`.
fn parse(src: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("fre.sysml".to_string(), src.to_string())];
    parser.parse(&files).graph
}

/// Collect every `FeatureReferenceExpression` whose display name equals `name`.
fn fres_named<'g>(graph: &'g ModelGraph, name: &str) -> Vec<&'g sysml_core::Element> {
    graph
        .element_ids_by_kind(&ElementKind::FeatureReferenceExpression)
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.name.as_deref() == Some(name))
        .collect()
}

/// First `AttributeUsage` id named `name`.
fn attr_id(graph: &ModelGraph, name: &str) -> sysml_id::ElementId {
    graph
        .element_ids_by_kind(&ElementKind::AttributeUsage)
        .iter()
        .find(|id| graph.get_element(id).and_then(|e| e.name.as_deref()) == Some(name))
        .cloned()
        .unwrap_or_else(|| panic!("attribute {name} exists"))
}

/// Bare-name expression references (both operands of an additive expression)
/// resolve to their sibling attributes; the resolved target id is written to
/// `resolved_props::FEATURE_REFERENCE`.
#[test]
fn bare_feature_references_resolve_to_sibling_attributes() {
    let src = r#"
package P {
    calc def Derivative {
        attribute V_applied : Real default 1.0;
        attribute N_drive : Real default 2.0;
        return dBdt = V_applied - N_drive;
    }
}
"#;
    let mut graph = parse(src);
    resolve_references(&mut graph);

    for name in ["V_applied", "N_drive"] {
        let target = attr_id(&graph, name);
        let refs = fres_named(&graph, name);
        assert!(
            !refs.is_empty(),
            "expected a FeatureReferenceExpression named {name}"
        );
        let resolved = refs
            .iter()
            .find_map(|e| e.props.get(resolved_props::FEATURE_REFERENCE))
            .unwrap_or_else(|| panic!("{name} FRE carries a resolved FEATURE_REFERENCE prop"));
        assert_eq!(
            resolved,
            &Value::Ref(target),
            "FRE {name} must resolve to its sibling attribute"
        );
    }
}

/// An expression reference to a name that exists nowhere in scope is left
/// untouched — no `FEATURE_REFERENCE` prop — while a resolvable reference in the
/// same expression still resolves (a miss must not poison the pass).
#[test]
fn unresolvable_feature_reference_is_left_untouched() {
    let src = r#"
package P {
    calc def Derivative {
        attribute known : Real default 1.0;
        return result = known - totallyMissingName;
    }
}
"#;
    let mut graph = parse(src);
    resolve_references(&mut graph);

    let missing = fres_named(&graph, "totallyMissingName");
    assert!(
        !missing.is_empty(),
        "expected a FeatureReferenceExpression named totallyMissingName"
    );
    for e in &missing {
        assert!(
            !e.props.contains_key(resolved_props::FEATURE_REFERENCE),
            "unresolvable FRE must NOT carry a FEATURE_REFERENCE prop"
        );
    }

    let known_id = attr_id(&graph, "known");
    let known = fres_named(&graph, "known");
    assert!(
        known.iter().any(|e| {
            e.props.get(resolved_props::FEATURE_REFERENCE) == Some(&Value::Ref(known_id.clone()))
        }),
        "the resolvable `known` reference must still resolve to its attribute"
    );
}

/// A dotted reference (`w.mass`) is NOT resolved by B.1 (gated to B.1.2) — the
/// pass must skip it rather than mint a wrong bare-name ref.
#[test]
fn dotted_reference_is_skipped_in_b1() {
    let src = r#"
package P {
    part def Wheel { attribute mass : Real default 1.0; }
    calc def C {
        attribute w : Wheel;
        return m = w.mass;
    }
}
"#;
    let mut graph = parse(src);
    resolve_references(&mut graph);

    for e in fres_named(&graph, "w.mass") {
        assert!(
            !e.props.contains_key(resolved_props::FEATURE_REFERENCE),
            "dotted FRE must be left for B.1.2, not resolved by B.1"
        );
    }
}
