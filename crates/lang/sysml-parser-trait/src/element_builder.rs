//! Element building from extraction structs.
//!
//! This module provides methods to convert parser-agnostic extraction structs
//! into `Element` instances. Both Pest and tree-sitter parsers can use these
//! builders to ensure consistent element construction.

use sysml_core::{CanonicalKey, Element, ElementKind, Value};
use sysml_span::Span;

use crate::extraction::{DefinitionExtraction, PackageExtraction, UsageExtraction};

/// Mint an Element using a canonical key derived from `(parent_key, kind, name?, sibling_index?)`
/// when `parent_key` is set; otherwise fall through to `Element::new_with_kind`
/// (today's fresh-UUID behaviour).
///
/// This is the single threading point used by every `*Extraction::build_element`
/// in this module. When `name` is `Some`, the canonical key follows the
/// "named child" rule from ADR-009 (`for_named`); when `name` is `None`, it
/// follows the "anonymous child" rule (`for_anonymous`) using
/// `sibling_index.unwrap_or(0)`.
///
/// `kind_str` is `ElementKind::as_str()` — the spec-aligned variant name
/// (e.g. `"PartUsage"`). It is embedded in the canonical key string so
/// overload/redefinition cases that share path-and-name but differ in kind
/// stay distinct.
fn mint_element(
    kind: ElementKind,
    name: Option<&str>,
    parent_key: Option<&CanonicalKey>,
    sibling_index: Option<usize>,
) -> Element {
    match parent_key {
        Some(parent) => {
            let kind_str = kind.as_str();
            // `sibling_index = Some(_)` is the caller's explicit signal that
            // this element must use anonymous keying — either because it
            // has no name, or because the caller detected a sibling-name
            // collision and is falling back to anonymous keying to keep
            // the duplicates as distinct graph elements (so S001
            // distinguishability can fire). The element still carries
            // its declared `name` for downstream identification.
            let key = match (sibling_index, name) {
                (Some(idx), _) => CanonicalKey::for_anonymous(parent, kind_str, idx),
                (None, Some(name)) => CanonicalKey::for_named(parent, kind_str, name),
                (None, None) => CanonicalKey::for_anonymous(parent, kind_str, 0),
            };
            Element::new_with_key(kind, &key)
        }
        None => Element::new_with_kind(kind),
    }
}

