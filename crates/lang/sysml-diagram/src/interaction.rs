//! Interaction descriptors (Bucket 1.5) — renderer-agnostic *semantic
//! affordances*, joined to scene regions by `ElementId`.
//!
//! The goal of the interaction layer is to let a frontend build context menus /
//! navigation affordances **generically**, without hardcoding SysML knowledge.
//! The split is deliberate and respects the crate layering:
//!
//! - This module (Layer 3, `sysml-diagram`) emits only **semantic facts** about
//!   each element that a renderer can act on — facts that are a pure function of
//!   the `ModelGraph`. It NEVER names a service command: the `#[service_command]`
//!   registry lives in `sysml-service` (above `sysml-ide-db`), which this crate
//!   cannot and must not see (principle #4 — command policy has one home).
//! - The **command/label annotation** (mapping each affordance to a registered
//!   command for the current session) is a thin overlay applied by the service
//!   layer when it serves the `ViewModel` (Bucket 1.7). It is *not* baked into
//!   this salsa-cached artifact, which stays session-context-free.
//!
//! Like the text-map (1.6), this is a **sidecar keyed by `ElementId::to_string()`**
//! that the frontend joins against `DiagramNode::element_id`. It deliberately does
//! **not** re-store anything the scene already owns (`element_kind`, `node_kind`,
//! `expanded`) — that would be a second home for scene data (principle #5).
//!
//! ## Scope (steward-ruled (A), 2026-06-25)
//!
//! The steward trimmed the affordance set hard (burden of proof on inclusion):
//! - `Selectable` / `Hoverable` — **implicit**: every scene region with an
//!   `element_id` is both; no flag needed.
//! - `Expandable` / `Collapsible` — **already on the scene** (`DiagramNode::
//!   expanded: Option<bool>`); read it there, don't duplicate.
//! - `HasSource` — **owned by the text-map**: presence of a span is the signal.
//! - `Drillable` ("has its own sub-view") — **tool policy**, emitted by the 1.7
//!   service layer (it knows which `(ElementId, ViewType)` views exist), not a
//!   model fact.
//! - `hover_target` (proxy indirection) — **deferred**: collapsed dual-projection
//!   nodes keep `element_id == self` (§F-2), so there is no indirection to record
//!   today; the only distinct-id regions are *synthetic* nodes (n-ary dot,
//!   sequence proxies, IBD context ports) whose represented element would have to
//!   be declared by the generator at mint time (a `DiagramNode` addition) —
//!   reverse-parsing the synthetic id strings is rejected. Lands when a generator
//!   surfaces the represented id.
//!
//! What remains is the one fully-correct, generator-agnostic, spec-grounded
//! affordance: [`InteractionEntry::type_definition`].

use std::collections::HashMap;

use sysml_core::resolution::scoping::chaining::find_feature_type;
use sysml_core::{ElementId, ModelGraph};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The semantic interaction affordances for one element. Sparse: an element only
/// appears in the [`InteractionMap`] when it carries at least one affordance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct InteractionEntry {
    /// The resolved typing classifier of this usage — the **go-to-definition**
    /// target (KerML `typed_by`, SysML CONFORMS-REQUIRED). A pure read of the
    /// elaborated graph's `FeatureTyping` reverse index. The 1.7 service layer
    /// maps this to a navigation command with this id as the parameter.
    pub type_definition: Option<ElementId>,
}

impl InteractionEntry {
    /// Whether this entry carries any affordance worth storing.
    fn is_meaningful(&self) -> bool {
        self.type_definition.is_some()
    }
}

/// Map from a scene node id (`ElementId::to_string()`) to its interaction
/// affordances. Joined to the scene by id, exactly like [`crate::TextMap`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct InteractionMap {
    entries: HashMap<String, InteractionEntry>,
}

impl InteractionMap {
    /// The affordances for a scene node id, if any.
    pub fn entry(&self, element_id: &str) -> Option<&InteractionEntry> {
        self.entries.get(element_id)
    }

    /// The go-to-definition target for a scene node id, if it is a typed usage.
    pub fn type_definition(&self, element_id: &str) -> Option<&ElementId> {
        self.entries
            .get(element_id)
            .and_then(|e| e.type_definition.as_ref())
    }

    /// Number of elements carrying affordances.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate `(node_id, entry)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &InteractionEntry)> {
        self.entries.iter()
    }
}

/// Build the [`InteractionMap`] for a model graph. Pure function of the graph
/// (view-independent — a model fact), keyed by `ElementId::to_string()` to match
/// scene node ids. Sparse: only elements with a resolved feature type are stored.
pub fn build_interaction_map(graph: &ModelGraph) -> InteractionMap {
    let mut entries = HashMap::new();
    for (id, _element) in &graph.elements {
        // O(1) resolved-typing lookup via the graph's FeatureTyping reverse
        // index. Returns `None` for non-features and unresolved usages.
        let type_definition = find_feature_type(graph, id).filter(|def| def != id);
        let entry = InteractionEntry { type_definition };
        if entry.is_meaningful() {
            entries.insert(id.to_string(), entry);
        }
    }
    InteractionMap { entries }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_yields_empty_map() {
        let map = build_interaction_map(&ModelGraph::new());
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn accessors_resolve_stored_entry() {
        let def = ElementId::from_string("def-1");
        let mut entries = HashMap::new();
        entries.insert(
            "usage-1".to_string(),
            InteractionEntry {
                type_definition: Some(def.clone()),
            },
        );
        let map = InteractionMap { entries };

        assert_eq!(map.len(), 1);
        assert_eq!(map.type_definition("usage-1"), Some(&def));
        assert!(map.entry("usage-1").is_some());
        // Untyped / absent ids resolve to None.
        assert_eq!(map.type_definition("nope"), None);
        assert!(map.entry("nope").is_none());
    }

    #[test]
    fn sparse_entries_skip_untyped() {
        // An entry with no affordance is not "meaningful" and is never stored.
        let empty = InteractionEntry {
            type_definition: None,
        };
        assert!(!empty.is_meaningful());
    }
}
