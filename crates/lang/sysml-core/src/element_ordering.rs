//! Shared utilities for ordering and locating elements by source position.
//!
//! These helpers are used by health-check passes across multiple crates
//! (import_health, run-actions, run-statemachine, run-flows, run-cases)
//! to sort elements deterministically by their source spans.

use crate::Element;
use sysml_span::Span;

/// Sort a slice of element references by their first source span offset.
///
/// Elements without spans sort to the end. Ties are broken first by name,
/// then by stringified element ID for full determinism.
pub fn sort_elements_by_source_order(elements: &mut Vec<&Element>) {
    elements.sort_by(|a, b| {
        let a_start = a.spans.first().map(|s| s.start).unwrap_or(usize::MAX);
        let b_start = b.spans.first().map(|s| s.start).unwrap_or(usize::MAX);
        a_start
            .cmp(&b_start)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    });
}

/// Return the primary (first) span of an element, or a synthetic span if none.
pub fn primary_span(element: &Element) -> Span {
    element
        .spans
        .first()
        .cloned()
        .unwrap_or_else(Span::synthetic)
}
