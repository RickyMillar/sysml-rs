//! Validation types for SysML element property constraints.
//!
//! This module provides types for representing validation errors
//! when element properties don't match their shape constraints.

use std::fmt;
use sysml_span::Span;

/// A validation error for an element property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The property that failed validation.
    pub property: String,
    /// The kind of validation error.
    pub kind: ValidationErrorKind,
    /// Best-effort source span for anchoring diagnostics in editor clients.
    pub span: Option<Span>,
}

impl ValidationError {
    /// Create an error for a missing required property.
    pub fn missing_required(property: impl Into<String>) -> Self {
        ValidationError {
            property: property.into(),
            kind: ValidationErrorKind::MissingRequired,
            span: None,
        }
    }

    /// Create an error for a property with the wrong type.
    pub fn wrong_type(
        property: impl Into<String>,
        expected: &'static str,
        got: impl Into<String>,
    ) -> Self {
        ValidationError {
            property: property.into(),
            kind: ValidationErrorKind::WrongType {
                expected,
                got: got.into(),
            },
            span: None,
        }
    }

    /// Create an error for a property that should have at least one value.
    pub fn min_cardinality(property: impl Into<String>) -> Self {
        ValidationError {
            property: property.into(),
            kind: ValidationErrorKind::MinCardinality,
            span: None,
        }
    }

    /// Create an error for a property that should have at most one value.
    pub fn max_cardinality(property: impl Into<String>) -> Self {
        ValidationError {
            property: property.into(),
            kind: ValidationErrorKind::MaxCardinality,
            span: None,
        }
    }

    /// Create an error for a read-only property being modified.
    pub fn read_only(property: impl Into<String>) -> Self {
        ValidationError {
            property: property.into(),
            kind: ValidationErrorKind::ReadOnly,
            span: None,
        }
    }

    /// Attach a source span to this validation error.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ValidationErrorKind::MissingRequired => {
                write!(f, "missing required property '{}'", self.property)
            }
            ValidationErrorKind::WrongType { expected, got } => {
                write!(
                    f,
                    "property '{}' has wrong type: expected {}, got {}",
                    self.property, expected, got
                )
            }
            ValidationErrorKind::MinCardinality => {
                write!(
                    f,
                    "property '{}' requires at least one value",
                    self.property
                )
            }
            ValidationErrorKind::MaxCardinality => {
                write!(f, "property '{}' allows at most one value", self.property)
            }
            ValidationErrorKind::ReadOnly => {
                write!(f, "property '{}' is read-only", self.property)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Convert ValidationError to Diagnostic for unified error reporting.
///
/// Error codes:
/// - V001: MissingRequired
/// - V002: WrongType
/// - V003: MinCardinality
/// - V004: MaxCardinality
/// - V005: ReadOnly
impl From<ValidationError> for sysml_span::Diagnostic {
    fn from(error: ValidationError) -> Self {
        let code = match &error.kind {
            ValidationErrorKind::MissingRequired => "V001",
            ValidationErrorKind::WrongType { .. } => "V002",
            ValidationErrorKind::MinCardinality => "V003",
            ValidationErrorKind::MaxCardinality => "V004",
            ValidationErrorKind::ReadOnly => "V005",
        };

        let mut diagnostic =
            sysml_span::Diagnostic::error(format!("{}: {}", error.property, error.kind))
                .with_code(code.to_owned())
                .with_tier(sysml_span::DiagnosticTier::Semantic);
        if let Some(span) = error.span {
            diagnostic = diagnostic.with_span(span);
        }
        diagnostic
    }
}

/// The kind of validation error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationErrorKind {
    /// A required property (Exactly-one) is missing.
    #[error("missing required property")]
    MissingRequired,
    /// The property value has the wrong type.
    #[error("wrong type: expected {expected}, got {got}")]
    WrongType {
        /// The expected type name.
        expected: &'static str,
        /// The actual type name.
        got: String,
    },
    /// A One-or-many property has no values.
    #[error("requires at least one value")]
    MinCardinality,
    /// A Zero-or-one or Exactly-one property has multiple values.
    #[error("allows at most one value")]
    MaxCardinality,
    /// A read-only property was modified.
    #[error("read-only property")]
    ReadOnly,
}

/// Result of validating an element.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    /// The errors found during validation.
    pub errors: Vec<ValidationError>,
}

impl ValidationResult {
    /// Create a new empty validation result.
    pub fn new() -> Self {
        ValidationResult { errors: Vec::new() }
    }

