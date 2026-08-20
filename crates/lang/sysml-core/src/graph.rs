//! `ModelGraph`: the universal in-memory model database for sysml-rs.
//!
//! Stores all elements and relationships plus the reverse indexes that
//! make name resolution, ownership traversal, and library lookup O(1).

use rustc_hash::{FxHashMap, FxHashSet};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use sysml_id::ElementId;

use crate::{Element, ElementKind, Relationship, RelationshipKind};

/// A graph of model elements and relationships.
#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ModelGraph {
    /// All elements in the graph, keyed by id.
    ///
    /// Backed by `FxHashMap` for O(1) lookup; UUID ordering is not observably
    /// needed by any consumer (canonical JSON, diagram, snapshot tests all
    /// sort explicitly). See perf commit history for the swap from BTreeMap.
    pub elements: FxHashMap<ElementId, Element>,
    /// All relationships in the graph, keyed by id.
    pub relationships: FxHashMap<ElementId, Relationship>,

    // Indexes (built lazily, not serialized)
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) owner_to_children: FxHashMap<ElementId, FxHashSet<ElementId>>,
    #[cfg_attr(feature = "serde", serde(skip))]
    source_to_rels: FxHashMap<ElementId, FxHashSet<ElementId>>,
    #[cfg_attr(feature = "serde", serde(skip))]
    target_to_rels: FxHashMap<ElementId, FxHashSet<ElementId>>,
    /// Reverse index: RelationshipKind -> relationship IDs of that kind, in
    /// insertion order. Enables `relationships_by_kind` to skip the O(|rels|)
    /// scan that previously ran inside per-element elaboration loops (O(n²)).
    #[cfg_attr(feature = "serde", serde(skip))]
    relationship_kind_index: FxHashMap<RelationshipKind, Vec<ElementId>>,

    // NEW: Membership-based ownership indexes
    /// Maps namespace ID to its membership element IDs.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) namespace_to_memberships: FxHashMap<ElementId, FxHashSet<ElementId>>,
    /// Maps element ID to its owning membership element ID.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) element_to_owning_membership: FxHashMap<ElementId, ElementId>,

    // Phase 1 Performance Optimization: Reverse indexes for O(1) relationship lookups
    /// Maps typed feature ID to FeatureTyping element IDs that type it.
    /// Used by find_feature_type() and find_feature_types() for O(1) lookup.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) typed_feature_to_typings: FxHashMap<ElementId, Vec<ElementId>>,
    /// Maps specific type ID to Specialization element IDs where it is the specific type.
    /// Used by find_general_types() for O(1) lookup.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) specific_to_specializations: FxHashMap<ElementId, Vec<ElementId>>,

    /// Set of root package IDs that are standard library packages.
    /// Library packages are available globally during name resolution.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "FxHashSet::is_empty")
    )]
    library_packages: FxHashSet<ElementId>,

    /// Pre-built index: name -> ElementId for all library members.
    /// This enables O(1) lookup instead of O(n) recursive search.
    /// Built lazily when library lookup is first needed.
    #[cfg_attr(feature = "serde", serde(skip))]
    library_name_index: FxHashMap<String, ElementId>,

    /// Whether the library name index needs to be rebuilt.
    /// Set to true when library packages change or elements are added/removed.
    #[cfg_attr(feature = "serde", serde(skip))]
    library_index_dirty: bool,

    /// Reverse index: ElementKind -> list of ElementIds of that kind.
    /// Enables O(1) lookup of all elements by kind instead of O(n) scan.
    #[cfg_attr(feature = "serde", serde(skip))]
    kind_index: FxHashMap<ElementKind, Vec<ElementId>>,

    /// Reverse index: element name -> list of ElementIds with that name.
    /// Enables O(1) lookup by name instead of O(n) scan.
    #[cfg_attr(feature = "serde", serde(skip))]
    name_index: FxHashMap<String, Vec<ElementId>>,

    /// Cached list of root element IDs (elements with no owner).
    /// Avoids O(n) scan of all elements when iterating roots.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) root_ids: Vec<ElementId>,

    #[cfg_attr(feature = "serde", serde(skip))]
    indexes_dirty: bool,

    /// Cache for `resolve_with_feature_chaining` results: `(scope, name) -> Option<resolved_id>`.
    ///
    /// The graph does not mutate during a simulation step, so resolution results
    /// for a given (scope_element, name) pair are stable across the whole session.
    /// The cache is naturally invalidated when the graph is replaced (Arc identity
    /// changes), or explicitly via `invalidate_resolution_cache()`.
    ///
    /// Profiler: `ResolutionContext::resolve_name` family was ~91% of session-step
    /// CPU on a large multi-subsystem workload before this cache was introduced.
    #[cfg_attr(feature = "serde", serde(skip))]
    resolution_cache: std::sync::RwLock<FxHashMap<(ElementId, String), Option<ElementId>>>,

    /// Lazily-computed cache of `fingerprint()`.
    ///
    /// `fingerprint()` is called on every salsa probe that hashes the graph
    /// (parse/analysis/resolution durability). Computing it from scratch is
    /// O(n log n) over every element (~75k for the merged stdlib graph), so we
    /// cache the result and invalidate it on every `&mut self` mutation entry
    /// point (add/merge/clear/rebuild/`get_element_mut`). The graph is frozen
    /// once construction/elaboration finishes, so the common case is one
    /// recompute followed by O(1) cache hits for the rest of the session.
    #[cfg_attr(feature = "serde", serde(skip))]
    fingerprint_cache: std::sync::RwLock<Option<u64>>,

    /// Whether [`crate::elaborate::elaborate`] has run over this graph's current
    /// content.
    ///
    /// Set by `elaborate`/`elaborate_with_library` as their last act, and
    /// cleared by every content mutation (`add_element`, `add_relationship`,
    /// `get_element_mut`, `clear`, `merge`, `merge_from_ref`) so it can never
    /// claim "elaborated" for content elaboration has not seen. Index rebuilds
    /// (`rebuild_indexes`) do NOT touch it — they derive nothing new about the
    /// content, and the production salsa path runs `rebuild_indexes` right after
    /// `elaborate`, so clearing here would defeat the marker.
    ///
    /// Trusted by `ModelCompiler::from_arc` to skip a redundant re-elaborate of
    /// an already-elaborated (salsa workspace) graph (RSC-6.1). Not serialized:
    /// a deserialized graph is parse-only until re-elaborated, so `false` (the
    /// `serde(skip)` default) is the correct restored state.
    #[cfg_attr(feature = "serde", serde(skip))]
    is_elaborated: bool,
}

