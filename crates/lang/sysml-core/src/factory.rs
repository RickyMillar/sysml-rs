//! Element factory for creating SysML v2 elements with type-appropriate defaults.
//!
//! The `ElementFactory` provides convenient methods for creating common element types
//! with sensible default property values based on the SysML v2 specification.
//!
//! ## Usage
//!
//! ```ignore
//! use sysml_core::ElementFactory;
//!
//! let pkg = ElementFactory::package("MyPackage");
//! let part_def = ElementFactory::part_definition("Vehicle");
//! let part_usage = ElementFactory::part_usage("engine");
//! ```
//!
//! ## Type-Appropriate Defaults
//!
//! Each factory method sets default property values that are appropriate for
//! the element kind, such as:
//! - `isAbstract = false` for definitions
//! - `isVariation = false` for usages
//! - `isComposite = true` for part usages

use std::borrow::Cow;

use crate::{Element, ElementKind, Value};

/// Factory for creating SysML v2 elements with type-appropriate defaults.
pub struct ElementFactory;

impl ElementFactory {
    /// Create an element of the given kind with default properties.
    ///
    /// This sets type-appropriate default values for common properties.
    ///
    /// Mints a fresh-UUID id — reparse-UNSTABLE, so this factory is for
    /// tests, examples, and ad-hoc graphs only (audited 2026-07-17: no
    /// production parse/elaboration path uses it). Parse/elaboration
    /// mint sites must derive ids from a [`crate::CanonicalKey`]
    /// (ADR-009) or identical re-runs compare unequal under the
    /// content-true `ModelGraph::fingerprint` and defeat salsa
    /// backdating. Pins: `sysml-ide-db/tests/id_stability.rs`,
    /// `sysml-service/tests/contract_id_stability.rs`.
    #[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
    pub fn create(kind: ElementKind) -> Element {
        let mut element = Element::new_with_kind(kind.clone());

        // Set type-appropriate defaults
        Self::apply_defaults(&mut element, &kind);

        element
    }

    /// Apply default properties based on element kind.
    fn apply_defaults(element: &mut Element, kind: &ElementKind) {
        // Defaults for Definition types
        if kind.is_definition() {
            element
                .props
                .insert(Cow::Borrowed("isAbstract"), Value::Bool(false));
        }

        // Defaults for Usage types
        if kind.is_usage() {
            element
                .props
                .insert(Cow::Borrowed("isVariation"), Value::Bool(false));
            element
                .props
                .insert(Cow::Borrowed("isReference"), Value::Bool(false));

            // Part usages are composite by default
            if *kind == ElementKind::PartUsage || kind.is_subtype_of(ElementKind::PartUsage) {
                element
                    .props
                    .insert(Cow::Borrowed("isComposite"), Value::Bool(true));
            }
        }

        // Defaults for Feature types
        if kind.is_feature() {
            element
                .props
                .insert(Cow::Borrowed("isUnique"), Value::Bool(true));
            element
                .props
                .insert(Cow::Borrowed("isOrdered"), Value::Bool(false));
            element
                .props
                .insert(Cow::Borrowed("isDerived"), Value::Bool(false));
            element
                .props
                .insert(Cow::Borrowed("isEnd"), Value::Bool(false));
        }

        // Defaults for Type types
        if kind.is_subtype_of(ElementKind::Type) || *kind == ElementKind::Type {
            element
                .props
                .insert(Cow::Borrowed("isAbstract"), Value::Bool(false));
            element
                .props
                .insert(Cow::Borrowed("isSufficient"), Value::Bool(false));
        }
    }

    // ========================
    // Package elements
    // ========================

    /// Create a Package element.
    pub fn package(name: &str) -> Element {
        Self::create(ElementKind::Package).with_name(name)
    }

    /// Create a LibraryPackage element.
    pub fn library_package(name: &str) -> Element {
        let mut elem = Self::create(ElementKind::LibraryPackage).with_name(name);
        elem.props
            .insert(Cow::Borrowed("isStandard"), Value::Bool(false));
        elem
    }