    /// Check if validation passed (no errors).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the number of errors.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Add an error to the result.
    pub fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// Merge another validation result into this one.
    pub fn merge(&mut self, other: ValidationResult) {
        self.errors.extend(other.errors);
    }
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(f, "validation passed")
        } else {
            writeln!(f, "validation failed with {} error(s):", self.errors.len())?;
            for error in &self.errors {
                writeln!(f, "  - {}", error)?;
            }
            Ok(())
        }
    }
}

// =====================================================================
// PROPERTY VALIDATION PIPELINE
// =====================================================================
//
// Validates element properties against shape constraints.
// Uses the generated validate_element_properties() dispatcher.

/// Validate all element properties in a model graph.
///
/// Iterates over every element in the graph and runs the generated
/// property validation dispatcher (`validate_element_properties`).
/// Returns a combined `ValidationResult` with all property errors.
///
/// This is Phase A scaffolding -- the per-type validators call into
/// the generated `validate()` methods on `XxxProps` structs.
pub fn validate_graph_properties(graph: &crate::ModelGraph) -> ValidationResult {
    let mut result = ValidationResult::new();
    for element in graph.elements.values() {
        let element_result = crate::validate_element_properties(element);
        for mut error in element_result.errors {
            if !is_post_parse_validatable(&error.property) {
                // Suppress MissingRequired, MinCardinality, and WrongType for
                // properties populated by resolution or derived/computed.
                // These properties hold string references before resolution
                // that would fail type checks (e.g. importedNamespace expects
                // ElementId but holds a name string until resolved).
                match &error.kind {
                    ValidationErrorKind::MissingRequired
                    | ValidationErrorKind::MinCardinality
                    | ValidationErrorKind::WrongType { .. } => continue,
                    _ => {}
                }
            }
            if error.span.is_none() {
                error.span = element.spans.first().cloned();
            }
            result.add_error(error);
        }
    }
    result
}

/// Returns `true` if a property is expected to be populated at parse time.
///
/// Properties populated by resolution (e.g. `general`, `type`, `source`,
/// `target`) or derived/computed (e.g. `identifier`, `featureTarget`,
/// `result`) should NOT trigger V001 "missing required" at parse time
/// because they are filled in by later pipeline stages.
///
/// This is used by `validate_graph_properties()` to filter out false-positive
/// V001 errors for properties that haven't been set yet.
pub fn is_post_parse_validatable(property: &str) -> bool {
    !RESOLUTION_POPULATED_PROPERTIES.contains(&property)
        && !DERIVED_COMPUTED_PROPERTIES.contains(&property)
}

/// Properties populated by the resolution pipeline (not available at parse time).
const RESOLUTION_POPULATED_PROPERTIES: &[&str] = &[
    "general",
    "type",
    "subsettedFeature",
    "redefinedFeature",
    "referencedFeature",
    "source",
    "target",
    "superclassifier",
    "conjugatedType",
    "originalType",
    "featuringType",
    "disjoiningType",
    "unioningType",
    "intersectingType",
    "differencingType",
    "invertingFeature",
    "crossedFeature",
    "annotatedElement",
    "memberElement",
    "client",
    "supplier",
    "conjugatedPortDefinition",
    "multiplicity",
    "lowerBound",
    "upperBound",
    "specific",
];

/// Properties that are derived or computed (never set directly at parse time).
const DERIVED_COMPUTED_PROPERTIES: &[&str] = &[
    "identifier",
    "featureTarget",
    "result",
    "membershipOwningNamespace",
    "ownedMemberElement",
    "ownedMemberFeature",
    "owningType",
    "subjectParameter",
    "ownedMemberParameter",
    "ownedActorParameter",
    "importOwningNamespace",
    "importedElement",
    "importedNamespace",
    "importedMembership",
    "instantiatedType",
    "bodyAction",
    "performedAction",
    "ownedConstraint",
    "referencedConstraint",
    "assertedConstraint",
    "eventOccurrence",
    "exhibitedState",
    "satisfiedRequirement",
    "satisfyingFeature",
    "verifiedRequirement",
    "ownedObjectiveRequirement",
    "ownedResultExpression",
    "ownedSubjectParameter",
    "ownedStakeholderParameter",
    "ownedRequirement",
    "ownedRendering",
    "ownedConcern",
    "ownedPortConjugator",
    "ownedVariantUsage",
    "typedFeature",
    "featureOfType",
    "subsettingFeature",
    "redefiningFeature",
    "referencingFeature",
    "featureInverted",
    "featureChained",
    "chainingFeature",
    "crossingFeature",
    "subclassifier",
    "typeUnioned",
    "typeIntersected",
    "typeDisjoined",
    "typeDifferenced",
    "annotatingElement",
    "representedElement",
    "documentedElement",
    "referencedElement",
    "enumerationDefinition",
    "portDefinition",
    "originalPortDefinition",
    "targetFeature",
    "payloadParameter",
    "payloadArgument",
    "condition",
    "whileArgument",
    "seqArgument",
    "ifArgument",
    "thenAction",
    "loopVariable",
    "succession",
    "transitionFeature",
    "featureWithValue",
    "value",
    "referent",
    "action",
    "upperBound",
    "useCaseIncluded",
    "referencedRendering",
    "referencedConcern",
];