// `RwLock` is not `Clone`; we manually implement `Clone` to reset the cache.
impl Clone for ModelGraph {
    fn clone(&self) -> Self {
        ModelGraph {
            elements: self.elements.clone(),
            relationships: self.relationships.clone(),
            owner_to_children: self.owner_to_children.clone(),
            source_to_rels: self.source_to_rels.clone(),
            target_to_rels: self.target_to_rels.clone(),
            relationship_kind_index: self.relationship_kind_index.clone(),
            namespace_to_memberships: self.namespace_to_memberships.clone(),
            element_to_owning_membership: self.element_to_owning_membership.clone(),
            typed_feature_to_typings: self.typed_feature_to_typings.clone(),
            specific_to_specializations: self.specific_to_specializations.clone(),
            library_packages: self.library_packages.clone(),
            library_name_index: self.library_name_index.clone(),
            library_index_dirty: self.library_index_dirty,
            kind_index: self.kind_index.clone(),
            name_index: self.name_index.clone(),
            root_ids: self.root_ids.clone(),
            indexes_dirty: self.indexes_dirty,
            // Reset the cache on clone — entries are tied to the source graph's
            // contents, but cloned graphs may diverge after mutation.
            resolution_cache: std::sync::RwLock::new(FxHashMap::default()),
            // Reset the fingerprint cache; the clone recomputes lazily on demand.
            fingerprint_cache: std::sync::RwLock::new(None),
            // A clone has the same content as its source, so it carries the
            // same elaboration state.
            is_elaborated: self.is_elaborated,
        }
    }
}

impl ModelGraph {
    /// Create a new empty model graph.
    pub fn new() -> Self {
        ModelGraph {
            elements: FxHashMap::default(),
            relationships: FxHashMap::default(),
            owner_to_children: FxHashMap::default(),
            source_to_rels: FxHashMap::default(),
            target_to_rels: FxHashMap::default(),
            relationship_kind_index: FxHashMap::default(),
            namespace_to_memberships: FxHashMap::default(),
            element_to_owning_membership: FxHashMap::default(),
            typed_feature_to_typings: FxHashMap::default(),
            specific_to_specializations: FxHashMap::default(),
            library_packages: FxHashSet::default(),
            library_name_index: FxHashMap::default(),
            library_index_dirty: true,
            kind_index: FxHashMap::default(),
            name_index: FxHashMap::default(),
            root_ids: Vec::new(),
            indexes_dirty: false,
            resolution_cache: std::sync::RwLock::new(FxHashMap::default()),
            fingerprint_cache: std::sync::RwLock::new(None),
            is_elaborated: false,
        }
    }

    /// Returns whether [`crate::elaborate::elaborate`] has run over the graph's
    /// current content. See the `is_elaborated` field docs.
    #[inline]
    pub fn is_elaborated(&self) -> bool {
        self.is_elaborated
    }

    /// Mark the graph as elaborated. Called by `elaborate`/`elaborate_with_library`
    /// as their final act, once all derived structure has been added. Callers
    /// that pre-elaborate a graph by other means may set it too; it is cleared
    /// automatically on the next content mutation.
    #[inline]
    pub fn mark_elaborated(&mut self) {
        self.is_elaborated = true;
    }

    /// Invalidate the lazily-computed fingerprint cache.
    ///
    /// Cheap: uses `get_mut()` (no locking — `&mut self` proves exclusive
    /// access), so it is safe to call on every mutation entry point.
    #[inline]
    fn invalidate_fingerprint(&mut self) {
        if let Ok(slot) = self.fingerprint_cache.get_mut() {
            *slot = None;
        }
    }

    /// Look up a `(scope, name)` resolution in the chaining cache.
    ///
    /// Returns:
    /// - `Some(Some(id))` — cached positive hit
    /// - `Some(None)` — cached negative hit
    /// - `None` — not in cache (caller must compute)
    ///
    /// Used by `resolution::scoping::chaining::resolve_with_feature_chaining`
    /// to short-circuit repeated lookups during simulation steps.
    pub fn resolution_cache_get(&self, scope: &ElementId, name: &str) -> Option<Option<ElementId>> {
        // Cheap probe: lock briefly, look up, drop.
        // Two-level Option: outer = "in cache?", inner = "found?".
        let cache = self.resolution_cache.read().ok()?;
        // Avoid allocating a String on every probe by constructing the key
        // as borrowed lookups via `.contains_key`/`.get` requires `&(EID, String)`,
        // so we still allocate on probe. Future optimisation: switch to Cow or interned names.
        cache.get(&(scope.clone(), name.to_owned())).cloned()
    }

    /// Insert a `(scope, name) -> result` entry into the chaining cache.
    pub fn resolution_cache_put(&self, scope: ElementId, name: String, value: Option<ElementId>) {
        if let Ok(mut cache) = self.resolution_cache.write() {
            cache.insert((scope, name), value);
            #[cfg(feature = "resolution-tracing")]
            tracing::debug!(target: "sysml_core::resolution_cache", "put: cache size {}", cache.len());
        }
    }