    // ========================
    // Definition elements
    // ========================

    /// Create a PartDefinition element.
    pub fn part_definition(name: &str) -> Element {
        Self::create(ElementKind::PartDefinition).with_name(name)
    }

    /// Create an abstract PartDefinition element.
    pub fn abstract_part_definition(name: &str) -> Element {
        let mut elem = Self::create(ElementKind::PartDefinition).with_name(name);
        elem.props
            .insert(Cow::Borrowed("isAbstract"), Value::Bool(true));
        elem
    }

    /// Create an ItemDefinition element.
    pub fn item_definition(name: &str) -> Element {
        Self::create(ElementKind::ItemDefinition).with_name(name)
    }

    /// Create an ActionDefinition element.
    pub fn action_definition(name: &str) -> Element {
        Self::create(ElementKind::ActionDefinition).with_name(name)
    }

    /// Create a StateDefinition element.
    pub fn state_definition(name: &str) -> Element {
        Self::create(ElementKind::StateDefinition).with_name(name)
    }

    /// Create a RequirementDefinition element.
    pub fn requirement_definition(name: &str) -> Element {
        Self::create(ElementKind::RequirementDefinition).with_name(name)
    }

    /// Create a ConstraintDefinition element.
    pub fn constraint_definition(name: &str) -> Element {
        Self::create(ElementKind::ConstraintDefinition).with_name(name)
    }

    /// Create a CalculationDefinition element.
    pub fn calculation_definition(name: &str) -> Element {
        Self::create(ElementKind::CalculationDefinition).with_name(name)
    }

    /// Create a CaseDefinition element.
    pub fn case_definition(name: &str) -> Element {
        Self::create(ElementKind::CaseDefinition).with_name(name)
    }

    /// Create an AnalysisCaseDefinition element.
    pub fn analysis_case_definition(name: &str) -> Element {
        Self::create(ElementKind::AnalysisCaseDefinition).with_name(name)
    }

    /// Create a VerificationCaseDefinition element.
    pub fn verification_case_definition(name: &str) -> Element {
        Self::create(ElementKind::VerificationCaseDefinition).with_name(name)
    }

    /// Create a UseCaseDefinition element.
    pub fn use_case_definition(name: &str) -> Element {
        Self::create(ElementKind::UseCaseDefinition).with_name(name)
    }

    /// Create a ViewDefinition element.
    pub fn view_definition(name: &str) -> Element {
        Self::create(ElementKind::ViewDefinition).with_name(name)
    }

    /// Create a ViewpointDefinition element.
    pub fn viewpoint_definition(name: &str) -> Element {
        Self::create(ElementKind::ViewpointDefinition).with_name(name)
    }

    /// Create a RenderingDefinition element.
    pub fn rendering_definition(name: &str) -> Element {
        Self::create(ElementKind::RenderingDefinition).with_name(name)
    }

    /// Create a PortDefinition element.
    pub fn port_definition(name: &str) -> Element {
        Self::create(ElementKind::PortDefinition).with_name(name)
    }

    /// Create a ConnectionDefinition element.
    pub fn connection_definition(name: &str) -> Element {
        Self::create(ElementKind::ConnectionDefinition).with_name(name)
    }

    /// Create an InterfaceDefinition element.
    pub fn interface_definition(name: &str) -> Element {
        Self::create(ElementKind::InterfaceDefinition).with_name(name)
    }

    /// Create an AllocationDefinition element.
    pub fn allocation_definition(name: &str) -> Element {
        Self::create(ElementKind::AllocationDefinition).with_name(name)
    }

    /// Create an AttributeDefinition element.
    pub fn attribute_definition(name: &str) -> Element {
        Self::create(ElementKind::AttributeDefinition).with_name(name)
    }

    /// Create an EnumerationDefinition element.
    pub fn enumeration_definition(name: &str) -> Element {
        Self::create(ElementKind::EnumerationDefinition).with_name(name)
    }

    /// Create an OccurrenceDefinition element.
    pub fn occurrence_definition(name: &str) -> Element {
        Self::create(ElementKind::OccurrenceDefinition).with_name(name)
    }

