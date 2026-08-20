//! Port well-formedness: a port's non-port nested/owned usages must be referential.
//!
//! SysML §8.3.12.5 (PortDefinition) / §8.3.12.6 (PortUsage):
//! - `validatePortUsageNestedUsagesNotComposite` (S145):
//!   `nestedUsage->reject(oclIsKindOf(PortUsage))->forAll(not isComposite)`.
//! - `validatePortDefinitionOwnedUsagesNotComposite` (S146):
//!   `ownedUsage->reject(oclIsKindOf(PortUsage))->forAll(not isComposite)`.
//!
//! `isComposite` here is the *derived* property: a directed feature
//! (`in`/`out`/`inout`) is referential, so directed port payload features like
//! `out item power : ACPhase` are exempt — see
//! [`super::composite::is_effectively_composite`].
//!
//! Both say the same thing on the same model shape: every owned/nested usage that
//! is not itself a `PortUsage` must be referential (non-composite). A nested
//! `PortUsage` is exempt (ports may be composite). Compositeness is decided by
//! [`super::composite::is_effectively_composite`].
//!
//! ## What counts as an `ownedUsage` / `nestedUsage`
//!
//! Both quantifiers range over *features*, not over every owned member:
//! - SysML `Definition::/ownedUsage : Usage [0..*] {subsets ownedFeature, usage}`
//!   — "The Usages that are ownedFeatures of this Definition."
//! - SysML `deriveUsageNestedUsage`: "The ownedUsages of a Usage are all its
//!   ownedFeatures that are Usages." — `nestedUsage = ownedFeature->selectByKind(Usage)`.
//! - KerML `deriveTypeOwnedFeature`: "The ownedFeatures of a Type are the
//!   ownedMemberFeatures of its ownedFeatureMemberships" —
//!   `ownedFeature = ownedFeatureMembership.ownedMemberFeature`; and
//!   `ownedFeatureMembership = ownedRelationship->selectByKind(FeatureMembership)`.
//!
//! An `AnnotatingElement` in a definition/usage body is owned through a plain
//! `OwningMembership`, never a `FeatureMembership`: SysML.xtext routes it via
//! `DefinitionMember returns SysML::OwningMembership` (→ `DefinitionElement` →
//! `AnnotatingElement`) and `AnnotatingMember returns SysML::OwningMembership`,
//! while ordinary body usages arrive through `NonOccurrenceUsageMember` /
//! `OccurrenceUsageMember`, both of which `return SysML::FeatureMembership`.
//! So an annotation is an `ownedMember` but not an `ownedFeature`, hence not an
//! `ownedUsage`/`nestedUsage`, and these constraints do not reach it.
//!
//! This matters because `MetadataUsage` is *both* a `MetadataFeature`
//! (→ `AnnotatingElement`) and an `ItemUsage` (→ `OccurrenceUsage`), so a bare
//! `@Signal;` tag inside a `port def` body otherwise passes the "is a Usage"
//! gate and reads as composite-by-default. It is an annotation, not a port
//! feature — see [`is_annotating`].

use super::composite::is_effectively_composite;
use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Rule S145: the nested usages of a `PortUsage` that are not themselves
/// `PortUsage`s must be referential.
pub fn port_usage_nested_usages_referential(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::PortUsage
        && !element.kind.is_subtype_of(ElementKind::PortUsage)
    {
        return None;
    }
    flag_composite_non_port_usages(element, graph, "S145")
}

/// Rule S146: the owned usages of a `PortDefinition` that are not `PortUsage`s
/// must be referential.
pub fn port_definition_owned_usages_referential(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::PortDefinition
        && !element.kind.is_subtype_of(ElementKind::PortDefinition)
    {
        return None;
    }
    flag_composite_non_port_usages(element, graph, "S146")
}