    /// Drop all cached chaining resolutions.
    ///
    /// Call this when the graph is mutated in a way that affects feature-chain
    /// resolution (added/removed elements, retyped features). Most callers don't
    /// need to call this manually — the cache is reset on `clone()`.
    pub fn invalidate_resolution_cache(&self) {
        if let Ok(mut cache) = self.resolution_cache.write() {
            cache.clear();
        }
    }

    /// Add an element to the graph.
    pub fn add_element(&mut self, element: Element) -> ElementId {
        let id = element.id.clone();

        // Update owner index and root_ids
        if let Some(owner) = &element.owner {
            self.owner_to_children
                .entry(owner.clone())
                .or_default()
                .insert(id.clone());
        } else {
            self.root_ids.push(id.clone());
        }

        // Update reverse indexes for FeatureTyping elements
        if element.kind == ElementKind::FeatureTyping
            || element.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(typed_feature) = element.props.get("typedFeature") {
                if let Some(tf_id) = typed_feature.as_ref() {
                    self.typed_feature_to_typings
                        .entry(tf_id.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }

        // Update reverse indexes for Specialization elements
        if element.kind == ElementKind::Specialization
            || element.kind.is_subtype_of(ElementKind::Specialization)
        {
            if let Some(specific) = element.props.get("specific") {
                if let Some(specific_id) = specific.as_ref() {
                    self.specific_to_specializations
                        .entry(specific_id.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }

        // Update kind index
        self.kind_index
            .entry(element.kind.clone())
            .or_default()
            .push(id.clone());

        // Update name index
        if let Some(name) = &element.name {
            self.name_index
                .entry(name.clone())
                .or_default()
                .push(id.clone());
        }

        self.elements.insert(id.clone(), element);
        // Resolution cache may be stale once elements change.
        // Invalidating in `add_*` keeps semantics conservative; in practice the
        // cache is populated only during simulation steps (post-elaboration),
        // so this is a no-op during initial graph construction.
        self.invalidate_resolution_cache();
        self.invalidate_fingerprint();
        self.is_elaborated = false;
        id
    }

    /// Add a relationship to the graph.
    pub fn add_relationship(&mut self, relationship: Relationship) -> ElementId {
        let id = relationship.id.clone();

        // Update source index
        self.source_to_rels
            .entry(relationship.source.clone())
            .or_default()
            .insert(id.clone());

        // Update target index
        self.target_to_rels
            .entry(relationship.target.clone())
            .or_default()
            .insert(id.clone());

        // Update kind index (insertion order preserved)
        self.relationship_kind_index
            .entry(relationship.kind.clone())
            .or_default()
            .push(id.clone());

        self.relationships.insert(id.clone(), relationship);
        self.invalidate_resolution_cache();
        self.invalidate_fingerprint();
        self.is_elaborated = false;
        id
    }

    /// Get an element by id.
    pub fn get_element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.get(id)
    }

    /// Get a mutable element by id.
    ///
    /// Conservatively invalidates the fingerprint cache, since the caller may
    /// mutate the element's `name`/`kind` (the fields the fingerprint hashes).
    pub fn get_element_mut(&mut self, id: &ElementId) -> Option<&mut Element> {
        self.invalidate_fingerprint();
        // The caller may change content the elaborator derived from.
        self.is_elaborated = false;
        self.elements.get_mut(id)
    }

    /// Get a relationship by id.
    pub fn get_relationship(&self, id: &ElementId) -> Option<&Relationship> {
        self.relationships.get(id)
    }

    /// Get the children of an owner element.
    pub fn children_of(&self, owner: &ElementId) -> impl Iterator<Item = &Element> {
        self.owner_to_children
            .get(owner)
            .into_iter()
            .flat_map(|children| children.iter())
            .filter_map(move |id| self.elements.get(id))
    }

    /// Get outgoing relationships from a source element.
    pub fn outgoing(&self, source: &ElementId) -> impl Iterator<Item = &Relationship> {
        self.source_to_rels
            .get(source)
            .into_iter()
            .flat_map(|rels| rels.iter())
            .filter_map(move |id| self.relationships.get(id))
    }

    /// Get incoming relationships to a target element.
    pub fn incoming(&self, target: &ElementId) -> impl Iterator<Item = &Relationship> {
        self.target_to_rels
            .get(target)
            .into_iter()
            .flat_map(|rels| rels.iter())
            .filter_map(move |id| self.relationships.get(id))
    }

    /// Get all elements of a specific kind.
    ///
    /// Uses the pre-built kind index for O(1) lookup when available,
    /// falling back to O(n) scan otherwise.
    pub fn elements_by_kind<'a>(
        &'a self,
        kind: &'a ElementKind,
    ) -> impl Iterator<Item = &'a Element> {
        self.element_ids_by_kind(kind)
            .iter()
            .filter_map(move |id| self.elements.get(id))
    }

    /// Get element IDs of a specific kind via the pre-built index.
    ///
    /// Returns an empty slice if no elements of that kind exist.
    /// This is O(1) and avoids iterating over all elements.
    pub fn element_ids_by_kind(&self, kind: &ElementKind) -> &[ElementId] {
        self.kind_index
            .get(kind)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Look up elements by name. Returns empty slice if no matches.
    ///
    /// This is O(1) via the pre-built name index.
    pub fn lookup_by_name(&self, name: &str) -> &[ElementId] {
        self.name_index
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Look up a root element by name in O(name-collisions) instead of
    /// O(|root_ids|). Used by `resolve_qualified_name_global` / `resolve_qname`
    /// for the first-segment lookup.
    ///
    /// Ridge B: post-Ridge-A.2 the workspace-merged graph absorbs every stdlib
    /// top-level package (Connections, ScalarValues, ISQ, SI, …) into its root
    /// list. The previous implementation iterated `root_ids` linearly per call;
    /// at the volume the resolver runs, that became 68.5 % exclusive on the
    /// workspace shape (May 29 §6 profile). This routes through `name_index`
    /// (one hashmap probe) then filters by `owner.is_none()` over the (small)
    /// hit list, so the per-call cost no longer scales with the merged root
    /// count.
    ///
    /// Falls back to a linear `roots()` scan only when the name index has no
    /// entry — handles names not in the index for any reason (e.g. quoted
    /// operator roots, though those don't occur today).
    pub fn lookup_root_by_name(&self, name: &str) -> Option<&Element> {
        for id in self.lookup_by_name(name) {
            if let Some(e) = self.elements.get(id) {
                if e.owner.is_none() {
                    return Some(e);
                }
            }
        }
        // Fallback: linear scan of roots for names not in name_index.
        self.roots().find(|e| {
            e.name
                .as_ref()
                .map(|n| n.trim_matches('\'') == name.trim_matches('\''))
                .unwrap_or(false)
        })
    }

    /// Get all relationships of a specific kind.
    ///
    /// Uses the pre-built `relationship_kind_index` for O(matches) lookup
    /// instead of an O(|rels|) scan. Yields relationships in insertion order.
    pub fn relationships_by_kind<'a>(
        &'a self,
        kind: &'a RelationshipKind,
    ) -> impl Iterator<Item = &'a Relationship> {
        self.relationship_kind_index
            .get(kind)
            .map(|ids| ids.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter_map(move |id| self.relationships.get(id))
            // Re-check kind: the index trusts its buckets, but this guards
            // against a stale bucket entry should a relationship ever be
            // re-added under a different kind (matches the old scan predicate).
            .filter(move |r| &r.kind == kind)
    }

    /// Get all root element IDs.
    pub fn root_ids(&self) -> &[ElementId] {
        &self.root_ids
    }

    /// Get all root elements (elements without an owner).
    pub fn roots(&self) -> impl Iterator<Item = &Element> {
        self.root_ids
            .iter()
            .filter_map(move |id| self.elements.get(id))
            .filter(|e| e.owner.is_none())
    }

    /// Get the number of elements.
    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    /// Get the number of relationships.
    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty() && self.relationships.is_empty()
    }

    /// Rebuild indexes after deserialization.
    pub fn rebuild_indexes(&mut self) {
        self.owner_to_children.clear();
        self.source_to_rels.clear();
        self.target_to_rels.clear();
        self.relationship_kind_index.clear();
        self.namespace_to_memberships.clear();
        self.element_to_owning_membership.clear();
        self.typed_feature_to_typings.clear();
        self.specific_to_specializations.clear();
        self.kind_index.clear();
        self.name_index.clear();
        self.root_ids.clear();
        self.library_name_index.clear();
        self.library_index_dirty = !self.library_packages.is_empty();

        for (id, element) in &self.elements {
            // Rebuild kind index
            self.kind_index
                .entry(element.kind.clone())
                .or_default()
                .push(id.clone());

            // Rebuild name index
            if let Some(name) = &element.name {
                self.name_index
                    .entry(name.clone())
                    .or_default()
                    .push(id.clone());
            }
            if let Some(owner) = &element.owner {
                self.owner_to_children
                    .entry(owner.clone())
                    .or_default()
                    .insert(id.clone());
            } else {
                self.root_ids.push(id.clone());
            }

            // Rebuild owning_membership index
            if let Some(membership_id) = &element.owning_membership {
                self.element_to_owning_membership
                    .insert(id.clone(), membership_id.clone());
            }

            // Rebuild typed_feature_to_typings index from FeatureTyping elements
            if element.kind == ElementKind::FeatureTyping
                || element.kind.is_subtype_of(ElementKind::FeatureTyping)
            {
                if let Some(typed_feature) = element.props.get("typedFeature") {
                    if let Some(tf_id) = typed_feature.as_ref() {
                        self.typed_feature_to_typings
                            .entry(tf_id.clone())
                            .or_default()
                            .push(id.clone());
                    }
                }
            }

            // Rebuild specific_to_specializations index from Specialization elements
            if element.kind == ElementKind::Specialization
                || element.kind.is_subtype_of(ElementKind::Specialization)
            {
                if let Some(specific) = element.props.get("specific") {
                    if let Some(specific_id) = specific.as_ref() {
                        self.specific_to_specializations
                            .entry(specific_id.clone())
                            .or_default()
                            .push(id.clone());
                    }
                }
            }
        }

        // Rebuild namespace_to_memberships index from Membership elements
        for (id, element) in &self.elements {
            // Check if this is a Membership element
            if element.kind == ElementKind::Membership
                || element.kind.is_subtype_of(ElementKind::Membership)
            {
                // Get the membershipOwningNamespace from props
                if let Some(ns_ref) = element.props.get("membershipOwningNamespace") {
                    if let Some(ns_id) = ns_ref.as_ref() {
                        self.namespace_to_memberships
                            .entry(ns_id.clone())
                            .or_default()
                            .insert(id.clone());
                    }
                }
            }
        }

        for (id, rel) in &self.relationships {
            self.source_to_rels
                .entry(rel.source.clone())
                .or_default()
                .insert(id.clone());
            self.target_to_rels
                .entry(rel.target.clone())
                .or_default()
                .insert(id.clone());
            self.relationship_kind_index
                .entry(rel.kind.clone())
                .or_default()
                .push(id.clone());
        }

        self.indexes_dirty = false;
        self.invalidate_fingerprint();
    }

    /// Clear the graph.
    pub fn clear(&mut self) {
        self.elements.clear();
        self.relationships.clear();
        self.owner_to_children.clear();
        self.source_to_rels.clear();
        self.target_to_rels.clear();
        self.relationship_kind_index.clear();
        self.namespace_to_memberships.clear();
        self.element_to_owning_membership.clear();
        self.typed_feature_to_typings.clear();
        self.specific_to_specializations.clear();
        self.library_packages.clear();
        self.library_name_index.clear();
        self.library_index_dirty = true;
        self.name_index.clear();
        self.root_ids.clear();
        self.indexes_dirty = false;
        self.invalidate_resolution_cache();
        self.invalidate_fingerprint();
        self.is_elaborated = false;
    }

    // === Standard Library Support (Phase 2d.5) ===

    /// Mark a root package as a standard library package.
    ///
    /// Library packages are available globally during name resolution,
    /// making their public members accessible from any namespace.
    ///
    /// # Arguments
    ///
    /// * `package_id` - The ID of a root Package element
    ///
    /// # Returns
    ///
    /// `true` if the package was successfully marked as a library package,
    /// `false` if the element doesn't exist or is not a root package.
    pub fn register_library_package(&mut self, package_id: ElementId) -> bool {
        // Verify the element exists and is a root package
        if let Some(element) = self.elements.get(&package_id) {
            let is_package = element.kind == ElementKind::Package
                || element.kind == ElementKind::LibraryPackage
                || element.kind.is_subtype_of(ElementKind::Package);
            let is_root = element.owner.is_none();

            if is_package && is_root {
                self.library_packages.insert(package_id);
                // Mark the library index as needing rebuild
                self.library_index_dirty = true;
                return true;
            }
        }
        false
    }

    /// Remove a package from the library packages set.
    pub fn unregister_library_package(&mut self, package_id: &ElementId) -> bool {
        self.library_packages.remove(package_id)
    }

    /// Check if a package is registered as a library package.
    pub fn is_library_package(&self, package_id: &ElementId) -> bool {
        self.library_packages.contains(package_id)
    }

    /// Check if an element belongs to a library package (walks owner chain).
    ///
    /// Owner resolution mirrors [`owner_of`](Self::owner_of): try the cached
    /// `owner` field first, else dereference the element's `owning_membership`
    /// → `membershipOwningNamespace`. The cached `owner` can be stale on merged
    /// or library graphs — `rebuild_indexes` does not recompute it from
    /// membership — which would otherwise strand a nested library element
    /// (e.g. `TradeStudies::MaximizeObjective`) as an apparent owner-less root
    /// and make this predicate wrongly return `false` for it. The fallback is
    /// inlined rather than calling `owner_of` to avoid a second element lookup
    /// per hop.
    pub fn is_library_element(&self, element_id: &ElementId) -> bool {
        let mut current = Some(element_id.clone());
        while let Some(id) = current {
            if self.library_packages.contains(&id) {
                return true;
            }
            current = self.elements.get(&id).and_then(|e| {
                e.owner.clone().or_else(|| {
                    let membership = self.elements.get(e.owning_membership.as_ref()?)?;
                    membership
                        .props
                        .get(crate::membership::props::MEMBERSHIP_OWNING_NAMESPACE)?
                        .as_ref()
                        .cloned()
                })
            });
        }
        false
    }

    /// Get all library package IDs.
    pub fn library_packages(&self) -> &FxHashSet<ElementId> {
        &self.library_packages
    }

    /// Get all library packages as elements.
    pub fn library_package_elements(&self) -> impl Iterator<Item = &Element> {
        self.library_packages
            .iter()
            .filter_map(move |id| self.elements.get(id))
    }

    /// Build the library name index for O(1) lookup of library members.
    ///
    /// This indexes all public members of all registered library packages,
    /// including nested namespaces (recursively). The index maps names
    /// to element IDs for fast resolution.
    pub fn build_library_index(&mut self) {
        self.library_name_index.clear();
        let mut visited = FxHashSet::default();

        // Clone the library_packages to avoid borrow issues
        let lib_pkg_ids: Vec<ElementId> = self.library_packages.iter().cloned().collect();

        for lib_pkg_id in lib_pkg_ids {
            // Index the library package itself by name
            if let Some(lib_pkg) = self.elements.get(&lib_pkg_id) {
                if let Some(name) = &lib_pkg.name {
                    self.library_name_index
                        .insert(name.clone(), lib_pkg_id.clone());
                }
            }
            // Recursively index all members
            self.index_library_recursively(&lib_pkg_id, &mut visited);
        }

        // Mark the index as up-to-date
        self.library_index_dirty = false;
    }

    /// Recursively index library namespace members.
    ///
    /// Adds all public members to the library_name_index, including nested namespaces.
    fn index_library_recursively(
        &mut self,
        namespace_id: &ElementId,
        visited: &mut FxHashSet<ElementId>,
    ) {
        if !visited.insert(namespace_id.clone()) {
            return; // Already visited, prevent cycles
        }

        // Collect membership info to avoid borrow issues
        let members_to_index: Vec<(String, ElementId, bool)> = {
            let membership_ids = self.namespace_to_memberships.get(namespace_id);
            membership_ids
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|membership_id| self.elements.get(membership_id))
                .filter_map(|membership| {
                    // Check visibility - only index public members
                    let visibility = membership
                        .props
                        .get("visibility")
                        .and_then(|v| v.as_str())
                        .unwrap_or("public");
                    if visibility != "public" {
                        return None;
                    }

                    // Get member element ID
                    let member_id = membership
                        .props
                        .get("memberElement")
                        .and_then(|v| v.as_ref())?;

                    // Get member name from membership or element
                    let member_name = membership
                        .props
                        .get("memberName")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned())
                        .or_else(|| self.elements.get(member_id).and_then(|e| e.name.clone()))?;

                    // Check if member is a namespace (needs recursive indexing)
                    let is_namespace = self.elements.get(member_id).is_some_and(|e| {
                        e.kind == ElementKind::Package
                            || e.kind == ElementKind::Namespace
                            || e.kind.is_subtype_of(ElementKind::Namespace)
                    });

                    Some((member_name, member_id.clone(), is_namespace))
                })
                .collect()
        };

        // Add members to index and recurse into namespaces
        for (name, member_id, is_namespace) in members_to_index {
            // Index both quoted and unquoted variants so lookups never
            // fall through to the expensive recursive scan.
            let stripped = name.trim_matches('\'');
            if stripped != name {
                // Name is quoted like 'Foo' — also index as Foo
                self.library_name_index
                    .entry(stripped.to_owned())
                    .or_insert(member_id.clone());
            } else {
                // Name is unquoted like Foo — also index as 'Foo'
                self.library_name_index
                    .entry(format!("'{}'", name))
                    .or_insert(member_id.clone());
            }

            // Insert into index (first occurrence wins)
            self.library_name_index
                .entry(name)
                .or_insert(member_id.clone());

            // Recurse into nested namespaces
            if is_namespace {
                self.index_library_recursively(&member_id, visited);
            }
        }
    }

