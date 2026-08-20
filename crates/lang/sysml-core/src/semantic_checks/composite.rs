//! Effective compositeness of a usage under the SysML default-composite rule.
//!
//! The textual notation materializes `isComposite` only on an explicit `composite`
//! keyword and `isReference` only on `ref`; the default declaration (no keyword)
//! sets neither property. Composition is an occurrence concept — SysML §8.9.2
//! notes that "the concept of composition only applies to occurrences" — so an
//! `OccurrenceUsage` owned without `ref` is composite by default, while data /
//! reference usages (`AttributeUsage`, `ReferenceUsage`) are referential by their
//! nature and are never composite.

use crate::{Element, ElementKind, Value};

/// Whether `element` is *effectively* composite. A *directed* feature
/// (`in` / `out` / `inout`) is never composite; otherwise an explicit
/// `isComposite` property is authoritative; otherwise an explicit
/// `isReference` makes it referential; otherwise it is composite iff it is an
/// occurrence usage.
///
/// Composition is restricted to undirected features. KerML derives
/// `Feature::isComposite` so that a feature with a non-`none` direction is
/// referential — the sysml-2ls reference LSP computes it as
/// `… && direction === "none"`, and the SysML port well-formedness
/// constraints (§8.3.12.5 / §8.3.12.6, S145/S146) only reject *undirected*
/// composite port members. Without this guard a directed port payload feature
/// such as `out item power : ACPhase` — the standard port idiom used
/// throughout the spec's own examples — is misclassified as composite and
/// falsely flagged.
pub fn is_effectively_composite(element: &Element) -> bool {
    // Directed (in/out/inout) features are referential, never composite.
    if matches!(
        element.get_prop("direction").and_then(Value::as_str),
        Some("in" | "out" | "inout")
    ) {
        return false;
    }
    match element.get_prop("isComposite").and_then(Value::as_bool) {
        Some(explicit) => explicit,
        None => {
            if element.get_prop("isReference").and_then(Value::as_bool) == Some(true) {
                return false;
            }
            element.kind == ElementKind::OccurrenceUsage
                || element.kind.is_subtype_of(ElementKind::OccurrenceUsage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    fn usage(kind: ElementKind) -> Element {
        Element::new(ElementId::new_v4(), kind)
    }

    #[test]
    fn part_usage_is_composite_by_default() {
        assert!(is_effectively_composite(&usage(ElementKind::PartUsage)));
    }

    #[test]
    fn attribute_usage_is_referential_by_default() {
        assert!(!is_effectively_composite(&usage(ElementKind::AttributeUsage)));
    }

    #[test]
    fn reference_usage_is_referential_by_default() {
        assert!(!is_effectively_composite(&usage(ElementKind::ReferenceUsage)));
    }

    #[test]
    fn directed_item_is_not_composite() {
        // `out item power : ACPhase` — a directed port payload feature is
        // referential even though ItemUsage is an occurrence usage. (S146)
        let e = usage(ElementKind::ItemUsage).with_prop("direction", "out");
        assert!(!is_effectively_composite(&e));
        let e_in = usage(ElementKind::ItemUsage).with_prop("direction", "in");
        assert!(!is_effectively_composite(&e_in));
        // Direction overrides even an explicit composite keyword.
        let e_both = usage(ElementKind::ItemUsage)
            .with_prop("direction", "out")
            .with_prop("isComposite", true);
        assert!(!is_effectively_composite(&e_both));
    }

    #[test]
    fn undirected_item_is_composite() {
        // `item power : ACPhase` (no direction) stays composite → still flagged.
        assert!(is_effectively_composite(&usage(ElementKind::ItemUsage)));
    }

    #[test]
    fn ref_part_is_not_composite() {
        let e = usage(ElementKind::PartUsage).with_prop("isReference", true);
        assert!(!is_effectively_composite(&e));
    }

    #[test]
    fn explicit_is_composite_wins() {
        // An explicit isComposite=false on a part overrides the occurrence default.
        let e = usage(ElementKind::PartUsage).with_prop("isComposite", false);
        assert!(!is_effectively_composite(&e));
        let e2 = usage(ElementKind::AttributeUsage).with_prop("isComposite", true);
        assert!(is_effectively_composite(&e2));
    }
}