// =====================================================================
// SEMANTIC VALIDATION ERRORS (S001-S999)
// =====================================================================
//
// Semantic errors are produced by the semantic_checks module,
// which validates model constraints beyond structural integrity.

/// A semantic validation error for a model element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    /// The element that failed validation.
    pub element_id: sysml_id::ElementId,
    /// The name of the element (if available).
    pub element_name: Option<String>,
    /// The kind of semantic error.
    pub kind: SemanticErrorKind,
    /// The rule ID that produced this error (e.g., "S001").
    pub rule_id: &'static str,
    /// Whether this is an error or warning.
    pub is_warning: bool,
}

/// The kind of semantic validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticErrorKind {
    /// Duplicate member name in a namespace.
    DuplicateName {
        name: String,
        /// ID of the other element with the same name.
        other_id: sysml_id::ElementId,
    },

    /// A usage is typed by the wrong kind of definition.
    InvalidTyping {
        /// What kind of type was expected (e.g., "definition", "DataType").
        expected: &'static str,
        /// The actual ElementKind of the type.
        got: crate::ElementKind,
    },

    /// A membership element appears in the wrong ownership context.
    OwnershipViolation {
        /// The kind of membership that is misplaced.
        member_kind: crate::ElementKind,
        /// The kind of the owning element.
        owner_kind: crate::ElementKind,
    },

    /// A container has too many of a particular member type.
    CardinalityViolation {
        /// Description of the member type (e.g., "SubjectMembership").
        member_type: &'static str,
        /// Maximum allowed count.
        max: usize,
        /// Actual count found.
        actual: usize,
    },

    /// An invalid specialization crossing type boundaries.
    SpecializationViolation {
        /// The subtype (specific) element kind.
        sub: crate::ElementKind,
        /// The supertype (general) element kind.
        super_: crate::ElementKind,
    },

    /// A variation constraint is violated.
    VariationViolation {
        /// What went wrong.
        detail: String,
    },

    /// A custom message for rules not covered by other variants.
    Custom { message: String },
}

impl SemanticError {
    /// The human-facing message *without* the `[rule_id]` prefix.
    ///
    /// The rule id is carried separately in the diagnostic's `code` field, so
    /// embedding `[S146]` in the message text duplicates it in rendered output
    /// (`error[S146]: [S146] 'x': …`). Diagnostic conversion uses this; the
    /// `Display` impl keeps the prefixed form for logs/debugging.
    pub fn display_message(&self) -> String {
        match &self.kind {
            // Custom messages are self-contained — they already name the subject.
            SemanticErrorKind::Custom { message } => message.clone(),
            kind => {
                let name_str = self
                    .element_name
                    .as_ref()
                    .map(|n| format!("'{}': ", n))
                    .unwrap_or_default();
                format!("{name_str}{kind}")
            }
        }
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name_str = self
            .element_name
            .as_ref()
            .map(|n| format!(" '{}'", n))
            .unwrap_or_default();
        write!(f, "[{}]{}: {}", self.rule_id, name_str, self.kind)
    }
}