    /// Ensure the library name index is built and up-to-date.
    ///
    /// Call this before starting resolution to enable O(1) library lookups.
    /// This is called automatically by `resolve_references()`.
    pub fn ensure_library_index(&mut self) {
        if self.library_index_dirty && !self.library_packages.is_empty() {
            self.build_library_index();
        }
    }

    /// Resolve a name in the library index with O(1) lookup.
    ///
    /// Note: Call `ensure_library_index()` before resolution to build the index.
    /// Returns the ElementId if found, None otherwise.
    pub fn resolve_in_library(&self, name: &str) -> Option<&ElementId> {
        // Try exact match first
        if let Some(id) = self.library_name_index.get(name) {
            return Some(id);
        }
        // Try with quotes stripped
        let stripped = name.trim_matches('\'');
        if stripped != name {
            if let Some(id) = self.library_name_index.get(stripped) {
                return Some(id);
            }
        }
        // Try with quotes added
        let quoted = format!("'{}'", stripped);
        if quoted != name {
            self.library_name_index.get(&quoted)
        } else {
            None
        }
    }

    /// Check if the library name index needs rebuilding.
    ///
    /// Used by resolution to determine if index should be built.
    pub fn library_index_needs_rebuild(&self) -> bool {
        self.library_index_dirty && !self.library_packages.is_empty()
    }

