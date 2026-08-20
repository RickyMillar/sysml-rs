//! Cached scope information for a namespace.

use rustc_hash::FxHashMap;
use sysml_id::ElementId;

use crate::VisibilityKind;

/// Cached scope information for a namespace.
///
/// This caches the expanded set of visible names in a namespace,
/// including members imported from other namespaces.
#[derive(Debug, Default, Clone)]
pub struct ScopeTable {
    /// Direct owned members: name -> element ID.
    pub(crate) owned: FxHashMap<String, ElementId>,
    /// Short names of owned members: short_name -> element ID.
    pub(crate) owned_short: FxHashMap<String, ElementId>,
    /// Imported members: name -> (element ID, visibility).
    pub(crate) imported: FxHashMap<String, (ElementId, VisibilityKind)>,
    /// Imported members by short name.
    pub(crate) imported_short: FxHashMap<String, (ElementId, VisibilityKind)>,
    /// Names that were brought in by two or more imports resolving to
    /// *distinct* element IDs at the same (imported) precedence tier.
    ///
    /// Populated by [`ScopeTable::add_imported`] as a side effect; the stored
    /// vector accumulates every distinct colliding ID (the first/last-wins
    /// value still lives in `imported`, so resolution behaviour is unchanged).
    /// This is the substrate for `ScopedResolution::Ambiguous` (ADR-016 D5).
    pub(crate) ambiguous_imported: FxHashMap<String, Vec<ElementId>>,
    /// Inherited members (via Specialization): name -> element ID.
    pub(crate) inherited: FxHashMap<String, ElementId>,
    /// Whether this scope table has been fully populated (owned members).
    pub(crate) populated: bool,
    /// Whether inherited members have been populated.
    pub(crate) inherited_populated: bool,
    /// Whether imported members have been populated.
    pub(crate) imported_populated: bool,
    /// Generation counter for cache invalidation.
    /// When the graph is mutated between passes, scope tables with a stale
    /// generation are rebuilt.
    pub(crate) generation: u64,
}

impl ScopeTable {
    /// Create a new empty scope table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an owned member.
    pub fn add_owned(&mut self, name: String, id: ElementId) {
        self.owned.insert(name, id);
    }

    /// Add an owned member by short name.
    pub fn add_owned_short(&mut self, short_name: String, id: ElementId) {
        self.owned_short.insert(short_name, id);
    }

    /// Add an imported member.
    ///
    /// If a different element id is already imported under this name, the name
    /// is a genuine cross-import collision: it is recorded in
    /// `ambiguous_imported` (accumulating every distinct id). The same id
    /// arriving twice (e.g. re-exported through two public imports that resolve
    /// to the same target) is NOT a collision. The stored `imported` value
    /// keeps its existing last-wins behaviour so resolution is unchanged.
    pub fn add_imported(&mut self, name: String, id: ElementId, visibility: VisibilityKind) {
        if let Some((existing_id, _)) = self.imported.get(&name) {
            if *existing_id != id {
                let entry = self
                    .ambiguous_imported
                    .entry(name.clone())
                    .or_insert_with(|| vec![existing_id.clone()]);
                if !entry.contains(&id) {
                    entry.push(id.clone());
                }
            }
        }
        self.imported.insert(name, (id, visibility));
    }

    /// Returns the distinct colliding element ids for an imported name, if the
    /// name was brought in ambiguously (two+ distinct ids across imports).
    pub fn ambiguous_imported(&self, name: &str) -> Option<&[ElementId]> {
        Self::lookup_with_quote_variants(&self.ambiguous_imported, name).map(|v| v.as_slice())
    }

    /// Iterate every ambiguously-imported name and its colliding ids.
    pub fn ambiguous_imported_iter(&self) -> impl Iterator<Item = (&String, &Vec<ElementId>)> {
        self.ambiguous_imported.iter()
    }

    /// Add an imported member by short name.
    pub fn add_imported_short(
        &mut self,
        short_name: String,
        id: ElementId,
        visibility: VisibilityKind,
    ) {
        self.imported_short.insert(short_name, (id, visibility));
    }

    /// Add an inherited member.
    pub fn add_inherited(&mut self, name: String, id: ElementId) {
        self.inherited.insert(name, id);
    }

    /// Try looking up a name in a map, also checking quoted/unquoted variants.
    fn lookup_with_quote_variants<'a, V>(
        map: &'a FxHashMap<String, V>,
        name: &str,
    ) -> Option<&'a V> {
        // Try exact match first
        if let Some(v) = map.get(name) {
            return Some(v);
        }
        // Try with quotes stripped
        let stripped = name.trim_matches('\'');
        if stripped != name {
            if let Some(v) = map.get(stripped) {
                return Some(v);
            }
        }
        // Try with quotes added
        let quoted = format!("'{}'", stripped);
        if quoted != name {
            map.get(&quoted)
        } else {
            None
        }
    }

    /// Look up a name in this scope (owned only).
    pub fn lookup_owned(&self, name: &str) -> Option<&ElementId> {
        Self::lookup_with_quote_variants(&self.owned, name)
            .or_else(|| Self::lookup_with_quote_variants(&self.owned_short, name))
    }

    /// Look up a name in inherited members.
    pub fn lookup_inherited(&self, name: &str) -> Option<&ElementId> {
        Self::lookup_with_quote_variants(&self.inherited, name)
    }

    /// Look up a name in imported members.
    pub fn lookup_imported(&self, name: &str) -> Option<&ElementId> {
        Self::lookup_with_quote_variants(&self.imported, name)
            .or_else(|| Self::lookup_with_quote_variants(&self.imported_short, name))
            .map(|(id, _)| id)
    }

    /// Look up a name with visibility check for imports.
    pub fn lookup_imported_visible(
        &self,
        name: &str,
        check_visibility: bool,
    ) -> Option<&ElementId> {
        let result = Self::lookup_with_quote_variants(&self.imported, name)
            .or_else(|| Self::lookup_with_quote_variants(&self.imported_short, name));
        match result {
            Some((id, visibility)) => {
                if !check_visibility || *visibility == VisibilityKind::Public {
                    Some(id)
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// Mark this scope table as fully populated.
    pub fn set_populated(&mut self) {
        self.populated = true;
    }

    /// Check if this scope table has been fully populated.
    pub fn is_populated(&self) -> bool {
        self.populated
    }

    /// Mark inherited members as populated.
    pub fn set_inherited_populated(&mut self) {
        self.inherited_populated = true;
    }

    /// Check if inherited members have been populated.
    pub fn has_inherited_populated(&self) -> bool {
        self.inherited_populated
    }

    /// Mark imported members as populated.
    pub fn set_imported_populated(&mut self) {
        self.imported_populated = true;
    }

    /// Check if imported members have been populated.
    pub fn has_imported_populated(&self) -> bool {
        self.imported_populated
    }

    /// Get the generation this scope table was built at.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Set the generation for this scope table.
    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Clear the scope table.
    pub fn clear(&mut self) {
        self.owned.clear();
        self.owned_short.clear();
        self.imported.clear();
        self.imported_short.clear();
        self.ambiguous_imported.clear();
        self.inherited.clear();
        self.populated = false;
        self.inherited_populated = false;
        self.imported_populated = false;
    }
}