/// Whether `kind` is an `AnnotatingElement` — a `Comment`, `Documentation`,
/// `TextualRepresentation`, or `MetadataUsage`.
///
/// Annotating members are owned through a plain `OwningMembership`, so they are
/// `ownedMember`s but not `ownedFeature`s and therefore never `ownedUsage`s /
/// `nestedUsage`s (see the module docs for the derivation chain). The
/// `AnnotatingElement` supertype is generated from the spec vocabulary
/// (`Kerml-Vocab.ttl`: `MetadataFeature rdfs:subClassOf AnnotatingElement`;
/// `SysML-vocab.ttl`: `MetadataUsage rdfs:subClassOf ItemUsage, MetadataFeature`),
/// so this is a spec-derived test, not a hand-rolled kind list.
fn is_annotating(kind: &ElementKind) -> bool {
    *kind == ElementKind::AnnotatingElement || kind.is_subtype_of(ElementKind::AnnotatingElement)
}

/// Shared body: flag every owned/nested usage that is not a `PortUsage` and is
/// effectively composite.
fn flag_composite_non_port_usages(
    element: &Element,
    graph: &ModelGraph,
    rule_id: &'static str,
) -> Option<Vec<SemanticError>> {
    let mut errors = Vec::new();
    for child in graph.children_of(&element.id) {
        // Nested ports are exempt (the OCL `reject(oclIsKindOf(PortUsage))`).
        if child.kind == ElementKind::PortUsage
            || child.kind.is_subtype_of(ElementKind::PortUsage)
        {
            continue;
        }
        // Annotations (`@Signal;`, `doc`, `comment`, `rep`) are owned members,
        // not owned *features* — outside `ownedUsage`/`nestedUsage` entirely.
        if is_annotating(&child.kind) {
            continue;
        }
        // Only usages participate; skip memberships and other relationship children.
        if child.kind != ElementKind::Usage && !child.kind.is_subtype_of(ElementKind::Usage) {
            continue;
        }
        if is_effectively_composite(child) {
            errors.push(SemanticError {
                element_id: child.id.clone(),
                element_name: child.name.clone(),
                kind: SemanticErrorKind::Custom {
                    message: format!(
                        "port feature '{name}' is composite; features owned by a port must be \
                         referential — mark it `ref` (e.g. `ref {name} : …`) or give it a \
                         direction (`in`/`out`) (SysML §8.3.12.5)",
                        name = child.name.as_deref().unwrap_or("<anonymous>")
                    ),
                },
                rule_id,
                is_warning: false,
            });
        }
    }
    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    fn child_in(graph: &mut ModelGraph, owner: &ElementId, kind: ElementKind, name: &str) {
        let e = Element::new(ElementId::new_v4(), kind)
            .with_name(name)
            .with_owner(owner.clone());
        graph.add_element(e);
    }

    #[test]
    fn composite_part_in_port_usage_is_flagged() {
        let mut graph = ModelGraph::new();
        let port = Element::new(ElementId::new_v4(), ElementKind::PortUsage).with_name("p");
        let pid = graph.add_element(port);
        child_in(&mut graph, &pid, ElementKind::PartUsage, "nested");
        let elem = graph.get_element(&pid).unwrap();
        let res = port_usage_nested_usages_referential(elem, &graph);
        assert!(res.is_some(), "composite part nested in a port usage must flag");
        assert_eq!(res.unwrap()[0].rule_id, "S145");
    }

    #[test]
    fn ref_part_in_port_usage_is_clean() {
        let mut graph = ModelGraph::new();
        let port = Element::new(ElementId::new_v4(), ElementKind::PortUsage).with_name("p");
        let pid = graph.add_element(port);
        let refpart = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("r")
            .with_owner(pid.clone())
            .with_prop("isReference", true);
        graph.add_element(refpart);
        let elem = graph.get_element(&pid).unwrap();
        assert!(port_usage_nested_usages_referential(elem, &graph).is_none());
    }

    #[test]
    fn nested_port_in_port_usage_is_clean() {
        let mut graph = ModelGraph::new();
        let port = Element::new(ElementId::new_v4(), ElementKind::PortUsage).with_name("p");
        let pid = graph.add_element(port);
        child_in(&mut graph, &pid, ElementKind::PortUsage, "q");
        let elem = graph.get_element(&pid).unwrap();
        assert!(
            port_usage_nested_usages_referential(elem, &graph).is_none(),
            "a nested port is exempt (ports may be composite)"
        );
    }

    #[test]
    fn attribute_in_port_usage_is_clean() {
        let mut graph = ModelGraph::new();
        let port = Element::new(ElementId::new_v4(), ElementKind::PortUsage).with_name("p");
        let pid = graph.add_element(port);
        child_in(&mut graph, &pid, ElementKind::AttributeUsage, "a");
        let elem = graph.get_element(&pid).unwrap();
        assert!(
            port_usage_nested_usages_referential(elem, &graph).is_none(),
            "attributes are referential by nature"
        );
    }

    #[test]
    fn metadata_annotation_in_port_usage_is_clean() {
        // `@Signal;` lowers to an anonymous MetadataUsage. It is a Usage AND an
        // OccurrenceUsage (via ItemUsage), so without the AnnotatingElement
        // exemption it reads as composite-by-default and falsely flags.
        let mut graph = ModelGraph::new();
        let port = Element::new(ElementId::new_v4(), ElementKind::PortUsage).with_name("p");
        let pid = graph.add_element(port);
        let meta = Element::new(ElementId::new_v4(), ElementKind::MetadataUsage)
            .with_owner(pid.clone());
        graph.add_element(meta);
        let elem = graph.get_element(&pid).unwrap();
        assert!(
            port_usage_nested_usages_referential(elem, &graph).is_none(),
            "an annotating member is not a nestedUsage"
        );
    }

    #[test]
    fn metadata_usage_is_both_a_usage_and_an_annotating_element() {
        // Pins the generated hierarchy this fix keys on: MetadataUsage subsets
        // ItemUsage → OccurrenceUsage → Usage AND MetadataFeature →
        // AnnotatingElement (SysML-vocab.ttl / Kerml-Vocab.ttl).
        assert!(ElementKind::MetadataUsage.is_subtype_of(ElementKind::OccurrenceUsage));
        assert!(is_annotating(&ElementKind::MetadataUsage));
        assert!(!is_annotating(&ElementKind::PartUsage));
        assert!(!is_annotating(&ElementKind::ItemUsage));
    }

    #[test]
    fn metadata_annotation_in_port_definition_is_clean() {
        let mut graph = ModelGraph::new();
        let pd = Element::new(ElementId::new_v4(), ElementKind::PortDefinition).with_name("Pt");
        let pid = graph.add_element(pd);
        let meta = Element::new(ElementId::new_v4(), ElementKind::MetadataUsage)
            .with_owner(pid.clone());
        graph.add_element(meta);
        let elem = graph.get_element(&pid).unwrap();
        assert!(
            port_definition_owned_usages_referential(elem, &graph).is_none(),
            "an annotating member is not an ownedUsage"
        );
    }

    #[test]
    fn annotation_exemption_does_not_blunt_the_rule() {
        // Negative control: a real composite part alongside an annotation still flags.
        let mut graph = ModelGraph::new();
        let pd = Element::new(ElementId::new_v4(), ElementKind::PortDefinition).with_name("Pt");
        let pid = graph.add_element(pd);
        let meta = Element::new(ElementId::new_v4(), ElementKind::MetadataUsage)
            .with_owner(pid.clone());
        graph.add_element(meta);
        child_in(&mut graph, &pid, ElementKind::PartUsage, "owned");
        let elem = graph.get_element(&pid).unwrap();
        let res = port_definition_owned_usages_referential(elem, &graph);
        assert!(res.is_some(), "composite part must still flag");
        let errs = res.unwrap();
        assert_eq!(errs.len(), 1, "only the part flags, not the annotation: {errs:?}");
        assert_eq!(errs[0].element_name.as_deref(), Some("owned"));
    }

    #[test]
    fn composite_part_in_port_definition_is_flagged() {
        let mut graph = ModelGraph::new();
        let pd = Element::new(ElementId::new_v4(), ElementKind::PortDefinition).with_name("Pt");
        let pid = graph.add_element(pd);
        child_in(&mut graph, &pid, ElementKind::PartUsage, "owned");
        let elem = graph.get_element(&pid).unwrap();
        let res = port_definition_owned_usages_referential(elem, &graph);
        assert!(res.is_some(), "composite part owned by a port def must flag");
        assert_eq!(res.unwrap()[0].rule_id, "S146");
    }
}