    /// Add an element as a library package.
    ///
    /// This is a convenience method that combines `add_element` and
    /// `register_library_package`.
    ///
    /// # Returns
    ///
    /// The ElementId of the added package.
    pub fn add_library_package(&mut self, element: Element) -> ElementId {
        let id = self.add_element(element);
        self.library_packages.insert(id.clone());
        id
    }

    /// Merge another graph's elements into this graph.
    ///
    /// This is useful for loading standard library graphs into a user graph.
    /// If `as_library` is true, all root packages from the source graph
    /// are registered as library packages.
    ///
    /// # Arguments
    ///
    /// * `other` - The graph to merge from
    /// * `as_library` - Whether to mark merged root packages as library packages
    ///
    /// # Returns
    ///
    /// The number of elements merged.
    pub fn merge(&mut self, other: ModelGraph, as_library: bool) -> usize {
        let count = other.elements.len();

        // Collect root package IDs before merging. Iterate the pre-built
        // `root_ids` index (owner-less elements) rather than scanning every
        // element — O(roots) instead of O(|elements|).
        let root_package_ids: Vec<ElementId> = if as_library {
            other
                .root_ids
                .iter()
                .filter_map(|id| other.elements.get(id))
                .filter(|e| {
                    e.owner.is_none()
                        && (e.kind == ElementKind::Package
                            || e.kind == ElementKind::LibraryPackage
                            || e.kind.is_subtype_of(ElementKind::Package))
                })
                .map(|e| e.id.clone())
                .collect()
        } else {
            Vec::new()
        };

        // Merge elements
        for (id, element) in other.elements {
            self.elements.insert(id.clone(), element);
            // Note: We don't update owner_to_children here as they're for the original graph
        }

        // Merge relationships
        for (id, rel) in other.relationships {
            self.relationships.insert(id, rel);
        }

        // Register library packages
        for id in root_package_ids {
            self.library_packages.insert(id);
        }

        // Merge indexes from the other graph to preserve pre-built index data.
        // This is critical for library merging: the library's namespace_to_memberships
        // index enables build_library_index() to work correctly.

        // Merge namespace_to_memberships index
        for (ns_id, membership_ids) in other.namespace_to_memberships {
            self.namespace_to_memberships
                .entry(ns_id)
                .or_default()
                .extend(membership_ids);
        }

        // Merge owner_to_children index
        for (owner_id, child_ids) in other.owner_to_children {
            self.owner_to_children
                .entry(owner_id)
                .or_default()
                .extend(child_ids);
        }

        // Merge element_to_owning_membership index
        for (elem_id, membership_id) in other.element_to_owning_membership {
            self.element_to_owning_membership
                .entry(elem_id)
                .or_insert(membership_id);
        }

        // Merge typed_feature_to_typings index
        for (feature_id, typing_ids) in other.typed_feature_to_typings {
            self.typed_feature_to_typings
                .entry(feature_id)
                .or_default()
                .extend(typing_ids);
        }

        // Merge specific_to_specializations index
        for (specific_id, spec_ids) in other.specific_to_specializations {
            self.specific_to_specializations
                .entry(specific_id)
                .or_default()
                .extend(spec_ids);
        }

        // Merge source_to_rels and target_to_rels indexes
        for (source_id, rel_ids) in other.source_to_rels {
            self.source_to_rels
                .entry(source_id)
                .or_default()
                .extend(rel_ids);
        }
        for (target_id, rel_ids) in other.target_to_rels {
            self.target_to_rels
                .entry(target_id)
                .or_default()
                .extend(rel_ids);
        }

        // Merge relationship_kind_index
        for (kind, rel_ids) in other.relationship_kind_index {
            self.relationship_kind_index
                .entry(kind)
                .or_default()
                .extend(rel_ids);
        }

        // Merge kind_index
        for (kind, elem_ids) in other.kind_index {
            self.kind_index.entry(kind).or_default().extend(elem_ids);
        }

        // Merge name_index
        for (name, elem_ids) in other.name_index {
            self.name_index.entry(name).or_default().extend(elem_ids);
        }

        // Merge library_name_index if the other graph had one built
        if !other.library_name_index.is_empty() {
            for (name, elem_id) in other.library_name_index {
                self.library_name_index.entry(name).or_insert(elem_id);
            }
        }

        // Merge root_ids from the other graph
        self.root_ids.extend(other.root_ids);

        // Note: We don't mark indexes_dirty since we've properly merged them.
        // The indexes are now consistent with the merged elements/relationships.

        // Mark library index as needing rebuild if we added library packages
        // (to index newly registered library packages)
        if as_library {
            self.library_index_dirty = true;
        }

        self.invalidate_fingerprint();
        self.is_elaborated = false;
        count
    }

