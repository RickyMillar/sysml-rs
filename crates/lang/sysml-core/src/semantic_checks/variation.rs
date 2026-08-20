//! Variation constraint validation checks.
//!
//! Validates variation point rules:
//! - Variations must be abstract
//! - Variation members must be variants
//! - No variation chaining

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: check if an element is a variation point.
fn is_variation(element: &Element) -> bool {
    element
        .props
        .get("isVariation")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Rules S080-S081: A variation must be abstract.
pub fn variation_must_be_abstract(
    element: &Element,
    _graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !is_variation(element) {
        return None;
    }

    let is_abstract = element
        .props
        .get("isAbstract")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_abstract {
        None
    } else {
        let rule_id = if element.kind.is_definition() {
            "S080"
        } else {
            "S081"
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::VariationViolation {
                detail: "a variation must be abstract".to_owned(),
            },
            rule_id,
            is_warning: false,
        }])
    }
}

/// Rules S082-S083: Owned usages of a variation must be variants.
pub fn variation_members_are_variants(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !is_variation(element) {
        return None;
    }

    let rule_id = if element.kind.is_definition() {
        "S082"
    } else {
        "S083"
    };

    let mut errors = Vec::new();

    for child in graph.children_of(&element.id) {
        // Only check usage children
        if !child.kind.is_usage() {
            continue;
        }

        // Check if this child is owned through a VariantMembership
        // Simple heuristic: check if the child's owning membership is a VariantMembership
        if let Some(membership_id) = &child.owning_membership {
            if let Some(membership) = graph.elements.get(membership_id) {
                if membership.kind != ElementKind::VariantMembership {
                    errors.push(SemanticError {
                        element_id: child.id.clone(),
                        element_name: child.name.clone(),
                        kind: SemanticErrorKind::VariationViolation {
                            detail: "an owned usage of a variation must be a variant".to_owned(),
                        },
                        rule_id,
                        is_warning: false,
                    });
                }
            }
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rules S084-S085: A variation must not specialize another variation.
pub fn variation_no_chain(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if !is_variation(element) {
        return None;
    }

    let rule_id = if element.kind.is_definition() {
        "S084"
    } else {
        "S085"
    };

    // Check if any general type is also a variation
    if let Some(spec_ids) = graph.specific_to_specializations.get(&element.id) {
        for spec_id in spec_ids {
            if let Some(spec_elem) = graph.elements.get(spec_id) {
                if let Some(general_id) = spec_elem.props.get("general").and_then(|v| v.as_ref()) {
                    if let Some(general) = graph.elements.get(general_id) {
                        if is_variation(general) {
                            return Some(vec![SemanticError {
                                element_id: element.id.clone(),
                                element_name: element.name.clone(),
                                kind: SemanticErrorKind::VariationViolation {
                                    detail: "a variation must not specialize another variation"
                                        .to_owned(),
                                },
                                rule_id,
                                is_warning: false,
                            }]);
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn non_variation_passes() {
        let graph = ModelGraph::new();
        let elem =
            Element::new(ElementId::new_v4(), ElementKind::PartDefinition).with_name("Regular");

        assert!(variation_must_be_abstract(&elem, &graph).is_none());
    }

    #[test]
    fn abstract_variation_passes() {
        let graph = ModelGraph::new();
        let elem = Element::new(ElementId::new_v4(), ElementKind::PartDefinition)
            .with_name("MyVariation")
            .with_prop("isVariation", true)
            .with_prop("isAbstract", true);

        assert!(variation_must_be_abstract(&elem, &graph).is_none());
    }

    #[test]
    fn non_abstract_variation_fails() {
        let graph = ModelGraph::new();
        let elem = Element::new(ElementId::new_v4(), ElementKind::PartDefinition)
            .with_name("MyVariation")
            .with_prop("isVariation", true)
            .with_prop("isAbstract", false);

        let result = variation_must_be_abstract(&elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S080");
    }

    #[test]
    fn variation_usage_non_abstract_fails() {
        let graph = ModelGraph::new();
        let elem = Element::new(ElementId::new_v4(), ElementKind::PartUsage)
            .with_name("MyVariation")
            .with_prop("isVariation", true)
            .with_prop("isAbstract", false);

        let result = variation_must_be_abstract(&elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S081");
    }
}