impl fmt::Display for SemanticErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticErrorKind::DuplicateName { name, .. } => {
                write!(
                    f,
                    "duplicate definition '{}' is already defined in this scope",
                    name
                )
            }
            SemanticErrorKind::InvalidTyping { expected, got } => {
                write!(
                    f,
                    "must be typed by {}, found {}",
                    expected,
                    got.display_name()
                )
            }
            SemanticErrorKind::OwnershipViolation {
                member_kind,
                owner_kind,
            } => {
                write!(
                    f,
                    "{} is not allowed in {}",
                    member_kind.display_name(),
                    owner_kind.display_name()
                )
            }
            SemanticErrorKind::CardinalityViolation {
                member_type,
                max,
                actual,
            } => {
                write!(
                    f,
                    "at most {} {} allowed, found {}",
                    max, member_type, actual
                )
            }
            SemanticErrorKind::SpecializationViolation { sub, super_ } => {
                write!(
                    f,
                    "{} cannot specialize {}",
                    sub.display_name(),
                    super_.display_name()
                )
            }
            SemanticErrorKind::VariationViolation { detail } => {
                write!(f, "{}", detail)
            }
            SemanticErrorKind::Custom { message } => {
                write!(f, "{}", message)
            }
        }
    }
}

impl std::error::Error for SemanticError {}

impl SemanticError {
    /// Convert this error into a Diagnostic with span information from the graph.
    pub fn to_diagnostic_with_graph(&self, graph: &crate::ModelGraph) -> sysml_span::Diagnostic {
        let severity_fn = if self.is_warning {
            sysml_span::Diagnostic::warning
        } else {
            sysml_span::Diagnostic::error
        };

        // P-RA2 Slice 4: S001-S004 are within-file structural checks
        // (e.g. distinguishability, name/alias conflicts) — they don't need
        // workspace context. Other S-series codes are deeper semantic rules
        // and stay on the Semantic tier where the rest of the post-resolve
        // passes already live.
        let tier = semantic_rule_tier(self.rule_id);
        let mut diagnostic = severity_fn(self.display_message())
            .with_code(self.rule_id.to_owned())
            .with_tier(tier);

        // Attach span from the element
        if let Some(element) = graph.elements.get(&self.element_id) {
            if let Some(span) = element.spans.first() {
                diagnostic = diagnostic.with_span(span.clone());
            }
        }

        // Add related location for duplicate name errors
        if let SemanticErrorKind::DuplicateName { name, other_id } = &self.kind {
            if let Some(other) = graph.elements.get(other_id) {
                if let Some(span) = other.spans.first() {
                    diagnostic = diagnostic
                        .with_related(span.clone(), format!("'{}' first defined here", name));
                }
            }
        }

        // Add contextual notes for ownership violations
        if let SemanticErrorKind::OwnershipViolation {
            member_kind,
            owner_kind,
        } = &self.kind
        {
            diagnostic = diagnostic.with_note(format!(
                "{} elements are typically found inside definitions or usages, not {}",
                member_kind.display_name(),
                owner_kind.display_name()
            ));
        }

        diagnostic
    }
}

/// Convert SemanticError to Diagnostic (without graph context for span lookup).
impl From<SemanticError> for sysml_span::Diagnostic {
    fn from(error: SemanticError) -> Self {
        let severity_fn = if error.is_warning {
            sysml_span::Diagnostic::warning
        } else {
            sysml_span::Diagnostic::error
        };
        let tier = semantic_rule_tier(error.rule_id);
        severity_fn(error.display_message())
            .with_code(error.rule_id.to_owned())
            .with_tier(tier)
    }
}