    /// Merge another graph by reference (avoids cloning the source graph).
    ///
    /// This is functionally identical to `merge()` but borrows `other` instead
    /// of taking ownership, cloning individual entries during insertion.
    /// Useful when merging from an `Arc<ModelGraph>` without cloning the whole graph.
    pub fn merge_from_ref(&mut self, other: &ModelGraph, as_library: bool) -> usize {
        let count = other.elements.len();

        // Collect root package IDs before merging. Iterate the pre-built
        // `root_ids` index (owner-less elements) rather than scanning every
        // element — O(roots) instead of O(|elements|).
        let root_package_ids: Vec<ElementId> = if as_library {
            other
                .root_ids
                .iter()
                .filter_map(|id| other.elements.get(id))
                .filter(|e| {
                    e.owner.is_none()
                        && (e.kind == ElementKind::Package
                            || e.kind == ElementKind::LibraryPackage
                            || e.kind.is_subtype_of(ElementKind::Package))
                })
                .map(|e| e.id.clone())
                .collect()
        } else {
            Vec::new()
        };

        // Merge elements
        for (id, element) in &other.elements {
            self.elements.insert(id.clone(), element.clone());
        }

        // Merge relationships
        for (id, rel) in &other.relationships {
            self.relationships.insert(id.clone(), rel.clone());
        }

        // Register library packages
        for id in root_package_ids {
            self.library_packages.insert(id);
        }

        // Merge indexes
        for (ns_id, membership_ids) in &other.namespace_to_memberships {
            self.namespace_to_memberships
                .entry(ns_id.clone())
                .or_default()
                .extend(membership_ids.iter().cloned());
        }

        for (owner_id, child_ids) in &other.owner_to_children {
            self.owner_to_children
                .entry(owner_id.clone())
                .or_default()
                .extend(child_ids.iter().cloned());
        }

        for (elem_id, membership_id) in &other.element_to_owning_membership {
            self.element_to_owning_membership
                .entry(elem_id.clone())
                .or_insert_with(|| membership_id.clone());
        }

        for (feature_id, typing_ids) in &other.typed_feature_to_typings {
            self.typed_feature_to_typings
                .entry(feature_id.clone())
                .or_default()
                .extend(typing_ids.iter().cloned());
        }

        for (specific_id, spec_ids) in &other.specific_to_specializations {
            self.specific_to_specializations
                .entry(specific_id.clone())
                .or_default()
                .extend(spec_ids.iter().cloned());
        }

        for (source_id, rel_ids) in &other.source_to_rels {
            self.source_to_rels
                .entry(source_id.clone())
                .or_default()
                .extend(rel_ids.iter().cloned());
        }

        for (target_id, rel_ids) in &other.target_to_rels {
            self.target_to_rels
                .entry(target_id.clone())
                .or_default()
                .extend(rel_ids.iter().cloned());
        }

        // Merge relationship_kind_index
        for (kind, rel_ids) in &other.relationship_kind_index {
            self.relationship_kind_index
                .entry(kind.clone())
                .or_default()
                .extend(rel_ids.iter().cloned());
        }

        // Merge kind_index
        for (kind, elem_ids) in &other.kind_index {
            self.kind_index
                .entry(kind.clone())
                .or_default()
                .extend(elem_ids.iter().cloned());
        }

        // Merge name_index
        for (name, elem_ids) in &other.name_index {
            self.name_index
                .entry(name.clone())
                .or_default()
                .extend(elem_ids.iter().cloned());
        }

        if !other.library_name_index.is_empty() {
            for (name, elem_id) in &other.library_name_index {
                self.library_name_index
                    .entry(name.clone())
                    .or_insert_with(|| elem_id.clone());
            }
        }

        // Merge root_ids from the other graph
        self.root_ids.extend(other.root_ids.iter().cloned());

        if as_library {
            self.library_index_dirty = true;
        }

        self.invalidate_fingerprint();
        self.is_elaborated = false;
        count
    }