    /// Create a MetadataDefinition element.
    pub fn metadata_definition(name: &str) -> Element {
        Self::create(ElementKind::MetadataDefinition).with_name(name)
    }

    // ========================
    // Usage elements
    // ========================

    /// Create a PartUsage element.
    pub fn part_usage(name: &str) -> Element {
        Self::create(ElementKind::PartUsage).with_name(name)
    }

    /// Create a reference PartUsage element (isComposite = false, isReference = true).
    pub fn reference_part_usage(name: &str) -> Element {
        let mut elem = Self::create(ElementKind::PartUsage).with_name(name);
        elem.props
            .insert(Cow::Borrowed("isComposite"), Value::Bool(false));
        elem.props
            .insert(Cow::Borrowed("isReference"), Value::Bool(true));
        elem
    }

    /// Create an ItemUsage element.
    pub fn item_usage(name: &str) -> Element {
        Self::create(ElementKind::ItemUsage).with_name(name)
    }

    /// Create an ActionUsage element.
    pub fn action_usage(name: &str) -> Element {
        Self::create(ElementKind::ActionUsage).with_name(name)
    }

    /// Create a StateUsage element.
    pub fn state_usage(name: &str) -> Element {
        Self::create(ElementKind::StateUsage).with_name(name)
    }

    /// Create a RequirementUsage element.
    pub fn requirement_usage(name: &str) -> Element {
        Self::create(ElementKind::RequirementUsage).with_name(name)
    }

    /// Create a ConstraintUsage element.
    pub fn constraint_usage(name: &str) -> Element {
        Self::create(ElementKind::ConstraintUsage).with_name(name)
    }

    /// Create a CalculationUsage element.
    pub fn calculation_usage(name: &str) -> Element {
        Self::create(ElementKind::CalculationUsage).with_name(name)
    }

    /// Create a CaseUsage element.
    pub fn case_usage(name: &str) -> Element {
        Self::create(ElementKind::CaseUsage).with_name(name)
    }

    /// Create an AnalysisCaseUsage element.
    pub fn analysis_case_usage(name: &str) -> Element {
        Self::create(ElementKind::AnalysisCaseUsage).with_name(name)
    }

    /// Create a VerificationCaseUsage element.
    pub fn verification_case_usage(name: &str) -> Element {
        Self::create(ElementKind::VerificationCaseUsage).with_name(name)
    }

    /// Create a UseCaseUsage element.
    pub fn use_case_usage(name: &str) -> Element {
        Self::create(ElementKind::UseCaseUsage).with_name(name)
    }

    /// Create a ViewUsage element.
    pub fn view_usage(name: &str) -> Element {
        Self::create(ElementKind::ViewUsage).with_name(name)
    }

    /// Create a ViewpointUsage element.
    pub fn viewpoint_usage(name: &str) -> Element {
        Self::create(ElementKind::ViewpointUsage).with_name(name)
    }

    /// Create a RenderingUsage element.
    pub fn rendering_usage(name: &str) -> Element {
        Self::create(ElementKind::RenderingUsage).with_name(name)
    }

    /// Create a PortUsage element.
    pub fn port_usage(name: &str) -> Element {
        Self::create(ElementKind::PortUsage).with_name(name)
    }

    /// Create a ConnectionUsage element.
    pub fn connection_usage(name: &str) -> Element {
        Self::create(ElementKind::ConnectionUsage).with_name(name)
    }

    /// Create an InterfaceUsage element.
    pub fn interface_usage(name: &str) -> Element {
        Self::create(ElementKind::InterfaceUsage).with_name(name)
    }

    /// Create an AllocationUsage element.
    pub fn allocation_usage(name: &str) -> Element {
        Self::create(ElementKind::AllocationUsage).with_name(name)
    }

    /// Create an AttributeUsage element.
    pub fn attribute_usage(name: &str) -> Element {
        Self::create(ElementKind::AttributeUsage).with_name(name)
    }