impl UsageExtraction {
    /// Build an Element from this extraction data.
    ///
    /// This creates an Element with all properties set from the extracted data.
    /// The caller should then add the element to a ModelGraph and create any
    /// necessary relationship elements (FeatureTyping, Subsetting, etc.).
    ///
    /// # Arguments
    ///
    /// * `kind` - The ElementKind for this usage (e.g., PartUsage, AttributeUsage)
    /// * `span` - Optional span for source location tracking
    ///
    /// When `self.parent_key` is `Some`, the resulting element's `id` is
    /// derived from a [`sysml_core::CanonicalKey`] (ADR-009 / S1) so it
    /// stays stable across reparses. Otherwise the legacy fresh-UUID path
    /// is used.
    pub fn build_element(&self, kind: ElementKind, span: Option<Span>) -> Element {
        let mut element = mint_element(
            kind,
            self.name.as_deref(),
            self.parent_key.as_ref(),
            self.sibling_index,
        );

        if let Some(ref name) = self.name {
            element.name = Some(name.clone());
        }

        if let Some(ref short_name) = self.short_name {
            element.set_prop("declaredShortName", short_name.clone());
        }

        if let Some(ref direction) = self.direction {
            element.set_prop("direction", direction.as_str());
        }

        if let Some((lower, upper)) = self.multiplicity {
            element.set_prop("multiplicity_lower", Value::Int(lower));
            match upper {
                Some(u) => element.set_prop("multiplicity_upper", Value::Int(u)),
                None => element.set_prop("multiplicity_upper", Value::String("*".to_owned())),
            }

            // Build formatted multiplicity string for LSP display
            let bounds = match upper {
                Some(u) if u == lower => format!("{}", lower),
                Some(u) => format!("{}..{}", lower, u),
                None => format!("{}..*", lower),
            };
            let mut display = bounds;
            if self.is_ordered {
                display.push_str(" ordered");
            }
            if self.is_nonunique {
                display.push_str(" nonunique");
            }
            element.set_prop("multiplicity", Value::String(display));
        } else if self.multiplicity_lower_text.is_some() || self.multiplicity_upper_text.is_some() {
            // Symbolic multiplicity bounds (e.g., [min..max], [0..n], [n])
            if let Some(ref lower) = self.multiplicity_lower_text {
                element.set_prop("multiplicity_lower_text", Value::String(lower.clone()));
            }
            if let Some(ref upper) = self.multiplicity_upper_text {
                element.set_prop("multiplicity_upper_text", Value::String(upper.clone()));
            }

            // Build display string for symbolic bounds
            let lower_display = self.multiplicity_lower_text.as_deref().unwrap_or("0");
            let upper_display = self.multiplicity_upper_text.as_deref().unwrap_or("*");
            let mut display = format!("{}..{}", lower_display, upper_display);
            if self.is_ordered {
                display.push_str(" ordered");
            }
            if self.is_nonunique {
                display.push_str(" nonunique");
            }
            element.set_prop("multiplicity", Value::String(display));
        } else if self.is_ordered || self.is_nonunique {
            // Modifiers without explicit bounds
            let mut display = String::new();
            if self.is_ordered {
                display.push_str("ordered");
            }
            if self.is_nonunique {
                if !display.is_empty() {
                    display.push(' ');
                }
                display.push_str("nonunique");
            }
            element.set_prop("multiplicity", Value::String(display));
        }

        // Set spec-conforming boolean props (isOrdered, isUnique)
        if self.is_ordered {
            element.set_prop("isOrdered", true);
        }
        if self.is_nonunique {
            // Spec uses isUnique (default true); nonunique means isUnique=false
            element.set_prop("isUnique", false);
        }

        if let Some(ref value_expression) = self.value_expression {
            if self.value_is_literal {
                // For literals, parse and store as a typed "value" property
                // so context extraction (e.g. CLI) can find attribute values.
                //
                // RSC-5.1 (D-5.0.5): a quantity literal `num [unit]` folds to its
                // magnitude `value` plus a `unit` measurement reference. The
                // single source of truth (slot mint / model-level eval) reads
                // these props identically — folding (rather than leaving a child
                // expression subtree) keeps the attribute's declared value where
                // every consumer looks for it.
                let (mag, unit) = UsageExtraction::split_unit_annotation(value_expression);
                let text = mag.trim();
                if let Ok(i) = text.parse::<i64>() {
                    element.set_prop("value", Value::Int(i));
                } else if let Ok(f) = text.parse::<f64>() {
                    element.set_prop("value", Value::Float(f));
                } else if text == "true" {
                    element.set_prop("value", Value::Bool(true));
                } else if text == "false" {
                    element.set_prop("value", Value::Bool(false));
                } else {
                    // String or unrecognized — store as string
                    let s = text.trim_matches('"').to_owned();
                    element.set_prop("value", Value::String(s));
                }
                if let Some(unit) = unit {
                    element.set_prop("unit", Value::String(unit.to_owned()));
                }
            }
            // Phase 6D.1: complex expressions no longer populate
            // `unresolved_value`. The parser emits a structured expression
            // subtree via `process_expression`; elaboration re-hydrates the
            // legacy string prop during the migration (Phase 6D.2).
            if self.value_is_default {
                element.set_prop("isDefault", true);
            }
            if self.value_is_initial {
                element.set_prop("isInitial", true);
            }
        }

        // Apply flags
        if self.is_abstract {
            element.set_prop("isAbstract", true);
        }
        if self.is_variation {
            element.set_prop("isVariation", true);
        }
        if self.is_readonly {
            element.set_prop("isReadOnly", true);
        }
        if self.is_derived {
            element.set_prop("isDerived", true);
        }
        if self.is_end {
            element.set_prop("isEnd", true);
        }
        if self.is_reference {
            element.set_prop("isReference", true);
        }
        if self.is_composite {
            element.set_prop("isComposite", true);
        }
        if self.is_portion {
            element.set_prop("isPortion", true);
        }
        if self.is_variable {
            element.set_prop("isVariable", true);
        }
        if self.is_constant {
            element.set_prop("isConstant", true);
        }
        if self.is_individual {
            element.set_prop("isIndividual", true);
        }
        if let Some(ref portion_kind) = self.portion_kind {
            element.set_prop("portionKind", portion_kind.as_str());
        }

        if let Some(s) = span {
            element.spans.push(s);
        }

        element
    }
}