    /// Compute a content-true fingerprint for change detection.
    ///
    /// Hashes the FULL content of every element and relationship
    /// ([`Element::content_hash`] / [`Relationship::content_hash`],
    /// sorted by id for determinism): ids, kinds, names, ownership,
    /// property values (doc text, requirement statements, attribute
    /// defaults), spans, and relationship endpoints.
    ///
    /// This is the salsa change-detection seam: every fingerprint-mode
    /// Arc wrapper in `sysml-ide-db` (ParseResult, MergedGraph,
    /// ElaboratedWorkspace, …) compares through it, and an "equal"
    /// result BACKDATES the recompute — downstream consumers keep the
    /// old graph. The pre-2026-07-16 version hashed only sorted
    /// (name, kind) pairs, so doc-/value-/span-only edits compared
    /// equal and were served stale across every transport until a
    /// structural edit happened to land (live-observed as requirement
    /// rows keeping pre-edit text across workspace reloads). A false
    /// "changed" merely costs a recompute; a false "unchanged" serves
    /// stale data — when in doubt, hash it.
    ///
    /// Parse-tier element ids are reparse-stable (`CanonicalKey`,
    /// ADR-009), so identical source still fingerprints identically
    /// across parses. Synthetic elements minted with fresh UUIDs
    /// (`Element::new_with_kind`) make repeat elaborations compare
    /// unequal — honest, and shrinking as mint sites move to canonical
    /// keys (see the MembershipBuilder/ElementFactory id-stability
    /// audit follow-up).
    pub fn fingerprint(&self) -> u64 {
        // Fast path: return the cached value if present. The graph is frozen
        // once construction/elaboration completes, so salsa probes hit this.
        if let Ok(slot) = self.fingerprint_cache.read() {
            if let Some(fp) = *slot {
                return fp;
            }
        }
        let fp = self.compute_fingerprint();
        if let Ok(mut slot) = self.fingerprint_cache.write() {
            *slot = Some(fp);
        }
        fp
    }

