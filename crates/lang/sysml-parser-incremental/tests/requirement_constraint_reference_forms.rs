//! F1 regression: `assume`/`require` constraint and `frame concern` reference
//! forms mint a REAL relationship, not a `referencedConstraint` string prop.
//!
//! Spec (SysML.xtext:2061-2076): a RequirementConstraintUsage / FramedConcernUsage
//! reference form is `ownedRelationship += OwnedReferenceSubsetting
//! FeatureSpecialization*`, lowered onto the membership's owned ConstraintUsage
//! (`ownedConstraint`). The bare-name form is a ReferenceSubsetting
//! (OwnedReferenceSubsetting → ReferenceSubsetting, :448); the `: Def` form is a
//! FeatureTyping. `referencedConstraint` is DERIVED from that relationship
//! (SysML-vocab.ttl:2576) — never a parse-time string. Clean-room fixture
//! (espresso domain / abstract names).

#![cfg(feature = "semantic")]

use sysml_core::query::{
    referenced_constraint_ref_name, referenced_constraint_target,
    requirement_constraint_body_owner,
};
use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

const SOURCE: &str = r#"
package Espresso {
    constraint def TempOk { brewTemp < 96 }
    constraint tempCheck { brewTemp > 88 }
    requirement def BrewReq {
        attribute brewTemp;
        require constraint : TempOk;
        require tempCheck;
        require constraint { brewTemp > 0 }
    }
}
"#;

fn build_resolved() -> ModelGraph {
    let parser = TreeSitterParser::new();
    let mut result = parser.parse(&[SysmlFile::new("espresso.sysml", SOURCE)]);
    // Reference targets are read from the RESOLVED relationship (the production
    // verification graph is always resolved) — resolve before asserting.
    sysml_core::resolution::resolve_references(&mut result.graph);
    result.graph
}

/// The three `require` memberships under `BrewReq`, split by form.
fn memberships(graph: &ModelGraph) -> Vec<sysml_core::ElementId> {
    graph
        .elements
        .values()
        .filter(|e| e.kind == ElementKind::RequirementConstraintMembership)
        .map(|e| e.id.clone())
        .collect()
}

#[test]
fn no_membership_carries_a_referenced_constraint_string_prop() {
    let graph = build_resolved();
    for id in memberships(&graph) {
        let m = graph.get_element(&id).unwrap();
        assert!(
            m.get_prop("referencedConstraint").is_none(),
            "referencedConstraint is a DERIVED accessor, never a parse-time \
             string prop (SysML-vocab.ttl:2576)"
        );
    }
}

#[test]
fn bare_name_form_mints_a_reference_subsetting_that_resolves() {
    let graph = build_resolved();
    // The bare-name membership: its owned ConstraintUsage owns a
    // ReferenceSubsetting; the declared name is `tempCheck`.
    let bare = memberships(&graph)
        .into_iter()
        .map(|id| graph.get_element(&id).unwrap())
        .find(|m| referenced_constraint_ref_name(m, &graph) == Some("tempCheck"))
        .expect("bare-name reference form present");

    let owned = requirement_constraint_body_owner(bare, &graph);
    assert_eq!(owned.kind, ElementKind::ConstraintUsage, "owns a ConstraintUsage");
    assert!(
        graph
            .children_of(&owned.id)
            .any(|c| c.kind == ElementKind::ReferenceSubsetting),
        "the owned constraint owns a ReferenceSubsetting (not a string prop)"
    );

    let target = referenced_constraint_target(bare, &graph)
        .expect("bare-name reference resolves to its target constraint");
    assert_eq!(target.name.as_deref(), Some("tempCheck"));
    assert_eq!(target.kind, ElementKind::ConstraintUsage);
}

#[test]
fn typed_form_mints_a_feature_typing_that_resolves() {
    let graph = build_resolved();
    // The `: Def` membership: its owned ConstraintUsage owns a FeatureTyping;
    // the declared name is `TempOk`.
    let typed = memberships(&graph)
        .into_iter()
        .map(|id| graph.get_element(&id).unwrap())
        .find(|m| referenced_constraint_ref_name(m, &graph) == Some("TempOk"))
        .expect("`: Def` reference form present");

    let owned = requirement_constraint_body_owner(typed, &graph);
    assert!(
        graph
            .children_of(&owned.id)
            .any(|c| c.kind == ElementKind::FeatureTyping),
        "the owned constraint owns a FeatureTyping (not a string prop)"
    );

    let target = referenced_constraint_target(typed, &graph)
        .expect("`: Def` reference resolves to its definition");
    assert_eq!(target.name.as_deref(), Some("TempOk"));
    assert_eq!(target.kind, ElementKind::ConstraintDefinition);
}

#[test]
fn inline_body_form_has_no_reference() {
    let graph = build_resolved();
    // The inline-body membership references nothing: no ReferenceSubsetting and
    // no FeatureTyping on its owned usage, so both derived accessors are None.
    let inline = memberships(&graph)
        .into_iter()
        .map(|id| graph.get_element(&id).unwrap())
        .find(|m| {
            let owned = requirement_constraint_body_owner(m, &graph);
            graph
                .children_of(&owned.id)
                .any(|c| c.kind == ElementKind::ResultExpressionMembership)
        })
        .expect("inline-body require constraint present");

    assert_eq!(
        referenced_constraint_ref_name(inline, &graph),
        None,
        "an inline body is not a reference form"
    );
    assert!(
        referenced_constraint_target(inline, &graph).is_none(),
        "an inline body references nothing"
    );
}