impl DefinitionExtraction {
    /// Build an Element from this extraction data.
    ///
    /// # Arguments
    ///
    /// * `kind` - The ElementKind for this definition (e.g., PartDefinition, ActionDefinition)
    /// * `span` - Optional span for source location tracking
    ///
    /// Canonical-key behaviour mirrors [`UsageExtraction::build_element`].
    pub fn build_element(&self, kind: ElementKind, span: Option<Span>) -> Element {
        let mut element = mint_element(
            kind,
            self.name.as_deref(),
            self.parent_key.as_ref(),
            self.sibling_index,
        );

        if let Some(ref name) = self.name {
            element.name = Some(name.clone());
        }

        if let Some(ref short_name) = self.short_name {
            element.set_prop("declaredShortName", short_name.clone());
        }

        if self.is_abstract {
            element.set_prop("isAbstract", true);
        }
        if self.is_variation {
            element.set_prop("isVariation", true);
        }

        if let Some(s) = span {
            element.spans.push(s);
        }

        element
    }
}

impl PackageExtraction {
    /// Build an Element from this extraction data.
    ///
    /// # Arguments
    ///
    /// * `kind` - The ElementKind (Package or LibraryPackage)
    /// * `span` - Optional span for source location tracking
    ///
    /// Canonical-key behaviour mirrors [`UsageExtraction::build_element`].
    pub fn build_element(&self, kind: ElementKind, span: Option<Span>) -> Element {
        let mut element = mint_element(
            kind,
            self.name.as_deref(),
            self.parent_key.as_ref(),
            self.sibling_index,
        );

        if let Some(ref name) = self.name {
            element.name = Some(name.clone());
        }

        if self.is_standard {
            element.set_prop("isStandard", true);
        }

        if let Some(s) = span {
            element.spans.push(s);
        }

        element
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn build_usage_element_basic() {
        let extraction = UsageExtraction {
            name: Some("myPart".to_string()),
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(element.kind, ElementKind::PartUsage);
        assert_eq!(element.name, Some("myPart".to_string()));
    }

    #[test]
    fn build_usage_element_with_flags() {
        let extraction = UsageExtraction {
            name: Some("abstractPart".to_string()),
            is_abstract: true,
            is_readonly: true,
            direction: Some("in".to_string()),
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element.get_prop("isAbstract").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isReadOnly").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("direction").and_then(|v| v.as_str()),
            Some("in")
        );
    }

    #[test]
    fn build_usage_element_with_multiplicity() {
        let extraction = UsageExtraction {
            name: Some("parts".to_string()),
            multiplicity: Some((0, None)), // [0..*]
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element
                .get_prop("multiplicity_lower")
                .and_then(|v| v.as_int()),
            Some(0)
        );
        assert_eq!(
            element
                .get_prop("multiplicity_upper")
                .and_then(|v| v.as_str()),
            Some("*")
        );
    }

    #[test]
    fn build_usage_element_with_multiplicity_modifiers() {
        let extraction = UsageExtraction {
            name: Some("parts".to_string()),
            multiplicity: Some((0, None)), // [0..*]
            is_ordered: true,
            is_nonunique: true,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element.get_prop("multiplicity").and_then(|v| v.as_str()),
            Some("0..* ordered nonunique")
        );
        assert_eq!(
            element.get_prop("isOrdered").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isUnique").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn build_usage_element_multiplicity_display_exact() {
        let extraction = UsageExtraction {
            name: Some("single".to_string()),
            multiplicity: Some((1, Some(1))),
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element.get_prop("multiplicity").and_then(|v| v.as_str()),
            Some("1")
        );
    }

    #[test]
    fn build_definition_element() {
        let extraction = DefinitionExtraction {
            name: Some("MyDefinition".to_string()),
            is_abstract: true,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartDefinition, None);

        assert_eq!(element.kind, ElementKind::PartDefinition);
        assert_eq!(element.name, Some("MyDefinition".to_string()));
        assert_eq!(
            element.get_prop("isAbstract").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn build_package_element() {
        let extraction = PackageExtraction {
            name: Some("MyPackage".to_string()),
            is_standard: false,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::Package, None);

        assert_eq!(element.kind, ElementKind::Package);
        assert_eq!(element.name, Some("MyPackage".to_string()));
    }

    #[test]
    fn build_library_package_element() {
        let extraction = PackageExtraction {
            name: Some("Base".to_string()),
            is_standard: true,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::LibraryPackage, None);

        assert_eq!(element.kind, ElementKind::LibraryPackage);
        assert_eq!(
            element.get_prop("isStandard").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn build_usage_element_with_reference_and_composite() {
        let extraction = UsageExtraction {
            name: Some("refPart".to_string()),
            is_reference: true,
            is_composite: false,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element.get_prop("isReference").and_then(|v| v.as_bool()),
            Some(true),
            "is_reference should produce isReference property"
        );
        // isComposite should not be set when false (default)
        assert!(
            element.get_prop("isComposite").is_none(),
            "isComposite should not be set when false"
        );
    }

    #[test]
    fn build_usage_element_with_all_modifiers() {
        let extraction = UsageExtraction {
            name: Some("full".to_string()),
            is_reference: true,
            is_composite: true,
            is_portion: true,
            is_variable: true,
            is_constant: true,
            ..Default::default()
        };

        let element = extraction.build_element(ElementKind::PartUsage, None);

        assert_eq!(
            element.get_prop("isReference").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isComposite").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isPortion").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isVariable").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            element.get_prop("isConstant").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    // === Canonical-key threading (ADR-009 / S1) ===

    #[test]
    fn build_usage_element_without_parent_key_uses_fresh_uuid() {
        // Today's behaviour: no parent_key → fresh uuid each call.
        let extraction = UsageExtraction {
            name: Some("part".to_string()),
            ..Default::default()
        };

        let a = extraction.build_element(ElementKind::PartUsage, None);
        let b = extraction.build_element(ElementKind::PartUsage, None);

        assert_ne!(a.id, b.id, "without parent_key, ids must be fresh");
    }

    #[test]
    fn build_usage_element_with_parent_key_named_is_stable() {
        let parent = CanonicalKey::for_named(&CanonicalKey::root("p"), "Package", "Pkg");
        let extraction = UsageExtraction {
            name: Some("part".to_string()),
            parent_key: Some(parent.clone()),
            ..Default::default()
        };

        let a = extraction.build_element(ElementKind::PartUsage, None);
        let b = extraction.build_element(ElementKind::PartUsage, None);

        // Same key → same id across calls.
        assert_eq!(a.id, b.id);
        // And it matches the canonical key derivation explicitly.
        let expected_key = CanonicalKey::for_named(&parent, "PartUsage", "part");
        assert_eq!(a.id, expected_key.to_element_id());
    }

    #[test]
    fn build_usage_element_with_parent_key_anonymous_uses_sibling_index() {
        let parent = CanonicalKey::for_named(&CanonicalKey::root("p"), "Package", "Pkg");

        // Anonymous: name is None; sibling_index distinguishes siblings.
        let zero = UsageExtraction {
            name: None,
            parent_key: Some(parent.clone()),
            sibling_index: Some(0),
            ..Default::default()
        }
        .build_element(ElementKind::ReferenceUsage, None);

        let one = UsageExtraction {
            name: None,
            parent_key: Some(parent.clone()),
            sibling_index: Some(1),
            ..Default::default()
        }
        .build_element(ElementKind::ReferenceUsage, None);

        assert_ne!(zero.id, one.id, "different sibling indices must differ");

        // And `0` matches the explicit anonymous key.
        let expected_zero = CanonicalKey::for_anonymous(&parent, "ReferenceUsage", 0);
        assert_eq!(zero.id, expected_zero.to_element_id());
    }

    #[test]
    fn build_definition_element_with_parent_key_is_stable() {
        let parent = CanonicalKey::root("p");
        let extraction = DefinitionExtraction {
            name: Some("MyDef".to_string()),
            parent_key: Some(parent.clone()),
            ..Default::default()
        };

        let a = extraction.build_element(ElementKind::PartDefinition, None);
        let b = extraction.build_element(ElementKind::PartDefinition, None);

        assert_eq!(a.id, b.id);
        let expected = CanonicalKey::for_named(&parent, "PartDefinition", "MyDef");
        assert_eq!(a.id, expected.to_element_id());
    }

    #[test]
    fn build_package_element_with_parent_key_is_stable() {
        let parent = CanonicalKey::root("p");
        let extraction = PackageExtraction {
            name: Some("Pkg".to_string()),
            is_standard: false,
            parent_key: Some(parent.clone()),
            sibling_index: None,
        };

        let a = extraction.build_element(ElementKind::Package, None);
        let b = extraction.build_element(ElementKind::Package, None);

        assert_eq!(a.id, b.id);
        let expected = CanonicalKey::for_named(&parent, "Package", "Pkg");
        assert_eq!(a.id, expected.to_element_id());
    }
}