    /// Create an EnumerationUsage element.
    pub fn enumeration_usage(name: &str) -> Element {
        Self::create(ElementKind::EnumerationUsage).with_name(name)
    }

    /// Create an OccurrenceUsage element.
    pub fn occurrence_usage(name: &str) -> Element {
        Self::create(ElementKind::OccurrenceUsage).with_name(name)
    }

    /// Create a MetadataUsage element.
    pub fn metadata_usage(name: &str) -> Element {
        Self::create(ElementKind::MetadataUsage).with_name(name)
    }

    // ========================
    // Membership elements
    // ========================

    /// Create an OwningMembership element.
    pub fn owning_membership() -> Element {
        Self::create(ElementKind::OwningMembership)
    }

    /// Create a FeatureMembership element.
    pub fn feature_membership() -> Element {
        Self::create(ElementKind::FeatureMembership)
    }

    /// Create a Membership element.
    pub fn membership() -> Element {
        Self::create(ElementKind::Membership)
    }

    // ========================
    // Relationship elements
    // ========================

    /// Create a Specialization element.
    pub fn specialization() -> Element {
        Self::create(ElementKind::Specialization)
    }

    /// Create a FeatureTyping element.
    pub fn feature_typing() -> Element {
        Self::create(ElementKind::FeatureTyping)
    }

    /// Create a Subsetting element.
    pub fn subsetting() -> Element {
        Self::create(ElementKind::Subsetting)
    }

    /// Create a Redefinition element.
    pub fn redefinition() -> Element {
        Self::create(ElementKind::Redefinition)
    }

    // ========================
    // Annotation elements
    // ========================

    /// Create a Comment element.
    pub fn comment(body: &str) -> Element {
        let mut elem = Self::create(ElementKind::Comment);
        elem.props
            .insert(Cow::Borrowed("body"), Value::String(body.to_owned()));
        elem
    }

    /// Create a Documentation element.
    pub fn documentation(body: &str) -> Element {
        let mut elem = Self::create(ElementKind::Documentation);
        elem.props
            .insert(Cow::Borrowed("body"), Value::String(body.to_owned()));
        elem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_package() {
        let pkg = ElementFactory::package("TestPackage");
        assert_eq!(pkg.kind, ElementKind::Package);
        assert_eq!(pkg.name, Some("TestPackage".to_string()));
    }

    #[test]
    fn factory_part_definition_defaults() {
        let part = ElementFactory::part_definition("Vehicle");
        assert_eq!(part.kind, ElementKind::PartDefinition);
        assert_eq!(part.name, Some("Vehicle".to_string()));

        // Check defaults
        assert_eq!(
            part.props.get("isAbstract").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn factory_abstract_part_definition() {
        let part = ElementFactory::abstract_part_definition("AbstractVehicle");
        assert_eq!(
            part.props.get("isAbstract").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn factory_part_usage_defaults() {
        let part = ElementFactory::part_usage("engine");
        assert_eq!(part.kind, ElementKind::PartUsage);
        assert_eq!(part.name, Some("engine".to_string()));

        // Check defaults
        assert_eq!(
            part.props.get("isComposite").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            part.props.get("isVariation").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            part.props.get("isReference").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn factory_reference_part_usage() {
        let part = ElementFactory::reference_part_usage("ref_engine");
        assert_eq!(
            part.props.get("isComposite").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            part.props.get("isReference").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn factory_create_generic() {
        let elem = ElementFactory::create(ElementKind::ActionDefinition);
        assert_eq!(elem.kind, ElementKind::ActionDefinition);
        assert_eq!(
            elem.props.get("isAbstract").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn factory_comment() {
        let comment = ElementFactory::comment("This is a test comment");
        assert_eq!(comment.kind, ElementKind::Comment);
        assert_eq!(
            comment.props.get("body").and_then(|v| v.as_str()),
            Some("This is a test comment")
        );
    }

    #[test]
    fn factory_owning_membership() {
        let membership = ElementFactory::owning_membership();
        assert_eq!(membership.kind, ElementKind::OwningMembership);
    }
}
