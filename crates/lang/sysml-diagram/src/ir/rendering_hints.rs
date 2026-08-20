//! Rendering hints — generator-level overrides for ELK layout options.
//!
//! Hints exist so a single generator can serve multiple layout
//! presentations of the same view kind. Example: a constraint-binding
//! ("parametric") presentation is `InterconnectionView` with
//! `direction = RIGHT`. Rather than a peer generator that differs only in
//! its `elk_direction()`, it's `Interconnection + RenderingHints
//! { direction: Some("RIGHT") }` — same generator, different layout.
//!
//! Hints override the generator's default layout options. Absent hint
//! fields leave the generator's defaults intact. The contract is purely
//! additive: applying empty hints to an IR is a no-op.

use std::collections::BTreeMap;

/// Optional ELK layout overrides applied to a generator's output.
///
/// Generators set their own defaults (via `elk_algorithm()` / their
/// `ir.layout_algorithm` / `ir.graph_layout_options`). After collection,
/// the generator calls [`crate::ir::GeneratorContext::apply_hints`],
/// which overlays any present hint fields onto the IR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RenderingHints {
    /// ELK layout algorithm (overrides `ir.layout_algorithm`).
    /// Example: `"org.eclipse.elk.layered"`, `"org.eclipse.elk.fixed"`.
    pub algorithm: Option<String>,

    /// ELK layout direction (overrides `elk.direction`).
    /// Example: `"DOWN"`, `"RIGHT"`.
    pub direction: Option<String>,

    /// Inter-node spacing (overrides `elk.spacing.nodeNode`).
    pub spacing_node_node: Option<String>,

    /// Arbitrary `elk.*` overrides applied last. Use this for hint
    /// fields that haven't been promoted to first-class above.
    pub extra: BTreeMap<String, String>,
}

impl RenderingHints {
    /// New empty hints — `apply` is a no-op.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_algorithm(mut self, algo: impl Into<String>) -> Self {
        self.algorithm = Some(algo.into());
        self
    }

    pub fn with_direction(mut self, dir: impl Into<String>) -> Self {
        self.direction = Some(dir.into());
        self
    }

    pub fn with_spacing_node_node(mut self, spacing: impl Into<String>) -> Self {
        self.spacing_node_node = Some(spacing.into());
        self
    }

    pub fn with_extra(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// True if no override is set.
    pub fn is_empty(&self) -> bool {
        self.algorithm.is_none()
            && self.direction.is_none()
            && self.spacing_node_node.is_none()
            && self.extra.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hints_is_empty() {
        let h = RenderingHints::new();
        assert!(h.is_empty());
    }

    #[test]
    fn builders_set_fields() {
        let h = RenderingHints::new()
            .with_algorithm("org.eclipse.elk.layered")
            .with_direction("RIGHT")
            .with_spacing_node_node("40")
            .with_extra("elk.layered.nodePlacement.strategy", "BRANDES_KOEPF");
        assert_eq!(h.algorithm.as_deref(), Some("org.eclipse.elk.layered"));
        assert_eq!(h.direction.as_deref(), Some("RIGHT"));
        assert_eq!(h.spacing_node_node.as_deref(), Some("40"));
        assert_eq!(
            h.extra.get("elk.layered.nodePlacement.strategy"),
            Some(&"BRANDES_KOEPF".to_owned())
        );
        assert!(!h.is_empty());
    }
}
