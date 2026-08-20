//! Resolution context and name resolution logic.

use std::cell::RefCell;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use sysml_id::ElementId;
use sysml_span::{Diagnostic, Diagnostics};

use crate::membership::MembershipView;
use crate::{ElementKind, ModelGraph, VisibilityKind};

use super::res_trace;
use super::scope_table::ScopeTable;
use super::scoping;
use super::{import_props, primitive_type_alias, resolved_props, unresolved_props};

/// Maximum depth for inheritance traversal.
/// This prevents infinite recursion in case of cycles not caught by the visited set.
const MAX_INHERITANCE_DEPTH: usize = 50;

/// Pre-computed inheritance index: maps types to their direct supertypes.
///
/// This is built lazily and provides O(1) lookup of supertypes,
/// avoiding repeated iteration over owned members during inheritance expansion.
///
/// Pre-computed inheritance index: maps types to their direct supertypes.
///
/// Built once before resolution and provides O(1) lookup of supertypes,
/// avoiding repeated iteration over owned members during inheritance expansion.
#[derive(Debug, Clone)]
pub struct InheritanceIndex {
    /// Maps type ElementId -> list of direct supertype ElementIds
    direct_supertypes: FxHashMap<ElementId, Vec<ElementId>>,
}

impl InheritanceIndex {
    /// Build the inheritance index from a ModelGraph.
    ///
    /// Iterates over all elements once to find Specialization relationships
    /// and pre-computes the direct supertype mapping.
    pub fn build(graph: &ModelGraph) -> Self {
        Self::build_combined(graph, None)
    }

    /// Build the inheritance index from a primary graph and optional fallback graph.
    ///
    /// Collects specialization relationships from both graphs for a complete
    /// inheritance picture. This is needed when library types have inheritance
    /// chains that must be visible during file resolution.
    pub fn build_combined(graph: &ModelGraph, fallback: Option<&ModelGraph>) -> Self {
        let mut map: FxHashMap<ElementId, Vec<ElementId>> = FxHashMap::default();

        Self::collect_specializations(graph, &mut map);
        if let Some(fg) = fallback {
            Self::collect_specializations(fg, &mut map);
        }

        Self {
            direct_supertypes: map,
        }
    }

    /// Build a combined index by overlaying user-graph specializations onto a
    /// pre-built library index.
    ///
    /// This is the build-once seam for the library half: callers that already
    /// hold a long-lived [`InheritanceIndex`] for the standard library can
    /// avoid the O(|library|) scan by cloning the prebuilt map and only
    /// scanning the (small) user graph. Mirrors the `library_name_index`
    /// build-once pattern that lives on `LibraryData`.
    pub fn build_user_overlay(prebuilt_library: &Self, user_graph: &ModelGraph) -> Self {
        let mut map = prebuilt_library.direct_supertypes.clone();
        Self::collect_specializations(user_graph, &mut map);
        Self {
            direct_supertypes: map,
        }
    }

    /// Collect specialization relationships from a graph into the index map.
    ///
    /// Every specialization-family relationship is read using the property pair
    /// appropriate for its concrete kind (Subclassification -> superclassifier,
    /// FeatureTyping -> type, Subsetting -> subsettedFeature, Redefinition ->
    /// redefinedFeature, plain Specialization -> general). Reading only `general`
    /// (the historical behaviour) silently dropped every supertype reached via a
    /// non-plain-Specialization relationship — e.g. a definition's `:> Super`
    /// lowers to a `Subclassification` whose target lives in `superclassifier`,
    /// so its members never entered the inherited scope tier and bare-name
    /// references to them only resolved via the library sweep.
    ///
    /// Only the *resolved* target id (filled in by pass 1) is indexed; unresolved
    /// names are intentionally NOT resolved here so the index never invents
    /// inheritance edges that pass 1 did not establish.
    fn collect_specializations(graph: &ModelGraph, map: &mut FxHashMap<ElementId, Vec<ElementId>>) {
        for elem in graph.elements.values() {
            if elem.kind == ElementKind::Specialization
                || elem.kind.is_subtype_of(ElementKind::Specialization)
            {
                let Some(specific_id) = &elem.owner else {
                    continue;
                };
                let Some((resolved_key, _)) =
                    ResolutionContext::specialization_target_props(&elem.kind)
                else {
                    continue;
                };
                if let Some(general_id) = elem.props.get(resolved_key).and_then(|v| v.as_ref()) {
                    map.entry(specific_id.clone())
                        .or_default()
                        .push(general_id.clone());
                }
            }
        }
    }

    /// Get the direct supertypes for a type.
    fn supertypes(&self, type_id: &ElementId) -> &[ElementId] {
        self.direct_supertypes
            .get(type_id)
            .map(|v| &v[..])
            .unwrap_or(&[])
    }
}

/// Storage for the inheritance index inside a [`ResolutionContext`].
///
/// `Owned` is used for lazy-built per-context indexes (the legacy path) and
/// for dual-graph contexts that overlay a user graph on a prebuilt library
/// index. `Shared` is used when the library prebuilt index can be reused
/// verbatim — context construction is then an `Arc::clone` (refcount bump)
/// instead of a full HashMap deep copy.
#[derive(Debug, Clone)]
enum InheritanceIndexHandle {
    Owned(InheritanceIndex),
    Shared(Arc<InheritanceIndex>),
}

impl InheritanceIndexHandle {
    fn supertypes(&self, type_id: &ElementId) -> &[ElementId] {
        match self {
            Self::Owned(idx) => idx.supertypes(type_id),
            Self::Shared(idx) => idx.supertypes(type_id),
        }
    }
}

/// Context for name resolution.
///
/// Tracks state during resolution to prevent cycles and provide context
/// for visibility checks.
///
/// Supports an optional `fallback_graph` for dual-graph resolution. When set,
/// element lookups, membership iteration, and library resolution check the
/// fallback graph when the primary graph doesn't have the requested data.
/// This enables resolving file references against library types without
/// merging library elements into the file graph.
#[derive(Debug)]
pub struct ResolutionContext<'a> {
    /// The model graph being resolved.
    pub(crate) graph: &'a ModelGraph,
    /// Optional fallback graph for dual-graph resolution (e.g., library graph).
    /// Checked after the primary graph for element lookups and namespace iteration.
    fallback_graph: Option<&'a ModelGraph>,
    /// Cached scope tables per namespace (locally built / mutated).
    pub(crate) scope_tables: FxHashMap<ElementId, ScopeTable>,
    /// Optional pre-built scope tables shared across parallel chunks.
    ///
    /// Used by the parallel resolution path to avoid cloning the entire
    /// pre-built table map into every per-chunk `ResolutionContext`. Lookup
    /// order: local `scope_tables` first, then `prebuilt_scope_tables`.
    /// Any new entries built lazily during this chunk go into the local map
    /// only — the shared map is treated as immutable.
    pub(crate) prebuilt_scope_tables: Option<Arc<FxHashMap<ElementId, ScopeTable>>>,
    /// Elements currently being visited (cycle detection).
    visiting: FxHashSet<ElementId>,
    /// Whether we're inside a scope (affects private member visibility).
    inside_scope: Option<ElementId>,
    /// Whether we're inheriting (affects protected member visibility).
    inheriting: bool,
    /// Collected diagnostics.
    diagnostics: Diagnostics,
    /// Recorded ambiguous-import resolutions (ADR-016 D5).
    ///
    /// Keyed by `(importing_namespace_id, name)` -> sorted distinct candidate
    /// ids. Populated only on the user (fallback) resolution path; the library
    /// self-resolution path never records here (its ~83 benign cross-file
    /// re-export collisions must not be flagged). Drained via
    /// [`take_ambiguities`](Self::take_ambiguities).
    ambiguity_sink: RefCell<FxHashMap<(ElementId, String), Vec<ElementId>>>,
    /// Cache for resolved import targets (qualified name -> resolved ElementId).
    /// Uses RefCell for interior mutability since resolve_import_target is called from &self contexts.
    import_cache: RefCell<FxHashMap<String, Option<ElementId>>>,
    /// Negative lookup cache: (namespace_id, name) pairs that have already failed resolution.
    /// Avoids redundant parent-walking for names that don't exist anywhere.
    failed_lookups: RefCell<FxHashSet<(ElementId, String)>>,
    /// Library-level negative cache: names that don't exist in the library.
    /// Unlike failed_lookups, this is namespace-independent — if a name isn't
    /// in the library, it won't be found regardless of the querying namespace.
    failed_library_lookups: RefCell<FxHashSet<String>>,
    /// Pre-computed inheritance index for O(1) supertype lookup.
    /// Lazily built on first use, or supplied by the library load via
    /// the `*_with_lib_inheritance_index` ctors.
    inheritance_index: Option<InheritanceIndexHandle>,
    /// Generation counter for scope table cache invalidation.
    /// Bump this when the graph is mutated between resolution passes
    /// to force stale scope tables to be rebuilt.
    generation: u64,
    /// Opt-in import-gating flag (ADR-016 / import-resolution plan §6a P2 step 1).
    ///
    /// **Default `false` = zero behavior change** (byte-for-byte identical to the
    /// pre-gate resolver). When `true`, the spec-violating *bare-name library
    /// member sweep* is disabled: a bare, unqualified cross-package name only
    /// resolves through owned/inherited/imported/parent/global tiers, NOT by
    /// recursively scanning library package members. A bare single-segment
    /// *top-level library package name* (e.g. `ScalarValues`) still resolves —
    /// that is spec-legal global scope. Qualified names (`ScalarValues::Real`),
    /// implicit generalization (`gen()`), and supertype-reference resolution
    /// remain unaffected by the gate.
    ///
    /// This exists so gated resolution can be run for *measurement/migration*
    /// (see the migration measurement test) without flipping default behavior.
    /// Do NOT default this to `true` until the migration pass (P2 step 2/3) lands.
    gate_bare_library: bool,
}

