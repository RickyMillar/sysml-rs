//! Canonical-key derivation + ownership-aware element insertion.
//!
//! Implements the ADR-009 `CanonicalKey`-based ID minting strategy for
//! anonymous elements: each child element's ID is derived from
//! `(parent_canonical_key, kind, sibling_index)`, where `sibling_index`
//! is allocated per-parent, per-kind via the builder's `sibling_counters`
//! map. Named elements use `(parent_canonical_key, kind, name)` instead;
//! the sibling slot is still consumed so the anonymous-index space stays
//! monotonic and free of accidental collisions.

use super::AstBuilder;
use sysml_core::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph, VisibilityKind};

impl<'a> AstBuilder<'a> {
    /// Allocate a sibling slot for `kind` under `parent_id`. Returns the
    /// zero-based index used by `CanonicalKey::for_anonymous`. Must be called
    /// in `OwningMembership` order (i.e. exactly once per element minted at
    /// this parent, in the order they are added to the graph).
    pub(super) fn next_sibling_index(
        &mut self,
        parent_id: &Option<ElementId>,
        kind: &ElementKind,
    ) -> usize {
        let frame = self
            .sibling_counters
            .entry(parent_id.clone())
            .or_default();
        let kind_str: &'static str = kind.as_str();
        let entry = frame.entry(kind_str).or_insert(0);
        let idx = *entry;
        *entry += 1;
        idx
    }

    /// Compute the canonical key for a child of `parent_key`/`parent_id`,
    /// allocating the sibling slot for `kind` regardless of whether the child
    /// is named. The named branch consumes a slot too so the anonymous-index
    /// space stays monotonic and free of accidental collisions.
    ///
    /// Returns `(child_key, sibling_index_for_anonymous_fallback)`. The
    /// second tuple element is `Some(idx)` iff this child was minted with
    /// `for_anonymous` (either because `name` is `None`, or because `name`
    /// was already used by an earlier sibling at the same `(parent_id, kind)`
    /// and we fell back to anonymous keying so the duplicate stays distinct
    /// in the graph). Callers populating `*Extraction::sibling_index` must
    /// pass this through so the extraction-build path picks the same key.
    pub(super) fn child_canonical_key(
        &mut self,
        parent_key: &CanonicalKey,
        parent_id: &Option<ElementId>,
        kind: &ElementKind,
        name: Option<&str>,
    ) -> (CanonicalKey, Option<usize>) {
        let kind_str: &'static str = kind.as_str();
        let sibling_idx = self.next_sibling_index(parent_id, kind);
        match name {
            Some(n) => {
                let seen = self
                    .seen_named_keys
                    .entry((parent_id.clone(), kind_str))
                    .or_default();
                if seen.insert(n.to_string()) {
                    (CanonicalKey::for_named(parent_key, kind_str, n), None)
                } else {
                    (
                        CanonicalKey::for_anonymous(parent_key, kind_str, sibling_idx),
                        Some(sibling_idx),
                    )
                }
            }
            None => (
                CanonicalKey::for_anonymous(parent_key, kind_str, sibling_idx),
                Some(sibling_idx),
            ),
        }
    }

    /// Resolve the canonical key for the current scope. When the work item's
    /// `parent_key` is `None` (entry into the walk), fall back to
    /// `CanonicalKey::root(file_path)` so top-level mints are still derived
    /// from a stable, project-scoped key.
    pub(super) fn resolve_parent_key(&self, parent_key: Option<&CanonicalKey>) -> CanonicalKey {
        parent_key
            .cloned()
            .unwrap_or_else(|| CanonicalKey::root(self.root_scope))
    }

    /// Compute `(parent_key, child_canonical_key, sibling_index)` for a
    /// new element of `kind` under `parent_id`/`parent_key` with the given
    /// `name`. Allocates the sibling slot via `child_canonical_key`, and
    /// recovers the same index for the extraction-builder path so
    /// `build_element` mints the matching key.
    pub(super) fn prep_canonical_key(
        &mut self,
        parent_key: Option<&CanonicalKey>,
        parent_id: &Option<ElementId>,
        kind: &ElementKind,
        name: Option<&str>,
    ) -> (CanonicalKey, CanonicalKey, Option<usize>) {
        let parent_key = self.resolve_parent_key(parent_key);
        let (child_key, sibling_index) =
            self.child_canonical_key(&parent_key, parent_id, kind, name);
        (parent_key, child_key, sibling_index)
    }

    /// Mint a directly-built (non-extraction) element with a canonical-key
    /// derived ID. Returns `(child_canonical_key, Element)` so the caller can
    /// finish populating the element before adding it to the graph.
    pub(super) fn mint_direct_element(
        &mut self,
        parent_key: Option<&CanonicalKey>,
        parent_id: &Option<ElementId>,
        kind: ElementKind,
        name: Option<&str>,
    ) -> (CanonicalKey, Element) {
        let (_, child_key, _) = self.prep_canonical_key(parent_key, parent_id, &kind, name);
        let elem = Element::new_with_key(kind, &child_key);
        (child_key, elem)
    }

    /// Add an element under its parent, minting the wrapping
    /// `OwningMembership` with a reparse-stable id derived from
    /// `(owner_key, "OwningMembership", child_key, 0)` per ADR-009
    /// §Relationships.
    ///
    /// `parent_key` is `None` for top-level mints; we synthesise the
    /// canonical root key (`CanonicalKey::root(file_path)`) so the
    /// wrapping membership of any orphan is still keyed deterministically.
    pub(super) fn add_with_ownership_keyed(
        &self,
        element: Element,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        child_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) -> ElementId {
        match parent_id {
            Some(pid) => {
                let owner_key = self.resolve_parent_key(parent_key);
                graph.add_owned_element_with_key(
                    element,
                    pid.clone(),
                    VisibilityKind::Public,
                    &owner_key,
                    child_key,
                )
            }
            None => graph.add_element(element),
        }
    }

    /// Like [`add_with_ownership_keyed`](Self::add_with_ownership_keyed) but
    /// wraps the element in a caller-chosen `OwningMembership` **subtype**
    /// (e.g. `StateSubactionMembership`) so the spec-faithful membership kind
    /// is materialized rather than a plain `OwningMembership`. The wrapping
    /// membership is keyed on `(owner_key, membership_kind, child_key, 0)` per
    /// ADR-009, exactly as the plain variant, so ids stay reparse-stable.
    ///
    /// `membership_kind` must be `OwningMembership` or a subtype (asserted in
    /// `create_owned_membership_with_key`). The owned element — not the
    /// membership — is what lands in `owner_to_children`, so `children_of`
    /// walks are unaffected by the membership-kind choice.
    pub(super) fn add_with_membership_kind_keyed(
        &self,
        element: Element,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        child_key: &CanonicalKey,
        membership_kind: ElementKind,
        graph: &mut ModelGraph,
    ) -> ElementId {
        match parent_id {
            Some(pid) => {
                let owner_key = self.resolve_parent_key(parent_key);
                graph.add_owned_element_with_membership_kind_key(
                    element,
                    pid.clone(),
                    membership_kind,
                    VisibilityKind::Public,
                    &owner_key,
                    child_key,
                )
            }
            None => graph.add_element(element),
        }
    }
}