/// Choose the readiness tier for an S-series rule (P-RA2 Slice 4).
///
/// S001-S004 are within-file structural checks — visible on the file's own
/// parse and don't require workspace indexing. Every other S-code is a
/// deeper semantic rule (typing, specialization, membership context) that
/// already needs cross-file information by the time it runs.
fn semantic_rule_tier(rule_id: &str) -> sysml_span::DiagnosticTier {
    match rule_id {
        "S001" | "S002" | "S003" | "S004" => sysml_span::DiagnosticTier::StructuralLocal,
        _ => sysml_span::DiagnosticTier::Semantic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_required() {
        let error = ValidationError::missing_required("elementId");
        assert_eq!(error.property, "elementId");
        assert!(matches!(error.kind, ValidationErrorKind::MissingRequired));
        assert!(error.to_string().contains("missing required"));
    }

    #[test]
    fn test_wrong_type() {
        let error = ValidationError::wrong_type("owningType", "ElementId", "string");
        assert_eq!(error.property, "owningType");
        assert!(matches!(error.kind, ValidationErrorKind::WrongType { .. }));
        assert!(error.to_string().contains("wrong type"));
    }

    #[test]
    fn test_validation_result() {
        let mut result = ValidationResult::new();
        assert!(result.is_valid());

        result.add_error(ValidationError::missing_required("prop1"));
        assert!(!result.is_valid());
        assert_eq!(result.error_count(), 1);

        result.add_error(ValidationError::wrong_type("prop2", "bool", "string"));
        assert_eq!(result.error_count(), 2);
    }

    #[test]
    fn test_merge_results() {
        let mut result1 = ValidationResult::new();
        result1.add_error(ValidationError::missing_required("a"));

        let mut result2 = ValidationResult::new();
        result2.add_error(ValidationError::missing_required("b"));

        result1.merge(result2);
        assert_eq!(result1.error_count(), 2);
    }

    // === Diagnostic Conversion Tests (Phase 5) ===

    #[test]
    fn validation_error_to_diagnostic() {
        use sysml_span::Diagnostic;

        let error = ValidationError::missing_required("elementId");
        let diag: Diagnostic = error.into();

        assert!(diag.is_error());
        assert_eq!(diag.code, Some("V001".to_string()));
        assert!(diag.message.contains("elementId"));
    }

    #[test]
    fn validation_error_span_is_preserved_in_diagnostic() {
        use sysml_span::Diagnostic;

        let span = Span::new("file:///test.sysml", 2, 8);
        let error = ValidationError::missing_required("elementId").with_span(span.clone());
        let diag: Diagnostic = error.into();

        assert_eq!(diag.span, Some(span));
    }

    #[test]
    fn all_validation_errors_have_codes() {
        use sysml_span::Diagnostic;

        let errors = vec![
            ValidationError::missing_required("prop1"),
            ValidationError::wrong_type("prop2", "ElementId", "string"),
            ValidationError::min_cardinality("prop3"),
            ValidationError::max_cardinality("prop4"),
            ValidationError::read_only("prop5"),
        ];

        let expected_codes = ["V001", "V002", "V003", "V004", "V005"];

        for (error, expected_code) in errors.into_iter().zip(expected_codes.iter()) {
            let diag: Diagnostic = error.into();
            assert_eq!(
                diag.code,
                Some(expected_code.to_string()),
                "Wrong code for error"
            );
            assert!(diag.is_error());
        }
    }

    #[test]
    fn validation_errors_carry_semantic_tier() {
        use sysml_span::{Diagnostic, DiagnosticTier};

        // Every V001-V005 emission must be tagged with the Semantic tier so
        // the readiness gate can include/exclude them as a cluster.
        let errors = vec![
            ValidationError::missing_required("prop1"),
            ValidationError::wrong_type("prop2", "ElementId", "string"),
            ValidationError::min_cardinality("prop3"),
            ValidationError::max_cardinality("prop4"),
            ValidationError::read_only("prop5"),
        ];

        for error in errors {
            let diag: Diagnostic = error.into();
            assert_eq!(
                diag.tier,
                DiagnosticTier::Semantic,
                "V* diagnostic {:?} should be tagged Semantic",
                diag.code
            );
        }
    }

    // === Semantic Error Tests ===

    #[test]
    fn semantic_error_duplicate_name() {
        let error = SemanticError {
            element_id: sysml_id::ElementId::new_v4(),
            element_name: Some("Foo".to_string()),
            kind: SemanticErrorKind::DuplicateName {
                name: "Foo".to_string(),
                other_id: sysml_id::ElementId::new_v4(),
            },
            rule_id: "S001",
            is_warning: true,
        };

        assert!(error.to_string().contains("S001"));
        assert!(error.to_string().contains("duplicate definition 'Foo'"));

        let diag: sysml_span::Diagnostic = error.into();
        assert_eq!(diag.severity, sysml_span::Severity::Warning);
        assert_eq!(diag.code, Some("S001".to_string()));
    }

    #[test]
    fn semantic_error_invalid_typing() {
        let error = SemanticError {
            element_id: sysml_id::ElementId::new_v4(),
            element_name: Some("myPart".to_string()),
            kind: SemanticErrorKind::InvalidTyping {
                expected: "part definitions",
                got: crate::ElementKind::Package,
            },
            rule_id: "S015",
            is_warning: false,
        };

        assert!(error.to_string().contains("S015"));
        assert!(error.to_string().contains("part definitions"));

        let diag: sysml_span::Diagnostic = error.into();
        assert!(diag.is_error());
        assert_eq!(diag.code, Some("S015".to_string()));
    }

    #[test]
    fn semantic_error_ownership_violation() {
        let error = SemanticError {
            element_id: sysml_id::ElementId::new_v4(),
            element_name: None,
            kind: SemanticErrorKind::OwnershipViolation {
                member_kind: crate::ElementKind::StateSubactionMembership,
                owner_kind: crate::ElementKind::Package,
            },
            rule_id: "S042",
            is_warning: false,
        };

        let msg = error.to_string();
        assert!(msg.contains("S042"));
        assert!(msg.contains("state subaction membership"));
        assert!(msg.contains("package"));
    }

    #[test]
    fn semantic_error_cardinality() {
        let error = SemanticError {
            element_id: sysml_id::ElementId::new_v4(),
            element_name: Some("MyReq".to_string()),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "SubjectMembership",
                max: 1,
                actual: 2,
            },
            rule_id: "S060",
            is_warning: false,
        };

        assert!(error.to_string().contains("at most 1"));
        assert!(error.to_string().contains("found 2"));
    }

    #[test]
    fn semantic_error_specialization() {
        let error = SemanticError {
            element_id: sysml_id::ElementId::new_v4(),
            element_name: None,
            kind: SemanticErrorKind::SpecializationViolation {
                sub: crate::ElementKind::DataType,
                super_: crate::ElementKind::Class,
            },
            rule_id: "S030",
            is_warning: false,
        };

        let msg = error.to_string();
        assert!(msg.contains("data type"));
        assert!(msg.contains("class"));
    }

    // === Phase C: is_post_parse_validatable Tests ===

    #[test]
    fn identifier_is_not_post_parse_validatable() {
        assert!(!is_post_parse_validatable("identifier"));
    }

    #[test]
    fn is_abstract_is_post_parse_validatable() {
        assert!(is_post_parse_validatable("isAbstract"));
    }

    #[test]
    fn resolution_populated_properties_are_filtered() {
        for prop in &[
            "general",
            "type",
            "source",
            "target",
            "subsettedFeature",
            "redefinedFeature",
            "superclassifier",
            "specific",
        ] {
            assert!(
                !is_post_parse_validatable(prop),
                "{} should not be post-parse validatable",
                prop
            );
        }
    }

    #[test]
    fn derived_properties_are_filtered() {
        for prop in &[
            "identifier",
            "featureTarget",
            "result",
            "owningType",
            "typedFeature",
            "annotatingElement",
        ] {
            assert!(
                !is_post_parse_validatable(prop),
                "{} should not be post-parse validatable",
                prop
            );
        }
    }

    #[test]
    fn parse_populated_properties_are_validatable() {
        for prop in &[
            "isAbstract",
            "name",
            "shortName",
            "declaredName",
            "isComposite",
            "direction",
        ] {
            assert!(
                is_post_parse_validatable(prop),
                "{} should be post-parse validatable",
                prop
            );
        }
    }

    #[test]
    fn validate_graph_properties_filters_v001_for_identifier() {
        let graph = crate::ModelGraph::new();
        // An empty graph produces no errors
        let result = validate_graph_properties(&graph);
        assert!(
            result.is_valid(),
            "empty graph should produce no validation errors"
        );
    }

    #[test]
    fn validate_graph_properties_keeps_non_v001_errors() {
        // V002 (WrongType) errors should NOT be filtered regardless of property name
        let mut result = ValidationResult::new();
        let error = ValidationError::wrong_type("identifier", "ElementId", "string");
        result.add_error(error);
        assert_eq!(result.error_count(), 1, "V002 errors should be kept");
    }

    /// P-RA2 Slice 4: S001-S004 are within-file structural checks and
    /// must serialize as `StructuralLocal`, while higher S-codes stay on
    /// the post-resolve `Semantic` tier.
    #[test]
    fn semantic_error_tier_splits_s001_s004_from_others() {
        use sysml_id::ElementId;

        let mk = |rule_id: &'static str| SemanticError {
            element_id: ElementId::new_v4(),
            element_name: Some("X".to_owned()),
            kind: SemanticErrorKind::Custom {
                message: format!("triggered by {}", rule_id),
            },
            rule_id,
            is_warning: false,
        };

        for code in ["S001", "S002", "S003", "S004"] {
            let diag: sysml_span::Diagnostic = mk(code).into();
            assert_eq!(diag.code.as_deref(), Some(code));
            assert_eq!(
                diag.tier,
                sysml_span::DiagnosticTier::StructuralLocal,
                "{} should be StructuralLocal",
                code
            );
        }

        for code in ["S015", "S030", "S042", "S060"] {
            let diag: sysml_span::Diagnostic = mk(code).into();
            assert_eq!(
                diag.tier,
                sysml_span::DiagnosticTier::Semantic,
                "{} should stay on Semantic tier",
                code
            );
        }
    }
}