impl<'a> ResolutionContext<'a> {
    /// Create a new resolution context.
    pub fn new(graph: &'a ModelGraph) -> Self {
        ResolutionContext {
            graph,
            fallback_graph: None,
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: None,
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: None,
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Create a resolution context with a fallback graph for dual-graph resolution.
    ///
    /// The primary graph is checked first for all lookups, then the fallback graph.
    /// This enables resolving file references against library types without merging
    /// library elements into the file graph.
    pub fn new_with_fallback(graph: &'a ModelGraph, fallback: &'a ModelGraph) -> Self {
        ResolutionContext {
            graph,
            fallback_graph: Some(fallback),
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: None,
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: None,
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Create a library-only resolution context that reuses a pre-built
    /// inheritance index for the library graph.
    ///
    /// The 39.4 %-exclusive [`InheritanceIndex::collect_specializations`] frame
    /// in the May 29 perf baseline traces back to `ResolutionContext::new(lib)`
    /// rebuilding the stdlib inheritance closure on every IG-1 candidate. The
    /// prebuilt index is passed in by [`Arc`] so context construction is an
    /// O(1) refcount bump — no map clone, no rescan.
    pub fn new_with_lib_inheritance_index(
        library: &'a ModelGraph,
        lib_inheritance_index: Arc<InheritanceIndex>,
    ) -> Self {
        ResolutionContext {
            graph: library,
            fallback_graph: None,
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: None,
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: Some(InheritanceIndexHandle::Shared(lib_inheritance_index)),
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Create a dual-graph resolution context (user primary, library fallback)
    /// that reuses a pre-built library inheritance index.
    ///
    /// The lazy `ensure_inheritance_index` would otherwise call
    /// `build_combined(user, library)`, which re-scans every library element.
    /// Here we seed the index with the prebuilt library map and only scan the
    /// (small) user graph for its overlay specializations. The combined index
    /// is per-context (the user half must not be cached across files), so we
    /// pay one clone of the lib map per IG-1 file — not per candidate.
    pub fn new_with_fallback_and_lib_inheritance_index(
        graph: &'a ModelGraph,
        fallback: &'a ModelGraph,
        lib_inheritance_index: &InheritanceIndex,
    ) -> Self {
        let combined = InheritanceIndex::build_user_overlay(lib_inheritance_index, graph);
        ResolutionContext {
            graph,
            fallback_graph: Some(fallback),
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: None,
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: Some(InheritanceIndexHandle::Owned(combined)),
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Ensure the inheritance index is built.
    ///
    /// This is called lazily on first use to avoid building the index
    /// if inheritance expansion is never needed. When a fallback graph
    /// is present, the index is built from both graphs.
    fn ensure_inheritance_index(&mut self) {
        if self.inheritance_index.is_none() {
            self.inheritance_index = Some(InheritanceIndexHandle::Owned(
                InheritanceIndex::build_combined(self.graph, self.fallback_graph),
            ));
        }
    }

    /// Create a new resolution context, reusing a pre-built inheritance index.
    ///
    /// This avoids rebuilding the index from scratch when it was already
    /// constructed in a previous resolution pass.
    pub(crate) fn new_with_index(graph: &'a ModelGraph, index: InheritanceIndex) -> Self {
        ResolutionContext {
            graph,
            fallback_graph: None,
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: None,
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: Some(InheritanceIndexHandle::Owned(index)),
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Create a resolution context with pre-built scope tables and optional inheritance index.
    ///
    /// Used by the parallel resolution path: scope tables are pre-built once in a
    /// single-threaded pre-build phase, then **shared by `Arc`** across per-thread
    /// contexts. The shared map is treated as immutable; any new entries built
    /// lazily in this context go into a per-context overlay map. This avoids the
    /// O(N) HashMap deep-clone per parallel chunk that the previous implementation
    /// performed (per d284fe31 perf re-profile).
    pub(crate) fn new_with_prebuilt(
        graph: &'a ModelGraph,
        prebuilt_scope_tables: Arc<FxHashMap<ElementId, ScopeTable>>,
        inheritance_index: Option<InheritanceIndex>,
    ) -> Self {
        ResolutionContext {
            graph,
            fallback_graph: None,
            scope_tables: FxHashMap::default(),
            prebuilt_scope_tables: Some(prebuilt_scope_tables),
            visiting: FxHashSet::default(),
            inside_scope: None,
            inheriting: false,
            diagnostics: Diagnostics::new(),
            ambiguity_sink: RefCell::new(FxHashMap::default()),
            import_cache: RefCell::new(FxHashMap::default()),
            failed_lookups: RefCell::new(FxHashSet::default()),
            failed_library_lookups: RefCell::new(FxHashSet::default()),
            inheritance_index: inheritance_index.map(InheritanceIndexHandle::Owned),
            generation: 0,
            gate_bare_library: false,
        }
    }

    /// Take the inheritance index out of this context, if one has been built.
    ///
    /// This transfers ownership so the index can be reused in a new context
    /// (e.g., across resolution passes).
    pub(crate) fn take_inheritance_index(&mut self) -> Option<InheritanceIndex> {
        match self.inheritance_index.take()? {
            InheritanceIndexHandle::Owned(idx) => Some(idx),
            // Shared handles can't be transferred by value — the caller owns
            // the `Arc` already. Returning `None` is correct: the only caller
            // (parallel pre-build) only uses owned-built indexes.
            InheritanceIndexHandle::Shared(_) => None,
        }
    }

    /// Take all cached scope tables out of this context.
    ///
    /// Used by the parallel resolution path to extract pre-built scope tables
    /// for sharing across threads.
    pub(crate) fn take_scope_tables(&mut self) -> FxHashMap<ElementId, ScopeTable> {
        std::mem::take(&mut self.scope_tables)
    }

    /// Bump the generation counter, invalidating all cached scope tables.
    ///
    /// Call this after graph mutations (e.g., between resolution passes)
    /// to ensure stale scope tables are rebuilt on next access.
    pub fn bump_generation(&mut self) {
        self.generation += 1;
        // Clear caches that are stale after graph mutations
        self.failed_lookups.borrow_mut().clear();
        self.failed_library_lookups.borrow_mut().clear();
        self.import_cache.borrow_mut().clear();
    }

    /// Get the current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Enable or disable the opt-in import-gate (builder style).
    ///
    /// **Default is OFF.** When `on == true`, the bare-name library *member*
    /// sweep is disabled (see [`Self::gate_bare_library`]). Use this for gated
    /// resolution measurement/migration; do NOT enable by default until the
    /// migration pass lands (ADR-016 P2 sequencing).
    #[must_use]
    pub fn with_bare_library_gate(mut self, on: bool) -> Self {
        self.gate_bare_library = on;
        self
    }

    /// Set the import-gate flag in place (setter style).
    pub fn set_bare_library_gate(&mut self, on: bool) {
        self.gate_bare_library = on;
    }

    /// Whether the bare-name library member sweep is currently gated off.
    pub fn bare_library_gated(&self) -> bool {
        self.gate_bare_library
    }

    /// Resolve a bare name against *top-level library package names only*.
    ///
    /// This is the spec-legal slice of the library lookup that survives the
    /// gate: a single-segment name that names a registered top-level library
    /// package (e.g. `ScalarValues`, `SI`, `ISQ`, `Base`). It deliberately does
    /// NOT search package *members* — that recursive member scan is the
    /// spec-violating bare-name sweep the gate disables. Used both as the gated
    /// replacement for tier 6 in `resolve_name_inner` and to keep the first
    /// segment of a qualified name (`ScalarValues::Real`) resolvable when gated.
    fn resolve_library_package_name(&self, name: &str) -> Option<ElementId> {
        for lib_pkg_id in self.graph.library_packages() {
            if let Some(lib_pkg) = self.graph.get_element(lib_pkg_id) {
                if lib_pkg
                    .name
                    .as_ref()
                    .map(|n| Self::names_match(n, name))
                    .unwrap_or(false)
                {
                    return Some(lib_pkg_id.clone());
                }
            }
        }
        if let Some(fg) = self.fallback_graph {
            for lib_pkg_id in fg.library_packages() {
                if let Some(lib_pkg) = fg.get_element(lib_pkg_id) {
                    if lib_pkg
                        .name
                        .as_ref()
                        .map(|n| Self::names_match(n, name))
                        .unwrap_or(false)
                    {
                        return Some(lib_pkg_id.clone());
                    }
                }
            }
        }
        None
    }

    /// Get the underlying graph.
    pub fn graph(&self) -> &'a ModelGraph {
        self.graph
    }

    // ----- Dual-graph lookup helpers -----

    /// Look up an element by ID, checking the primary graph first, then the fallback.
    fn lookup_element(&self, id: &ElementId) -> Option<&'a crate::Element> {
        self.graph
            .get_element(id)
            .or_else(|| self.fallback_graph.and_then(|fg| fg.get_element(id)))
    }

    /// Get memberships for a namespace from both primary and fallback graphs.
    ///
    /// Returns an iterator that chains the primary graph's memberships with the
    /// fallback graph's (if present). Avoids the per-call `Vec` allocation that
    /// dominated the resolution profile (~2.8k self samples + 14k extend_desugared).
    fn memberships_combined<'b>(
        &'b self,
        namespace_id: &'b ElementId,
    ) -> impl Iterator<Item = &'a crate::Element> + 'b {
        let primary = self.graph.memberships(namespace_id);
        let fallback = self
            .fallback_graph
            .into_iter()
            .flat_map(move |fg| fg.memberships(namespace_id));
        primary.chain(fallback)
    }

    /// Get owned members from both primary and fallback graphs.
    fn owned_members_combined<'b>(
        &'b self,
        namespace_id: &'b ElementId,
    ) -> impl Iterator<Item = &'a crate::Element> + 'b {
        let primary = self.graph.owned_members(namespace_id);
        let fallback = self
            .fallback_graph
            .into_iter()
            .flat_map(move |fg| fg.owned_members(namespace_id));
        primary.chain(fallback)
    }

    /// Look up elements by name from both primary and fallback graphs.
    fn lookup_by_name_combined<'b>(&'b self, name: &str) -> Vec<&'a ElementId> {
        let mut result: Vec<&ElementId> = self.graph.lookup_by_name(name).iter().collect();
        if let Some(fg) = self.fallback_graph {
            result.extend(fg.lookup_by_name(name).iter());
        }
        result
    }

    /// Get root elements from both primary and fallback graphs.
    fn roots_combined(&self) -> impl Iterator<Item = &'a crate::Element> + '_ {
        let primary = self.graph.roots();
        let fallback = self.fallback_graph.into_iter().flat_map(|fg| fg.roots());
        primary.chain(fallback)
    }

    /// Get the owner of an element, checking both graphs.
    fn owner_of_combined(&self, element_id: &ElementId) -> Option<&'a crate::Element> {
        self.graph
            .owner_of(element_id)
            .or_else(|| self.fallback_graph.and_then(|fg| fg.owner_of(element_id)))
    }

    /// Get collected diagnostics.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Take the collected diagnostics.
    pub fn take_diagnostics(self) -> Diagnostics {
        self.diagnostics
    }

    /// Drain recorded ambiguous-import resolutions (ADR-016 D5).
    ///
    /// Returns `(namespace_id, name, sorted distinct candidate ids)`, ordered
    /// deterministically by `(namespace_id, name)`. Call this *before*
    /// [`take_diagnostics`](Self::take_diagnostics), which consumes `self`.
    pub fn take_ambiguities(&self) -> Vec<(ElementId, String, Vec<ElementId>)> {
        let mut out: Vec<_> = self
            .ambiguity_sink
            .borrow_mut()
            .drain()
            .map(|((ns, name), ids)| (ns, name, ids))
            .collect();
        out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        out
    }

    /// Add a diagnostic.
    pub fn add_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Set the "inside scope" context for visibility checks.
    pub fn set_inside_scope(&mut self, namespace_id: Option<ElementId>) {
        self.inside_scope = namespace_id;
    }

    /// Set the "inheriting" flag for visibility checks.
    pub fn set_inheriting(&mut self, inheriting: bool) {
        self.inheriting = inheriting;
    }

    /// Get or create a scope table for a namespace (owned members only).
    ///
    /// If a cached table exists but has a stale generation, it is rebuilt.
    #[allow(clippy::unwrap_used)] // invariant: we just inserted into scope_tables
    pub fn get_scope_table(&mut self, namespace_id: &ElementId) -> &ScopeTable {
        // Local cache takes precedence over the prebuilt shared map.
        let in_local = self
            .scope_tables
            .get(namespace_id)
            .map(|t| t.generation() >= self.generation)
            .unwrap_or(false);

        // Fast path: local hit, or prebuilt hit (when not present in local).
        // We do the rebuild branch separately so the borrow-checker can see
        // a clean &mut self path that doesn't overlap with the read borrows.
        let prebuilt_hit = !in_local
            && self
                .prebuilt_scope_tables
                .as_ref()
                .and_then(|pre| pre.get(namespace_id))
                .map(|t| t.generation() >= self.generation)
                .unwrap_or(false);

        if !in_local && !prebuilt_hit {
            res_trace!("scope cache miss for {}", namespace_id);
            let mut table = self.build_scope_table(namespace_id);
            table.set_generation(self.generation);
            self.scope_tables.insert(namespace_id.clone(), table);
            return self.scope_tables.get(namespace_id).unwrap();
        }

        if in_local {
            res_trace!("scope cache hit (local) for {}", namespace_id);
            self.scope_tables.get(namespace_id).unwrap()
        } else {
            res_trace!("scope cache hit (prebuilt) for {}", namespace_id);
            self.prebuilt_scope_tables
                .as_ref()
                .and_then(|pre| pre.get(namespace_id))
                .unwrap()
        }
    }

    /// Get or create a FULL scope table for a namespace.
    ///
    /// This includes:
    /// - Owned members (populated immediately)
    /// - Inherited members (populated on first call)
    /// - Imported members (populated on first call)
    ///
    /// This is the main entry point for name resolution - using this cached
    /// table avoids rebuilding inherited/imported lookups on every call.
    #[allow(clippy::unwrap_used)] // invariant: table always exists - just inserted above
    pub fn get_full_scope_table(&mut self, namespace_id: &ElementId) -> &ScopeTable {
        // Fast path: a prebuilt (shared) entry already has owned + inherited +
        // imported populated at the current generation. This is the common case
        // in the parallel resolution path, where every namespace touched here
        // was force-built during `prebuild_scope_tables`.
        let prebuilt_full_hit = !self.scope_tables.contains_key(namespace_id)
            && self
                .prebuilt_scope_tables
                .as_ref()
                .and_then(|pre| pre.get(namespace_id))
                .map(|t| {
                    t.generation() >= self.generation
                        && t.has_inherited_populated()
                        && t.has_imported_populated()
                })
                .unwrap_or(false);
        if prebuilt_full_hit {
            return self
                .prebuilt_scope_tables
                .as_ref()
                .and_then(|pre| pre.get(namespace_id))
                .unwrap();
        }

        // Lazily build the inheritance index on first use
        self.ensure_inheritance_index();

        // Check if the cached table is stale (generation mismatch)
        let is_stale = self
            .scope_tables
            .get(namespace_id)
            .map(|t| t.generation() < self.generation)
            .unwrap_or(false);

        // If stale, remove the cached table so it gets rebuilt from scratch
        if is_stale {
            self.scope_tables.remove(namespace_id);
        }

        // Check if we need to populate inherited/imported
        let needs_inherited = self
            .scope_tables
            .get(namespace_id)
            .map(|t| !t.has_inherited_populated())
            .unwrap_or(true);
        let needs_imported = self
            .scope_tables
            .get(namespace_id)
            .map(|t| !t.has_imported_populated())
            .unwrap_or(true);

        if needs_inherited || needs_imported {
            // Build or get owned members first
            if !self.scope_tables.contains_key(namespace_id) {
                let mut table = self.build_scope_table(namespace_id);
                table.set_generation(self.generation);
                self.scope_tables.insert(namespace_id.clone(), table);
            }

            // Now expand inherited and imported into the cached table
            // We need to remove the table, modify it, and reinsert due to borrow rules
            let mut table = self.scope_tables.remove(namespace_id).unwrap();

            if needs_inherited {
                let redefined = self.collect_redefined_names(namespace_id);
                let mut visited = FxHashSet::default();
                self.expand_inherited(namespace_id, &mut table, &mut visited, &redefined, 0);
                table.set_inherited_populated();
            }

            if needs_imported {
                let mut visited = FxHashSet::default();
                self.expand_imports(namespace_id, &mut table, &mut visited);
                table.set_imported_populated();
            }

            self.scope_tables.insert(namespace_id.clone(), table);
        }

        self.scope_tables.get(namespace_id).unwrap()
    }

    /// Build a scope table for a namespace by collecting owned members.
    fn build_scope_table(&self, namespace_id: &ElementId) -> ScopeTable {
        let mut table = ScopeTable::new();

        // Add owned members (from both primary and fallback graphs)
        for membership in self.memberships_combined(namespace_id) {
            if let Some(view) = MembershipView::try_from_element(membership) {
                if let Some(member_id) = view.member_element() {
                    // Get the member name (from membership or element)
                    let member_name = view
                        .member_name()
                        .map(|s| s.to_owned())
                        .or_else(|| self.lookup_element(member_id).and_then(|e| e.name.clone()));

                    if let Some(name) = member_name {
                        table.add_owned(name, member_id.clone());
                    }

                    // Also add by short name if available
                    if let Some(short_name) = view.member_short_name() {
                        table.add_owned_short(short_name.to_owned(), member_id.clone());
                    }
                }
            }
        }

        table.set_populated();
        table
    }

    /// Expand imports for a namespace and add them to a mutable scope table.
    ///
    /// This processes all Import elements owned by the namespace and adds
    /// the imported members to the scope table.
    fn expand_imports(
        &self,
        namespace_id: &ElementId,
        table: &mut ScopeTable,
        visited_imports: &mut FxHashSet<ElementId>,
    ) {
        // Find all Import elements owned by this namespace (from both graphs)
        let imports: Vec<_> = self
            .owned_members_combined(namespace_id)
            .filter(|e| {
                e.kind == ElementKind::Import
                    || e.kind == ElementKind::NamespaceImport
                    || e.kind == ElementKind::MembershipImport
                    || e.kind.is_subtype_of(ElementKind::Import)
            })
            .collect();

        for import in imports {
            // Skip if already visited (cycle prevention)
            if visited_imports.contains(&import.id) {
                continue;
            }
            visited_imports.insert(import.id.clone());

            // Get import properties
            let imported_ref = import
                .props
                .get(import_props::IMPORTED_REFERENCE)
                .and_then(|v| v.as_str())
                .or_else(|| {
                    import
                        .props
                        .get("importedNamespace")
                        .and_then(|v| v.as_str())
                })
                .or_else(|| {
                    import
                        .props
                        .get("unresolved_importedNamespace")
                        .and_then(|v| v.as_str())
                });

            let is_namespace = import
                .props
                .get(import_props::IS_NAMESPACE)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let is_recursive = import
                .props
                .get(import_props::IS_RECURSIVE)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // The Import's own visibility gates re-export of its imported
            // members (KerML default: private). `isImportAll` (default false)
            // controls whether non-public members of the target are imported.
            let import_visibility = import
                .props
                .get(import_props::VISIBILITY)
                .and_then(|v| v.as_str())
                .and_then(VisibilityKind::from_str)
                .unwrap_or(VisibilityKind::Private);

            let import_all = import
                .props
                .get(import_props::IS_IMPORT_ALL)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if let Some(ref_name) = imported_ref {
                // Try to resolve the imported reference
                if let Some(target_id) = self.resolve_import_target(ref_name) {
                    if is_namespace || is_recursive {
                        // Namespace import: import the target's visible members.
                        self.import_namespace_members(
                            &target_id,
                            table,
                            is_recursive,
                            import_all,
                            import_visibility,
                            visited_imports,
                        );
                    } else {
                        // Membership import: import the specific element. The
                        // recorded visibility is the import's own (re-export).
                        if let Some(target) = self.lookup_element(&target_id) {
                            if let Some(name) = &target.name {
                                table.add_imported(
                                    name.clone(),
                                    target_id.clone(),
                                    import_visibility,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Resolve the target of an import reference.
    ///
    /// Results are cached in `import_cache` to avoid redundant qualified name resolution
    /// when the same import target is referenced multiple times.
    pub(crate) fn resolve_import_target(&self, ref_name: &str) -> Option<ElementId> {
        // Check cache first
        {
            let cache = self.import_cache.borrow();
            if let Some(cached) = cache.get(ref_name) {
                return cached.clone();
            }
        }

        // Cache miss - perform resolution
        let result = self.resolve_import_target_uncached(ref_name);

        // Cache the result (including None for negative caching)
        self.import_cache
            .borrow_mut()
            .insert(ref_name.to_owned(), result.clone());

        result
    }

    /// Inner implementation of import target resolution (uncached).
    #[allow(clippy::indexing_slicing)] // segments[0] safe: checked !is_empty() above
    fn resolve_import_target_uncached(&self, ref_name: &str) -> Option<ElementId> {
        // Try to resolve as a qualified name from root
        let segments = Self::parse_qualified_name_segments(ref_name);
        if segments.is_empty() {
            return None;
        }

        // Find the root element - use name_index for O(1) lookup instead of O(n) scan
        let first = segments[0];
        let stripped_first = first.trim_matches('\'');
        let mut current = None;

        // Try name index first (O(1) lookup, checking both graphs)
        for candidate_name in &[first, stripped_first] {
            for candidate_id in self.lookup_by_name_combined(candidate_name) {
                if let Some(elem) = self.lookup_element(candidate_id) {
                    if elem.owner.is_none() {
                        current = Some(candidate_id.clone());
                        break;
                    }
                }
            }
            if current.is_some() {
                break;
            }
        }

        let mut current = current?;

        // Resolve each subsequent segment by checking owned members directly (both graphs)
        for segment in segments.iter().skip(1) {
            let next = self
                .owned_members_combined(&current)
                .find(|member| {
                    member
                        .name
                        .as_ref()
                        .map(|n| Self::names_match(n, segment))
                        .unwrap_or(false)
                })
                .map(|member| member.id.clone());
            match next {
                Some(id) => current = id,
                None => return None,
            }
        }

        Some(current)
    }

    /// Import the visible members of a namespace into a scope table.
    ///
    /// `import_all` (KerML `Import.isImportAll`) overrides the public-only
    /// filter, importing members of any declared visibility. `import_visibility`
    /// is the importing Import's own visibility, recorded against each imported
    /// name so it gates re-export when this namespace is itself imported.
    fn import_namespace_members(
        &self,
        namespace_id: &ElementId,
        table: &mut ScopeTable,
        recursive: bool,
        import_all: bool,
        import_visibility: VisibilityKind,
        visited: &mut FxHashSet<ElementId>,
    ) {
        // Skip if already processed
        if visited.contains(namespace_id) {
            return;
        }
        visited.insert(namespace_id.clone());

        // Add visible members (from both graphs)
        for membership in self.memberships_combined(namespace_id) {
            if let Some(view) = MembershipView::try_from_element(membership) {
                // Only public members are visible unless `isImportAll` is set.
                if !import_all && view.visibility() != VisibilityKind::Public {
                    continue;
                }

                if let Some(member_id) = view.member_element() {
                    // Get the member name
                    let member_name = view
                        .member_name()
                        .map(|s| s.to_owned())
                        .or_else(|| self.lookup_element(member_id).and_then(|e| e.name.clone()));

                    if let Some(name) = member_name {
                        table.add_imported(name, member_id.clone(), import_visibility);
                    }

                    // If recursive, also import from nested namespaces
                    if recursive {
                        if let Some(member) = self.lookup_element(member_id) {
                            // Check if member is a namespace (Package, etc.)
                            if member.kind == ElementKind::Package
                                || member.kind == ElementKind::Namespace
                                || member.kind.is_subtype_of(ElementKind::Namespace)
                            {
                                self.import_namespace_members(
                                    member_id,
                                    table,
                                    true,
                                    import_all,
                                    import_visibility,
                                    visited,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Follow re-exports: any PUBLIC import on the target namespace
        // re-exports its members through this import (KerML: a public Import's
        // memberships are visible from outside the importOwningNamespace). The
        // recorded visibility stays the outer import's (these are members
        // imported by the OUTER namespace); the recursive member-visibility
        // filter uses the re-export import's own isImportAll.
        for re_export in self.owned_members_combined(namespace_id) {
            let is_import = re_export.kind == ElementKind::Import
                || re_export.kind == ElementKind::NamespaceImport
                || re_export.kind == ElementKind::MembershipImport
                || re_export.kind.is_subtype_of(ElementKind::Import);
            if !is_import {
                continue;
            }
            let re_visibility = re_export
                .props
                .get(import_props::VISIBILITY)
                .and_then(|v| v.as_str())
                .and_then(VisibilityKind::from_str)
                .unwrap_or(VisibilityKind::Private);
            if re_visibility != VisibilityKind::Public {
                continue;
            }
            let imported_ref = re_export
                .props
                .get(import_props::IMPORTED_REFERENCE)
                .and_then(|v| v.as_str())
                .or_else(|| {
                    re_export
                        .props
                        .get("unresolved_importedNamespace")
                        .and_then(|v| v.as_str())
                });
            let Some(ref_name) = imported_ref else {
                continue;
            };
            let Some(target_id) = self.resolve_import_target(ref_name) else {
                continue;
            };
            let re_is_namespace = re_export
                .props
                .get(import_props::IS_NAMESPACE)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let re_is_recursive = re_export
                .props
                .get(import_props::IS_RECURSIVE)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let re_import_all = re_export
                .props
                .get(import_props::IS_IMPORT_ALL)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if re_is_namespace || re_is_recursive {
                self.import_namespace_members(
                    &target_id,
                    table,
                    re_is_recursive,
                    re_import_all,
                    import_visibility,
                    visited,
                );
            } else if let Some(target) = self.lookup_element(&target_id) {
                if let Some(name) = &target.name {
                    table.add_imported(name.clone(), target_id.clone(), import_visibility);
                }
            }
        }
    }

    /// Expand inherited members for a Type and add them to the scope table.
    ///
    /// This processes all Specialization elements that have this Type as the
    /// specific type and adds the general type's members to the scope table.
    ///
    /// The `depth` parameter prevents infinite recursion in pathological cases.
    fn expand_inherited(
        &self,
        type_id: &ElementId,
        table: &mut ScopeTable,
        visited: &mut FxHashSet<ElementId>,
        redefined: &FxHashSet<String>,
        depth: usize,
    ) {
        // Safety limit to prevent infinite recursion
        if depth > MAX_INHERITANCE_DEPTH {
            return;
        }

        // Check if this is a Type (only Types can have specializations)
        let Some(type_element) = self.lookup_element(type_id) else {
            return;
        };

        // Only process Types (Definition, Usage, Classifier, etc.)
        if !type_element.kind.is_subtype_of(ElementKind::Type)
            && type_element.kind != ElementKind::Type
        {
            return;
        }

        // Skip if already processed (cycle prevention)
        if visited.contains(type_id) {
            return;
        }
        visited.insert(type_id.clone());

        // Fast path: use pre-computed InheritanceIndex for O(1) supertype lookup
        if let Some(ref index) = self.inheritance_index {
            let supertypes = index.supertypes(type_id);
            if !supertypes.is_empty() {
                for gid in supertypes {
                    self.add_inherited_members(gid, table, visited, redefined, depth);
                }
                return;
            }
            // Index has no entry for this type — fall through to owned_members scan
            // (the type may have unresolved specializations not yet in the index)
        }

        // Fallback: scan owned_members for specialization-family elements (both graphs).
        // Cloned so we don't hold an immutable borrow of `self` while resolving below.
        let specializations: Vec<crate::Element> = self
            .owned_members_combined(type_id)
            .filter(|e| {
                e.kind == ElementKind::Specialization
                    || e.kind.is_subtype_of(ElementKind::Specialization)
            })
            .cloned()
            .collect();

        for spec in specializations {
            // Each specialization-family kind stores its supertype under a
            // different property pair (Subclassification -> superclassifier,
            // FeatureTyping -> type, Subsetting -> subsettedFeature, Redefinition
            // -> redefinedFeature, plain Specialization -> general). Reading only
            // `general` dropped every non-plain-Specialization supertype, which is
            // why definitions' `:> Super` chains never reached the inherited tier.
            let Some((resolved_key, unresolved_key)) =
                Self::specialization_target_props(&spec.kind)
            else {
                continue;
            };

            // FI-2 FIX: Prioritize already-resolved ElementId to avoid losing package context.
            let general_id: Option<ElementId> = spec
                .props
                .get(resolved_key)
                .and_then(|v| v.as_ref())
                .cloned()
                // Fallback: resolve unresolved name if not yet resolved
                .or_else(|| {
                    let ref_name = spec.props.get(unresolved_key).and_then(|v| v.as_str())?;

                    // Try qualified name resolution, then library packages
                    self.resolve_import_target(ref_name)
                        .or_else(|| self.resolve_in_library_packages(ref_name))
                });

            if let Some(gid) = general_id {
                // Add inherited members from the general type
                self.add_inherited_members(&gid, table, visited, redefined, depth);
            }
        }
    }
    /// Add members from a supertype to the inherited section of the scope table.
    ///
    /// The `depth` parameter is passed through to prevent infinite recursion.
    fn add_inherited_members(
        &self,
        supertype_id: &ElementId,
        table: &mut ScopeTable,
        visited: &mut FxHashSet<ElementId>,
        redefined: &FxHashSet<String>,
        depth: usize,
    ) {
        // Get public and protected members from supertype (from both graphs)
        for membership in self.memberships_combined(supertype_id) {
            if let Some(view) = MembershipView::try_from_element(membership) {
                // Inherit public and protected members (not private)
                let visibility = view.visibility();
                if visibility == VisibilityKind::Private {
                    continue;
                }

                if let Some(member_id) = view.member_element() {
                    // Get the member name
                    let member_name = view
                        .member_name()
                        .map(|s| s.to_owned())
                        .or_else(|| self.lookup_element(member_id).and_then(|e| e.name.clone()));

                    if let Some(name) = member_name {
                        // Skip if this name is redefined
                        if redefined.contains(&name) {
                            continue;
                        }
                        table.add_inherited(name, member_id.clone());
                    }
                }
            }
        }

        // Recursively add inherited members from supertype's supertypes
        self.expand_inherited(supertype_id, table, visited, redefined, depth + 1);
    }

    /// Collect redefined feature names from a type.
    fn collect_redefined_names(&self, type_id: &ElementId) -> FxHashSet<String> {
        let mut redefined = FxHashSet::default();

        // Find all Redefinition elements (from both graphs)
        for member in self.owned_members_combined(type_id) {
            if member.kind == ElementKind::Redefinition
                || member.kind.is_subtype_of(ElementKind::Redefinition)
            {
                // Get the redefined feature name
                if let Some(name) = member
                    .props
                    .get(unresolved_props::REDEFINED_FEATURE)
                    .and_then(|v| v.as_str())
                {
                    // Extract just the name part (last segment of qualified name)
                    let name_part = name.rsplit("::").next().unwrap_or(name);
                    redefined.insert(name_part.to_owned());
                }
            }
        }

        redefined
    }

    /// Resolve a simple name within a namespace.
    ///
    /// Follows the precedence: OWNED -> INHERITED -> IMPORTED -> PARENT -> GLOBAL
    pub fn resolve_name(&mut self, namespace_id: &ElementId, name: &str) -> Option<ElementId> {
        // Check for cycles
        if self.visiting.contains(namespace_id) {
            return None;
        }
        self.visiting.insert(namespace_id.clone());

        let result = self.resolve_name_inner(namespace_id, name);

        self.visiting.remove(namespace_id);
        result
    }

    /// Inner resolution logic.
    ///
    /// Uses negative lookup caching to avoid re-walking parent hierarchies
    /// for names that have already failed resolution from a given namespace.
    fn resolve_name_inner(&mut self, namespace_id: &ElementId, name: &str) -> Option<ElementId> {
        // Check negative cache first - avoid redundant parent walking for known failures
        {
            let cache = self.failed_lookups.borrow();
            if cache.contains(&(namespace_id.clone(), name.to_owned())) {
                return None;
            }
        }

        // 0. PRIMITIVE ALIASES: Check if this is a primitive type alias
        // e.g., "float" -> "Real", "int" -> "Integer"
        if let Some(canonical) = primitive_type_alias(name) {
            let result = self.resolve_name_inner(namespace_id, canonical);
            // If the canonical name failed, also cache the alias as failed
            if result.is_none() {
                self.failed_lookups
                    .borrow_mut()
                    .insert((namespace_id.clone(), name.to_owned()));
            }
            return result;
        }

        // ADR-016 D5: ambiguity flagging is gated to the USER (fallback) path.
        // Capture the flag before borrowing the scope table (which holds a
        // `&mut self` borrow for its lifetime, so `self.fallback_graph` is
        // otherwise unreachable inside the block below).
        let has_fallback = self.fallback_graph.is_some();

        // ADR-016 D5: deferred ambiguity record, carrying
        // `(sorted distinct candidates, deterministic pick)` out of the scope
        // table borrow below so the `ambiguity_sink` insert (which also touches
        // `self`) happens only after `table` is dropped.
        let mut ambiguity: Option<(Vec<ElementId>, ElementId)> = None;

        // Use the cached full scope table (owned + inherited + imported)
        // This avoids rebuilding the table on every lookup - critical for performance!
        // We do lookups first, then extract parent_id to avoid keeping borrow alive
        let (_found_in_table, parent_id) = {
            let table = self.get_full_scope_table(namespace_id);

            // 1. OWNED: Check local owned members
            if let Some(id) = table.lookup_owned(name) {
                return Some(id.clone());
            }

            // 2. INHERITED: Check inherited members (for Types)
            if let Some(id) = table.lookup_inherited(name) {
                return Some(id.clone());
            }

            // 3. IMPORTED: Check imported members
            if let Some(id) = table.lookup_imported(name) {
                // On the USER (fallback) path, a name brought in by two+ imports
                // resolving to *distinct* ids is ambiguous. Record the collision
                // deterministically and resolve to the minimum candidate id
                // (instead of the FxHashMap last-wins value) so the pick is
                // stable across runs. Library self-resolution (no fallback) is
                // never flagged — its benign re-export collisions stay silent.
                if has_fallback {
                    if let Some(candidates) = table.ambiguous_imported(name) {
                        let mut sorted: Vec<ElementId> = candidates.to_vec();
                        sorted.sort();
                        let pick = sorted.first().cloned().unwrap_or_else(|| id.clone());
                        // Defer the sink insert past the `table` borrow; skip the
                        // parent walk by yielding a `None` parent below.
                        ambiguity = Some((sorted, pick));
                        (true, None)
                    } else {
                        return Some(id.clone());
                    }
                } else {
                    return Some(id.clone());
                }
            } else {
                // Get parent ID while we have immutable borrow (check both graphs)
                (
                    false,
                    self.owner_of_combined(namespace_id).map(|e| e.id.clone()),
                )
            }
        };

        // ADR-016 D5: flush the deferred ambiguity record now that `table` is
        // dropped, then return the deterministic pick.
        if let Some((sorted, pick)) = ambiguity {
            self.ambiguity_sink
                .borrow_mut()
                .insert((namespace_id.clone(), name.to_owned()), sorted);
            return Some(pick);
        }

        // 4. PARENT: Walk up to parent namespace
        if let Some(owner_id) = parent_id {
            if let Some(id) = self.resolve_name(&owner_id, name) {
                return Some(id);
            }
        }

        // 5. GLOBAL: Check root packages (from both primary and fallback graphs)
        for root in self.roots_combined() {
            if root.name.as_ref().map(|n| n == name).unwrap_or(false) {
                return Some(root.id.clone());
            }
        }

        // 6. LIBRARY: Check library package members
        // This allows implicit access to standard library types without imports.
        //
        // Import-gate (ADR-016): when `gate_bare_library` is ON, this
        // spec-violating bare-name *member* sweep is skipped. We still resolve a
        // bare single-segment *top-level library package name* (spec-legal
        // global scope), so `ScalarValues` resolves but the member `Real` does
        // not (it then requires `import ScalarValues::*;`). Default OFF keeps
        // the historical sweep — zero behavior change.
        if self.gate_bare_library {
            if let Some(id) = self.resolve_library_package_name(name) {
                return Some(id);
            }
        } else if let Some(id) = self.resolve_in_library_packages(name) {
            return Some(id);
        }

        // Cache this failure to avoid re-walking from this namespace for this name
        self.failed_lookups
            .borrow_mut()
            .insert((namespace_id.clone(), name.to_owned()));

        None
    }

    /// Resolve a name by searching library package contents.
    ///
    /// This searches the public members of all registered library packages,
    /// including nested packages recursively.
    /// For example, resolving "Anything" will search in "Base" library package.
    ///
    /// Uses a two-tier strategy:
    /// 1. **Fast path**: O(1) FxHashMap lookup via `graph.resolve_in_library()`
    /// 2. **Fallback**: Recursive O(k*d*m) scan only if the fast path misses
    pub(crate) fn resolve_in_library_packages(&self, name: &str) -> Option<ElementId> {
        // Check library-specific negative cache first (namespace-independent)
        if self.failed_library_lookups.borrow().contains(name) {
            return None;
        }

        // O(1) lookup in the pre-built library name index — the only path.
        //
        // The index is built by `ModelGraph::build_library_index`
        // (`graph.rs::index_library_recursively`) over every public member of
        // every `library_packages` entry, indexed under both quoted and
        // unquoted forms. It is, by construction, the single source of truth
        // for library name resolution; a name not in the index is not in the
        // library (re-exports resolve to their real owner, which IS indexed).
        // The previous recursive fallback (search_library_recursively + the
        // public-import-following branch) walked the same roots, applied the
        // same visibility filter, and so could never find a name the index
        // didn't already have — but a 30-min CPU sample of `sysml inspect
        // definitions.sysml --diagnostics` showed it dominating ~36 % of
        // exclusive samples during `prebuild_scope_tables`. The cost was
        // entirely confirming `None`. Removed; misses now return `None`
        // promptly and propagate to the standard unresolved-supertype /
        // missing-import diagnostics at the original reference.
        if let Some(id) = self.graph.resolve_in_library(name) {
            return Some(id.clone());
        }
        if let Some(fg) = self.fallback_graph {
            if let Some(id) = fg.resolve_in_library(name) {
                return Some(id.clone());
            }
        }

        // Cache this library-level failure to avoid repeated index lookups
        // for the same missing name across resolution passes.
        self.failed_library_lookups
            .borrow_mut()
            .insert(name.to_owned());
        None
    }

    /// Check if a name is a pure feature chain (contains '.' but not '::').
    ///
    /// Feature chains are dot-separated paths like `vehicle.engine.pistons` that
    /// require feature chaining resolution (each segment is resolved in the type
    /// of the previous segment).
    ///
    /// Names containing `::` are qualified names and should be handled by
    /// `resolve_qualified_name` even if they also contain dots (e.g., `A::B.c`).
    ///
    /// # Examples
    /// - `"a.b"` -> true (pure feature chain)
    /// - `"a.b.c"` -> true (pure feature chain)
    /// - `"A::B"` -> false (qualified name)
    /// - `"A::B.c"` -> false (qualified name with feature access - not pure chain)
    /// - `"simple"` -> false (simple name)
    /// - `"'a.b'"` -> false (dot is inside quotes)
    pub(crate) fn is_feature_chain(name: &str) -> bool {
        // If it contains ::, it's a qualified name, not a pure feature chain
        // (even if it also has dots like A::B.c)
        if name.contains("::") {
            return false;
        }

        let mut in_quotes = false;
        for c in name.chars() {
            match c {
                '\'' => in_quotes = !in_quotes,
                '.' if !in_quotes => return true,
                _ => {}
            }
        }
        false
    }

    /// Split a feature chain into segments by '.', respecting quoted names.
    ///
    /// Returns an iterator to avoid allocation in the common case.
    ///
    /// # Examples
    /// - `"a.b.c"` -> `["a", "b", "c"]`
    /// - `"'a.b'.c"` -> `["'a.b'", "c"]`
    pub(crate) fn split_feature_chain_segments(chain: &str) -> impl Iterator<Item = &str> {
        struct ChainIter<'a> {
            remaining: &'a str,
        }

        impl<'a> Iterator for ChainIter<'a> {
            type Item = &'a str;

            fn next(&mut self) -> Option<Self::Item> {
                if self.remaining.is_empty() {
                    return None;
                }

                let mut in_quotes = false;
                let mut end = 0;

                for (i, c) in self.remaining.char_indices() {
                    match c {
                        '\'' => in_quotes = !in_quotes,
                        '.' if !in_quotes => {
                            let segment = &self.remaining[..i];
                            self.remaining = &self.remaining[i + 1..];
                            return Some(segment);
                        }
                        _ => {}
                    }
                    end = i + c.len_utf8();
                }

                // Last segment
                let segment = &self.remaining[..end];
                self.remaining = "";
                Some(segment)
            }
        }

        ChainIter { remaining: chain }
    }

    /// Parse a qualified name into segments, respecting quoted names.
    ///
    /// Handles cases like "DataFunctions::'/'" where the segment contains `::`.
    /// Returns segments with quotes preserved.
    ///
    /// # Examples
    /// - `"Package::Element"` -> `["Package", "Element"]`
    /// - `"DataFunctions::'/'"` -> `["DataFunctions", "'/'"]`
    /// - `"ScalarFunctions::'-'"` -> `["ScalarFunctions", "'-'"]`
    #[allow(clippy::indexing_slicing)] // byte indexing safe: loop bounds checked
    pub(crate) fn parse_qualified_name_segments(qname: &str) -> Vec<&str> {
        let mut segments = Vec::new();
        let mut start = 0;
        let mut in_quotes = false;
        let bytes = qname.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            if bytes[i] == b'\'' {
                in_quotes = !in_quotes;
                i += 1;
            } else if !in_quotes && i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':'
            {
                if i > start {
                    segments.push(&qname[start..i]);
                }
                i += 2;
                start = i;
            } else {
                i += 1;
            }
        }
        if start < qname.len() {
            segments.push(&qname[start..]);
        }
        segments
    }

    /// Check if two names match, stripping quotes if present.
    ///
    /// Handles quoted operator names like `'/'` matching `'/'` or `/`.
    pub(crate) fn names_match(name: &str, target: &str) -> bool {
        let name = name.trim_matches('\'');
        let target = target.trim_matches('\'');
        name == target
    }

    /// Resolve a feature chain reference (dot-separated path like "a.b.c").
    ///
    /// Feature chains are used in expressions like `vehicle.engine.pistons` where
    /// each segment after the first is resolved in the type of the previous segment.
    ///
    /// # Algorithm
    /// 1. Split chain into segments by '.'
    /// 2. Resolve first segment in current scope (normal name resolution)
    /// 3. For each subsequent segment, use feature chaining strategy
    ///
    /// # Performance
    /// O(m * (k + d)) where:
    /// - m = number of segments in chain
    /// - k = average owned features per type
    /// - d = average inheritance depth
    ///
    /// Uses O(1) reverse indexes for type and specialization lookups.
    pub fn resolve_feature_chain(
        &mut self,
        namespace_id: &ElementId,
        chain: &str,
    ) -> Option<ElementId> {
        let mut segments = Self::split_feature_chain_segments(chain);

        // Step 1: Resolve first segment in normal scope
        let first_segment = segments.next()?;
        let mut current_id = self.resolve_name(namespace_id, first_segment)?;

        // Step 2: For each subsequent segment, use feature chaining
        for segment in segments {
            let resolution =
                scoping::resolve_with_feature_chaining(self.graph, &current_id, segment);
            match resolution {
                scoping::ScopedResolution::Found(id) => {
                    current_id = id;
                }
                _ => {
                    return None;
                }
            }
        }

        Some(current_id)
    }

    /// The prefix used for global qualification in SysML v2 qualified names.
    ///
    /// When a qualified name starts with `$::`, resolution anchors to the root
    /// namespace instead of the current scope.
    /// Spec: KerMLExpressions.xtext:541-543
    const GLOBAL_QUALIFICATION_PREFIX: &'static str = "$::";

    /// Strip the `$::` global qualification prefix from a qualified name, if present.
    ///
    /// Returns `Some(remainder)` if the prefix was present, `None` otherwise.
    fn strip_global_qualification(qname: &str) -> Option<&str> {
        qname.strip_prefix(Self::GLOBAL_QUALIFICATION_PREFIX)
    }

    /// Resolve a qualified name (e.g., "Package::SubPackage::Element") or feature chain (e.g., "a.b.c").
    ///
    /// Starts from the given namespace and resolves each segment.
    /// If local resolution fails for the first segment, falls back to global resolution.
    ///
    /// # Global Qualification
    /// If the name starts with `$::`, resolution anchors to the root namespace.
    /// For example, `$::TopPackage::Element` always starts from root, regardless
    /// of the current scope. (Spec: KerMLExpressions.xtext:541-551)
    ///
    /// # Feature Chains
    /// If the name contains '.' (outside of quotes), it's treated as a feature chain
    /// and resolved using feature chaining strategy.
    #[allow(clippy::indexing_slicing)] // segments[0] safe: checked !is_empty() above
    pub fn resolve_qualified_name(
        &mut self,
        namespace_id: &ElementId,
        qname: &str,
    ) -> Option<ElementId> {
        // Handle $:: global qualification: anchor to root namespace
        if let Some(remainder) = Self::strip_global_qualification(qname) {
            return self.resolve_qualified_name_global(remainder);
        }

        // Check for feature chain (contains '.' not in quotes)
        if Self::is_feature_chain(qname) {
            return self.resolve_feature_chain(namespace_id, qname);
        }

        let segments = Self::parse_qualified_name_segments(qname);
        if segments.is_empty() {
            return None;
        }

        // First segment: resolve in the current scope
        let first = segments[0];
        let mut current = self.resolve_name(namespace_id, first);

        // If local resolution failed, try global resolution (roots).
        // Ridge B: indexed lookup instead of linear `roots().find()`.
        if current.is_none() {
            current = self.graph.lookup_root_by_name(first).map(|r| r.id.clone());
        }

        // Also search library packages if still not found.
        //
        // Import-gate (ADR-016): when gated, the first segment may legitimately
        // be a top-level library package name (`ScalarValues::Real` — the
        // qualified name's root). We keep that resolvable but drop the bare
        // *member* sweep, so a single-segment "qualified" name that is actually
        // a library member no longer silently resolves without an import.
        if current.is_none() {
            current = if self.gate_bare_library {
                self.resolve_library_package_name(first)
            } else {
                self.resolve_in_library_packages(first)
            };
        }

        let mut current = current?;

        // Subsequent segments: resolve in the resolved element's scope
        for segment in segments.iter().skip(1) {
            current = self.resolve_name(&current, segment)?;
        }

        Some(current)
    }

    /// Resolve a qualified name from root (global).
    ///
    /// The first segment must be a root package name.
    #[allow(clippy::indexing_slicing)] // segments[0] safe: checked !is_empty() above
    pub fn resolve_qualified_name_global(&mut self, qname: &str) -> Option<ElementId> {
        let segments = Self::parse_qualified_name_segments(qname);
        if segments.is_empty() {
            return None;
        }

        // First segment: find root package or library package.
        //
        // Ridge B: use the indexed `lookup_root_by_name` helper instead of a
        // linear `roots().find()` walk. The post-Ridge-A.2 workspace-merged
        // graph absorbs every stdlib top-level package into root_ids, so the
        // linear scan was the May 29 §6 profile's 68.5 %-exclusive frame.
        let first = segments[0];
        let mut current = self
            .graph
            .lookup_root_by_name(first)
            .map(|r| r.id.clone())
            // Also search library packages for the first segment
            .or_else(|| self.resolve_in_library_packages(first))?;

        // Subsequent segments: resolve in the resolved element's scope
        for segment in segments.iter().skip(1) {
            current = self.resolve_name(&current, segment)?;
        }

        Some(current)
    }

    /// Resolve a qualified name using relative scoping for subsequent segments.
    ///
    /// For `A::B::C`:
    /// - A: full resolution (owned + inherited + imported + parent walking)
    /// - B: relative resolution in A (owned + inherited + imported, NO parent walking)
    /// - C: relative resolution in B
    ///
    /// If the name starts with `$::`, it is anchored to root via `resolve_qualified_name_global`.
    ///
    /// This is used for feature references like `useCase::actor` where we want to
    /// find `actor` only within `useCase`, not in parent scopes.
    #[allow(clippy::indexing_slicing)] // segments[0] safe: checked !is_empty() above
    pub fn resolve_qualified_name_relative(
        &mut self,
        namespace_id: &ElementId,
        qname: &str,
    ) -> Option<ElementId> {
        // Handle $:: global qualification: anchor to root namespace
        if let Some(remainder) = Self::strip_global_qualification(qname) {
            return self.resolve_qualified_name_global(remainder);
        }

        let segments = Self::parse_qualified_name_segments(qname);
        if segments.is_empty() {
            return None;
        }

        // First segment: normal resolution (walks up parent scopes)
        let mut current = self.resolve_name(namespace_id, segments[0])?;

        // Subsequent segments: relative resolution only (no parent walking)
        for segment in segments.iter().skip(1) {
            match scoping::resolve_in_relative_namespace(self.graph, &current, segment) {
                scoping::ScopedResolution::Found(id) => current = id,
                _ => return None,
            }
        }

        Some(current)
    }

    /// Resolve a feature reference that may be:
    /// - Simple name: "actor"
    /// - Qualified path: "useCase::actor" (navigate into owned namespace with relative scoping)
    /// - Feature chain: "subject.attribute" (navigate into type)
    /// - Global reference: "$::Pkg::Element" (root-anchored)
    ///
    /// This is the appropriate method for resolving redefinition, subsetting,
    /// and reference subsetting targets.
    pub fn resolve_feature_reference(
        &mut self,
        scope_id: &ElementId,
        reference: &str,
    ) -> Option<ElementId> {
        // Handle $:: global qualification: anchor to root namespace
        if let Some(remainder) = Self::strip_global_qualification(reference) {
            return self.resolve_qualified_name_global(remainder);
        }

        // Feature chain with '.' -> navigate into type
        if Self::is_feature_chain(reference) {
            return self.resolve_feature_chain(scope_id, reference);
        }

        // Qualified name with '::' -> navigate into owned namespace (relative scoping)
        if reference.contains("::") {
            return self.resolve_qualified_name_relative(scope_id, reference);
        }

        // Simple name -> normal resolution
        self.resolve_name(scope_id, reference)
    }

    /// Resolve a redefined feature reference.
    ///
    /// For redefinitions, we need to find the INHERITED feature being redefined,
    /// not the OWNED feature that's doing the redefining. This method:
    /// 1. Finds the containing type (walks up from scope until reaching a Type)
    /// 2. Looks in inherited members first (the redefined feature should be inherited)
    /// 3. Falls back to normal resolution if not found in inherited
    ///
    /// This handles the common case where `:>> id = value` creates a feature that
    /// redefines an inherited `id` - we need to find the inherited one, not the new one.
    pub fn resolve_redefined_feature(
        &mut self,
        scope_id: &ElementId,
        reference: &str,
    ) -> Option<ElementId> {
        // Feature chain with '.' -> chain navigation ONLY. KerML grants no
        // last-segment concession to a genuine feature-chain redefinedFeature:
        // the chain resolves by navigation or the resolution fails.
        if Self::is_feature_chain(reference) {
            return self.resolve_feature_chain(scope_id, reference);
        }

        // Qualified name with '::' -> use relative resolution
        if reference.contains("::") {
            return self.resolve_qualified_name_relative(scope_id, reference);
        }

        // KerML 8.2.3.5.1 (derived text KerML-spec-r2025-04.txt:6439): "The
        // basic name resolution process is used directly … in all cases except
        // when the qualified name specifies the redefinedFeature of a
        // Redefinition with an owningFeature that has an owningType. In this
        // case, the basic name resolution process is repeated with the general
        // Type of each ownedSpecialization of the owningType considered in
        // turn as the local Namespace, until a resolution is found. If no
        // resolution is found for any of these, then the overall resolution
        // fails." — a HARD failure: no lexical scope-walk fallback.
        //
        // `scope_id` is the Redefinition's owningFeature (the driver passes
        // element.owner). Its owner is the owningType — a Feature counts (a
        // Feature IS a Type), which is exactly how a FlowEnd's inner
        // FlowFeature resolves: the FlowEnd's ownedSpecializations include its
        // FlowEndSubsetting, whose general (the resolved end prefix) is the
        // namespace the bare last segment resolves against
        // (SysML-spec-r2025-04.txt:21842-21865 desugaring).
        let owning_type = self.lookup_element(scope_id).and_then(|f| f.owner.clone());
        let Some(owning_type) = owning_type.filter(|ot| {
            self.lookup_element(ot).is_some_and(|e| {
                e.kind == ElementKind::Type || e.kind.is_subtype_of(ElementKind::Type)
            })
        }) else {
            // No owningType: the 6439 exception does not apply and the basic
            // resolution process resolves the name relative to the local
            // namespace directly.
            return self.resolve_name(scope_id, reference);
        };
        self.resolve_in_owned_specialization_generals(&owning_type, reference)
    }

    /// KerML 8.2.3.5.1: resolve `name` with the general Type of each
    /// ownedSpecialization of `owning_type` considered in turn (source order)
    /// as the local Namespace — the general's owned members first, then its
    /// inherited members (its own specialization closure). Returns `None` when
    /// no general resolves the name: redefined-feature resolution then FAILS
    /// (E200), with no further fallback.
    fn resolve_in_owned_specialization_generals(
        &mut self,
        owning_type: &ElementId,
        name: &str,
    ) -> Option<ElementId> {
        // Collect the owned specialization-family relationships, source-ordered
        // (children_of iterates a hash set). `children_of` (owner-index) rather
        // than the membership-based owned_members view: elaboration passes mint
        // implied specializations (e.g. implicit_generalization) with an owner
        // but no wrapping OwningMembership, and those edges are exactly as
        // load-bearing for redefinition resolution as authored ones.
        let mut specs: Vec<crate::Element> = self
            .graph
            .children_of(owning_type)
            .chain(
                self.fallback_graph
                    .into_iter()
                    .flat_map(|fg| fg.children_of(owning_type)),
            )
            .filter(|e| {
                e.kind == ElementKind::Specialization
                    || e.kind.is_subtype_of(ElementKind::Specialization)
            })
            .cloned()
            .collect();
        specs.sort_by_key(|e| e.spans.first().map(|s| s.start).unwrap_or(usize::MAX));

        for rel in &specs {
            // Skip a sibling Redefinition whose own target is still unresolved
            // (resolving it here would recurse); resolved ones contribute their
            // general like any other specialization.
            if (rel.kind == ElementKind::Redefinition
                || rel.kind.is_subtype_of(ElementKind::Redefinition))
                && rel.props.get(resolved_props::REDEFINED_FEATURE).is_none()
            {
                continue;
            }
            let Some(general) = self.resolve_supertype_target(owning_type, rel) else {
                continue;
            };
            if general == *owning_type {
                continue;
            }
            // Basic resolution with `general` as the local Namespace: its owned
            // members, then its inherited members via its own supertype closure
            // (combined-graph aware, so library generals work).
            if let Some(id) = self.find_owned_member_by_name(&general, name) {
                return Some(id);
            }
            if let Some(id) = self.search_supertypes_for_feature(&general, name) {
                return Some(id);
            }
        }
        None
    }

    /// Look up a named owned member of `namespace_id` (combined-graph view).
    fn find_owned_member_by_name(
        &mut self,
        namespace_id: &ElementId,
        name: &str,
    ) -> Option<ElementId> {
        for membership in self.memberships_combined(namespace_id) {
            if let Some(view) = MembershipView::try_from_element(membership) {
                if let Some(member_id) = view.member_element() {
                    let member_name = view.member_name().map(|s| s.to_owned()).or_else(|| {
                        self.lookup_element(member_id).and_then(|e| e.name.clone())
                    });
                    if member_name.as_deref() == Some(name) {
                        return Some(member_id.clone());
                    }
                }
            }
        }
        None
    }

    /// Map a specialization-family relationship kind to the (resolved, unresolved)
    /// property keys that hold its supertype/target reference.
    ///
    /// All of these kinds are subtypes of `Specialization`, but each stores its
    /// target under a distinct property name. FeatureTyping is checked first
    /// because it is the only one that is NOT a `Subclassification`/`Subsetting`
    /// descendant. Returns `None` for kinds we don't follow for inheritance.
    fn specialization_target_props(kind: &ElementKind) -> Option<(&'static str, &'static str)> {
        // Order matters: check the most specific kinds before their supertypes.
        if *kind == ElementKind::Redefinition || kind.is_subtype_of(ElementKind::Redefinition) {
            Some((
                resolved_props::REDEFINED_FEATURE,
                unresolved_props::REDEFINED_FEATURE,
            ))
        } else if *kind == ElementKind::ReferenceSubsetting
            || kind.is_subtype_of(ElementKind::ReferenceSubsetting)
        {
            // A Subsetting subtype, but its target lives under referencedFeature
            // (e.g. a FlowEnd's FlowEndSubsetting) — must be matched before the
            // generic Subsetting arm or its target reads as absent.
            Some((
                resolved_props::REFERENCED_FEATURE,
                unresolved_props::REFERENCED_FEATURE,
            ))
        } else if *kind == ElementKind::CrossSubsetting
            || kind.is_subtype_of(ElementKind::CrossSubsetting)
        {
            Some((
                resolved_props::CROSSED_FEATURE,
                unresolved_props::CROSSED_FEATURE,
            ))
        } else if *kind == ElementKind::Subsetting || kind.is_subtype_of(ElementKind::Subsetting) {
            Some((
                resolved_props::SUBSETTED_FEATURE,
                unresolved_props::SUBSETTED_FEATURE,
            ))
        } else if *kind == ElementKind::Subclassification
            || kind.is_subtype_of(ElementKind::Subclassification)
        {
            Some((
                resolved_props::SUPERCLASSIFIER,
                unresolved_props::SUPERCLASSIFIER,
            ))
        } else if *kind == ElementKind::FeatureTyping
            || kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            Some((resolved_props::TYPE, unresolved_props::TYPE))
        } else if *kind == ElementKind::Specialization
            || kind.is_subtype_of(ElementKind::Specialization)
        {
            Some((resolved_props::GENERAL, unresolved_props::GENERAL))
        } else {
            None
        }
    }

    /// Resolve the supertype/target element a specialization-family relationship
    /// points at, using the property pair appropriate for its concrete kind.
    ///
    /// Prefers the already-resolved property (pass 1 normally fills these in),
    /// falling back to resolving the unresolved reference name through the same
    /// tiers used during pass 1 (local scope, then imports, then library packages).
    fn resolve_supertype_target(
        &mut self,
        scope_type_id: &ElementId,
        rel: &crate::Element,
    ) -> Option<ElementId> {
        let (resolved_key, unresolved_key) = Self::specialization_target_props(&rel.kind)?;

        if let Some(id) = rel.props.get(resolved_key).and_then(|v| v.as_ref()) {
            return Some(id.clone());
        }

        let ref_name = rel.props.get(unresolved_key).and_then(|v| v.as_str())?;
        if Self::is_feature_chain(ref_name) {
            // Dotted target (e.g. a FlowEndSubsetting whose end prefix is
            // itself a chain, `a.b.` of `a.b.out`): navigate it — the
            // simple-name tiers below cannot see dotted references.
            self.resolve_feature_chain(scope_type_id, ref_name)
        } else if ref_name.contains("::") {
            self.resolve_qualified_name(scope_type_id, ref_name)
        } else {
            self.resolve_name(scope_type_id, ref_name)
        }
        .or_else(|| self.resolve_import_target(ref_name))
        .or_else(|| self.resolve_in_library_packages(ref_name))
    }

    /// Search through supertype chains to find a feature by name.
    ///
    /// This is used when the normal scope table has excluded a name due to redefinition.
    fn search_supertypes_for_feature(
        &mut self,
        type_id: &ElementId,
        name: &str,
    ) -> Option<ElementId> {
        let mut visited = FxHashSet::default();
        self.search_supertypes_recursive(type_id, name, &mut visited)
    }

    fn search_supertypes_recursive(
        &mut self,
        type_id: &ElementId,
        name: &str,
        visited: &mut FxHashSet<ElementId>,
    ) -> Option<ElementId> {
        if visited.contains(type_id) {
            return None;
        }
        visited.insert(type_id.clone());

        // Determine this type's direct supertypes.
        //
        // Fast path: the pre-built `InheritanceIndex` already maps every type to
        // its resolved supertypes using the per-kind property pairs
        // (Subclassification -> superclassifier, FeatureTyping -> type, Subsetting
        // -> subsettedFeature, Redefinition -> redefinedFeature, plain
        // Specialization -> general). It is built once over BOTH graphs, so it
        // already follows supertype chains that cross from a file type into the
        // standard-library (fallback) graph — e.g. a `.sysml` definition `:> Array`
        // where `Array :> OrderedCollection :> Collection` live in
        // `Collections.kerml`. Using the index here avoids re-scanning owned
        // members and re-resolving names per call (which previously re-walked the
        // entire combined library subtree for every redefinition reference).
        self.ensure_inheritance_index();
        let indexed_supertypes: Vec<ElementId> = self
            .inheritance_index
            .as_ref()
            .map(|idx| idx.supertypes(type_id).to_vec())
            .unwrap_or_default();

        // Fallback: types with no index entry (e.g. unresolved specializations not
        // yet filled in by pass 1) are handled by scanning owned members and
        // resolving the target on the fly via the same per-kind property mapping.
        let target_ids: Vec<ElementId> = if indexed_supertypes.is_empty() {
            let type_rels: Vec<crate::Element> = self
                .owned_members_combined(type_id)
                .filter(|e| {
                    e.kind == ElementKind::Specialization
                        || e.kind.is_subtype_of(ElementKind::Specialization)
                        || e.kind == ElementKind::FeatureTyping
                        || e.kind.is_subtype_of(ElementKind::FeatureTyping)
                })
                .cloned()
                .collect();
            type_rels
                .iter()
                .filter_map(|rel| self.resolve_supertype_target(type_id, rel))
                .collect()
        } else {
            indexed_supertypes
        };

        for tid in target_ids {
            {
                // Look for the feature in the target type's owned members
                for membership in self.memberships_combined(&tid) {
                    if let Some(view) = MembershipView::try_from_element(membership) {
                        if let Some(member_id) = view.member_element() {
                            let member_name =
                                view.member_name().map(|s| s.to_owned()).or_else(|| {
                                    self.lookup_element(member_id).and_then(|e| e.name.clone())
                                });

                            if member_name.as_deref() == Some(name) {
                                return Some(member_id.clone());
                            }
                        }
                    }
                }

                // Recurse into target type's supertypes
                if let Some(found) = self.search_supertypes_recursive(&tid, name, visited) {
                    return Some(found);
                }
            }
        }

        None
    }
}