    /// Compute the content-true fingerprint from scratch (O(n log n)).
    ///
    /// Thin orchestration only — the per-item field walks live on
    /// [`Element::content_hash`] and [`Relationship::content_hash`]
    /// (each type owns "what counts as my content").
    fn compute_fingerprint(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.elements.len().hash(&mut hasher);
        self.relationships.len().hash(&mut hasher);
        let mut elem_ids: Vec<_> = self.elements.keys().collect();
        elem_ids.sort();
        for id in elem_ids {
            self.elements[id].content_hash(&mut hasher);
        }
        let mut rel_ids: Vec<_> = self.relationships.keys().collect();
        rel_ids.sort();
        for id in rel_ids {
            self.relationships[id].content_hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use crate::meta::Value;
    use crate::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};
    use sysml_id::ElementId;
    use sysml_span::Span;

    fn base_graph() -> (ModelGraph, ElementId) {
        let mut g = ModelGraph::new();
        let mut e = Element::new(
            ElementId::from_string("elem-1"),
            ElementKind::RequirementDefinition,
        );
        e.name = Some("R1".to_string());
        e.props
            .insert("body".into(), Value::String("the old statement".into()));
        e.spans = vec![Span::new("a.sysml".to_string(), 0, 10)];
        let id = g.add_element(e);
        (g, id)
    }

    /// Identical content ⇒ identical fingerprint (the dedup property the
    /// salsa fingerprint wrappers rely on).
    #[test]
    fn identical_graphs_fingerprint_equal() {
        let (a, _) = base_graph();
        let (b, _) = base_graph();
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    /// A PROPERTY-VALUE-only change (doc text, requirement statement,
    /// attribute default) must change the fingerprint. The 2026-07-16
    /// staleness bug: the old (name, kind)-only hash compared equal here,
    /// so salsa backdated re-parses and every downstream consumer served
    /// pre-edit doc text until a structural edit happened to land.
    #[test]
    fn prop_value_change_changes_fingerprint() {
        let (a, _) = base_graph();
        let (mut b, id) = base_graph();
        b.elements
            .get_mut(&id)
            .unwrap()
            .props
            .insert("body".into(), Value::String("the NEW statement".into()));
        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "doc/value-only edits must be visible to the fingerprint"
        );
    }

    /// A SPAN-only shift must change the fingerprint — position consumers
    /// (goto-def, hover, requirement-row source_span) read spans off the
    /// same backdated graph objects.
    #[test]
    fn span_shift_changes_fingerprint() {
        let (a, _) = base_graph();
        let (mut b, id) = base_graph();
        b.elements.get_mut(&id).unwrap().spans = vec![Span::new("a.sysml".to_string(), 5, 15)];
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    /// A relationship REWIRE between existing elements must change the
    /// fingerprint (endpoints are part of relationship content). Both
    /// graphs hold the SAME element set (two structurally symmetric
    /// siblings) and one relationship differing only in its target — the
    /// old id-blind hash compared these equal.
    #[test]
    fn relationship_rewire_changes_fingerprint() {
        fn with_rel_target(target: &str) -> ModelGraph {
            let (mut g, req_id) = base_graph();
            for sibling in ["elem-2", "elem-3"] {
                let mut e =
                    Element::new(ElementId::from_string(sibling), ElementKind::PartDefinition);
                e.name = Some("P".to_string());
                g.add_element(e);
            }
            let mut rel = Relationship::new(
                RelationshipKind::Dependency,
                req_id,
                ElementId::from_string(target),
            );
            rel.id = ElementId::from_string("rel-1");
            g.add_relationship(rel);
            g
        }
        let a = with_rel_target("elem-2");
        let b = with_rel_target("elem-3");
        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "relationship rewire must be visible to the fingerprint"
        );
    }
}
