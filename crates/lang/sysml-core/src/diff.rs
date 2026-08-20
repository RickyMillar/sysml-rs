//! Element-level diff between two [`ModelGraph`]s.
//!
//! The primitive behind snapshot comparison (`sysml.store.diff`), baseline
//! suspect detection (requirements workbench R9), and future model-diff
//! surfaces. One shape for every consumer — see the B3 steward ruling in
//!
//! ## Identity contract (ADR-009)
//!
//! Correlation is **strictly by [`ElementId`]**. Deterministic ids are
//! reparse-stable (gated by `identity_invariants`), so an in-place edit
//! (doc text, attribute value, link change) correlates as `modified`. Two edit
//! patterns are hostile to id continuity *by design* (ADR-009 §Consequences),
//! with distinct observable shapes (pinned by `diff_identity_baseline` in
//! sysml-spec-tests):
//!
//! - **Renaming a containing scope** regenerates every descendant's id →
//!   the whole subtree surfaces as `removed` + `added`, never `modified`.
//! - **Inserting/removing an anonymous sibling of the same kind** shifts
//!   positional keys (`parent/Kind[sibling_index]`), which **reuses id
//!   slots**: the diff then reports a `modified` entry whose id is bound to
//!   *different elements* in the two snapshots (e.g. a `PropChanged{body}`
//!   that is really old-note → inserted-note), plus one `added` tail id.
//!   A suspect consumer cannot distinguish this from a genuine edit.
//!
//! No name-similarity or positional heuristics are applied to smooth either
//! case, and none may be added: manufacturing continuity the identity model
//! doesn't have would hide real churn (fail-hard rule). Consumers (suspect
//! flags) must document both behaviours rather than compensate for them.
//!
//! ## What counts as "modified"
//!
//! Field comparison is **span-blind**: `spans` / `name_span` are ignored, so
//! an edit elsewhere in a file never marks untouched elements modified.
//! `qname` is skipped as derived (a qname can only change on this element via
//! `name`/`owner`, and ancestor renames regenerate the id anyway).
//! Relationship participation is compared by the `(kind, source, target)`
//! triple — never by relationship id, because elaboration-minted relationship
//! ids (`Relationship::new`) are deliberately reparse-unstable.

use std::collections::HashSet;

use sysml_id::ElementId;

use crate::element::Element;
use crate::graph::ModelGraph;
use crate::meta::Value;
use crate::relationship::RelationshipKind;
use crate::ElementKind;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A change to one directly-compared field of an element.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "field", rename_all = "snake_case"))]
pub enum FieldDelta {
    /// `kind` changed (same id, different metaclass — rare but honest).
    Kind { from: ElementKind, to: ElementKind },
    /// Declared name changed.
    Name {
        from: Option<String>,
        to: Option<String>,
    },
    /// Owner changed (element re-homed without an id change).
    Owner {
        from: Option<ElementId>,
        to: Option<ElementId>,
    },
    /// A `props` entry was added.
    PropAdded { key: String, to: Value },
    /// A `props` entry was removed.
    PropRemoved { key: String, from: Value },
    /// A `props` entry changed value (e.g. a `Documentation` element's
    /// `body` — the requirement-statement-text case).
    PropChanged { key: String, from: Value, to: Value },
}

/// Which end of the changed relationship this element occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RelDirection {
    /// This element is the relationship's source.
    Outgoing,
    /// This element is the relationship's target.
    Incoming,
}

/// A relationship (identified by its `(kind, source, target)` triple) that
/// exists in exactly one of the two graphs, attributed to a surviving
/// endpoint. A triple whose both endpoints survive is reported on **each**
/// endpoint (once outgoing, once incoming) — a per-element view, so the
/// duplication is intentional.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RelationshipDelta {
    pub kind: RelationshipKind,
    /// The element at the other end of the triple.
    pub other: ElementId,
    pub direction: RelDirection,
    /// `true` if the triple exists only in the *new* graph, `false` if it
    /// exists only in the *old* graph.
    pub added: bool,
}

/// The per-element record inside [`GraphDiff::modified`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElementDiff {
    pub id: ElementId,
    /// Kind in the *new* graph (or the old one for a pure-relationship
    /// delta on an otherwise unchanged element — they agree unless
    /// [`FieldDelta::Kind`] is present).
    pub kind: ElementKind,
    /// Field-level changes; may be empty when only relationships changed.
    pub changed_fields: Vec<FieldDelta>,
    /// Relationship-participation changes; may be empty when only fields
    /// changed.
    pub relationship_deltas: Vec<RelationshipDelta>,
}

/// Element-level difference between two graphs (`a` = old, `b` = new).
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GraphDiff {
    /// Ids present only in the new graph (sorted).
    pub added: Vec<ElementId>,
    /// Ids present only in the old graph (sorted).
    pub removed: Vec<ElementId>,
    /// Ids present in both with a field- or relationship-level delta
    /// (sorted by id).
    pub modified: Vec<ElementDiff>,
}

impl GraphDiff {
    /// True when the two graphs are element-level identical.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// Span-blind field comparison. Returns the deltas between two versions of
/// the same element (same id).
fn field_deltas(old: &Element, new: &Element) -> Vec<FieldDelta> {
    let mut deltas = Vec::new();
    if old.kind != new.kind {
        deltas.push(FieldDelta::Kind {
            from: old.kind.clone(),
            to: new.kind.clone(),
        });
    }
    if old.name != new.name {
        deltas.push(FieldDelta::Name {
            from: old.name.clone(),
            to: new.name.clone(),
        });
    }
    if old.owner != new.owner {
        deltas.push(FieldDelta::Owner {
            from: old.owner.clone(),
            to: new.owner.clone(),
        });
    }
    // Props: key-level comparison (BTreeMap iteration is ordered, output is
    // deterministic).
    for (key, old_val) in &old.props {
        match new.props.get(key) {
            None => deltas.push(FieldDelta::PropRemoved {
                key: key.to_string(),
                from: old_val.clone(),
            }),
            Some(new_val) if new_val != old_val => deltas.push(FieldDelta::PropChanged {
                key: key.to_string(),
                from: old_val.clone(),
                to: new_val.clone(),
            }),
            Some(_) => {}
        }
    }
    for (key, new_val) in &new.props {
        if !old.props.contains_key(key) {
            deltas.push(FieldDelta::PropAdded {
                key: key.to_string(),
                to: new_val.clone(),
            });
        }
    }
    deltas
}

type RelTriple = (RelationshipKind, ElementId, ElementId);

fn triple_set(graph: &ModelGraph) -> HashSet<RelTriple> {
    graph
        .relationships
        .values()
        .map(|r| (r.kind.clone(), r.source.clone(), r.target.clone()))
        .collect()
}

/// Compute the element-level diff between `old` and `new`.
///
/// O(|elements| + |relationships|) over both graphs; output vectors are
/// sorted by id for deterministic wire output.
pub fn diff_graphs(old: &ModelGraph, new: &ModelGraph) -> GraphDiff {
    let mut added: Vec<ElementId> = new
        .elements
        .keys()
        .filter(|id| !old.elements.contains_key(*id))
        .cloned()
        .collect();
    let mut removed: Vec<ElementId> = old
        .elements
        .keys()
        .filter(|id| !new.elements.contains_key(*id))
        .cloned()
        .collect();

    // Field-level deltas for surviving ids.
    let mut by_id: std::collections::BTreeMap<ElementId, ElementDiff> =
        std::collections::BTreeMap::new();
    for (id, old_el) in &old.elements {
        let Some(new_el) = new.elements.get(id) else {
            continue;
        };
        let fields = field_deltas(old_el, new_el);
        if !fields.is_empty() {
            by_id.insert(
                id.clone(),
                ElementDiff {
                    id: id.clone(),
                    kind: new_el.kind.clone(),
                    changed_fields: fields,
                    relationship_deltas: Vec::new(),
                },
            );
        }
    }

    // Relationship deltas by (kind, source, target) triple, attributed to
    // surviving endpoints only (added/removed elements imply their edges).
    let old_triples = triple_set(old);
    let new_triples = triple_set(new);
    let mut attribute = |triple: &RelTriple, is_added: bool| {
        let (kind, source, target) = triple;
        let graphs_have = |id: &ElementId| {
            old.elements.contains_key(id) && new.elements.contains_key(id)
        };
        for (this, other, direction) in [
            (source, target, RelDirection::Outgoing),
            (target, source, RelDirection::Incoming),
        ] {
            if !graphs_have(this) {
                continue;
            }
            let entry = by_id.entry(this.clone()).or_insert_with(|| ElementDiff {
                id: this.clone(),
                // Surviving element: kind from the new graph.
                kind: new.elements[this].kind.clone(),
                changed_fields: Vec::new(),
                relationship_deltas: Vec::new(),
            });
            entry.relationship_deltas.push(RelationshipDelta {
                kind: kind.clone(),
                other: other.clone(),
                direction,
                added: is_added,
            });
        }
    };
    for triple in new_triples.difference(&old_triples) {
        attribute(triple, true);
    }
    for triple in old_triples.difference(&new_triples) {
        attribute(triple, false);
    }

    added.sort();
    removed.sort();
    let mut modified: Vec<ElementDiff> = by_id.into_values().collect();
    for diff in &mut modified {
        diff.relationship_deltas.sort_by(|a, b| {
            (format!("{:?}", a.kind), &a.other, a.added)
                .cmp(&(format!("{:?}", b.kind), &b.other, b.added))
        });
    }
    GraphDiff {
        added,
        removed,
        modified,
    }
}

/// Canonical content digest over exactly the fields [`diff_graphs`]
/// compares: elements as `(id, kind, name, owner, props)` sorted by id,
/// relationships as `(kind, source, target)` triples sorted — span-blind,
/// qname-blind, and blind to relationship ids (which are deliberately
/// reparse-unstable, see the module doc). This yields the invariant
/// `content_digest(a) == content_digest(b)  ⟺  diff_graphs(a, b).is_empty()`,
/// which `sysml.store.save_workspace` relies on for idempotent,
/// content-addressed commit ids: reloading an unchanged workspace mints
/// no new commit.
///
/// Deliberately NOT a hash of [`crate::json::to_json_string`] (that
/// serialization includes spans and unstable relationship ids) and NOT
/// `sysml_query::graph_revision` (ids/counts only — misses in-place prop
/// edits, the canonical suspect-flag case).
#[cfg(feature = "serde")]
impl ModelGraph {
    /// See the free-standing doc above: canonical, diff-equivalent digest.
    pub fn content_digest(&self) -> String {
        content_digest(self)
    }

    /// Content digest of the ownership subtree rooted at `id` — the
    /// element's own content folded with that of its transitive owned
    /// children (`children_of`, recursive), in the SAME hashing family as
    /// [`content_digest`]: SHA-256 over the diff-compared element field set
    /// (`id, kind, name, owner, props`) plus the `(kind, source, target)`
    /// triples of relationships internal to the subtree, all sorted for
    /// determinism. Returns `None` when `id` is not in the graph (honest
    /// "unknown", never a digest of the empty set).
    ///
    /// This is the per-CASE change-detection primitive (P6 of the
    /// test-management model study): a verification execution pins the
    /// digest of the case it ran, and a later read flags "this case changed
    /// since this execution" by comparing against the current subtree
    /// digest. A child-value edit (objective / check / expression) moves
    /// the digest; an edit to an element OUTSIDE the subtree does not —
    /// exactly the blindness [`content_digest`] has (span-blind,
    /// relationship-id-blind), scoped to one ownership subtree. Composes the
    /// existing digest machinery — it introduces no second hash mechanism.
    pub fn subtree_digest(&self, id: &ElementId) -> Option<String> {
        subtree_digest(self, id)
    }
}

/// One element, restricted to the field set [`diff_graphs`] compares.
/// Module-scoped so [`content_digest`] and [`subtree_digest`] hash an
/// element through the identical serialization — there is exactly one
/// definition of "an element's contribution to a content digest".
#[cfg(feature = "serde")]
#[derive(Serialize)]
struct DigestElement<'a> {
    id: &'a ElementId,
    kind: &'a ElementKind,
    name: &'a Option<String>,
    owner: &'a Option<ElementId>,
    props: &'a std::collections::BTreeMap<std::borrow::Cow<'static, str>, Value>,
}

/// Feed one element's diff-compared fields into the running SHA-256, with a
/// trailing separator byte. The single home for how an element contributes
/// to a content digest (shared by the whole-graph and subtree digests).
#[cfg(feature = "serde")]
fn hash_element_into(hasher: &mut sha2::Sha256, el: &Element) {
    use sha2::Digest;
    let record = DigestElement {
        id: &el.id,
        kind: &el.kind,
        name: &el.name,
        owner: &el.owner,
        props: &el.props,
    };
    #[allow(clippy::expect_used)] // Infallible: all fields are serializable
    serde_json::to_writer(&mut HashWriter(hasher), &record)
        .expect("digest record should always be serializable");
    hasher.update([0u8]);
}

/// Feed one relationship triple into the running SHA-256, with a trailing
/// separator byte. Shared by the whole-graph and subtree digests.
#[cfg(feature = "serde")]
fn hash_triple_into(hasher: &mut sha2::Sha256, triple: &RelTriple) {
    use sha2::Digest;
    #[allow(clippy::expect_used)] // Infallible: all fields are serializable
    serde_json::to_writer(&mut HashWriter(hasher), &(&triple.0, &triple.1, &triple.2))
        .expect("digest triple should always be serializable");
    hasher.update([0u8]);
}

/// Deterministic sort key for relationship triples — one ordering rule for
/// every digest path.
#[cfg(feature = "serde")]
fn triple_sort_key(t: &RelTriple) -> (String, &ElementId, &ElementId) {
    (format!("{:?}", t.0), &t.1, &t.2)
}

/// Finalize a SHA-256 hasher to a lowercase hex string.
#[cfg(feature = "serde")]
fn finalize_hex(hasher: sha2::Sha256) -> String {
    use sha2::Digest;
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        #[allow(clippy::expect_used)] // Infallible: writing hex into a String
        write!(out, "{byte:02x}").expect("hex formatting into a String cannot fail");
    }
    out
}

#[cfg(feature = "serde")]
pub fn content_digest(graph: &ModelGraph) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();

    let mut ids: Vec<&ElementId> = graph.elements.keys().collect();
    ids.sort();
    for id in ids {
        let Some(el) = graph.elements.get(id) else {
            continue;
        };
        hash_element_into(&mut hasher, el);
    }

    let mut triples: Vec<RelTriple> = triple_set(graph).into_iter().collect();
    triples.sort_by(|a, b| triple_sort_key(a).cmp(&triple_sort_key(b)));
    for triple in &triples {
        hash_triple_into(&mut hasher, triple);
    }

    finalize_hex(hasher)
}

/// See [`ModelGraph::subtree_digest`].
#[cfg(feature = "serde")]
pub fn subtree_digest(graph: &ModelGraph, id: &ElementId) -> Option<String> {
    use sha2::{Digest, Sha256};

    // Root must exist — otherwise there is no subtree and the honest answer
    // is "unknown" (`None`), never a digest of the empty set.
    graph.get_element(id)?;

    // Collect the ownership subtree: root + transitive owned children. A
    // `BTreeSet` gives the id-sorted deterministic iteration order the
    // digest needs (same ordering rule as `content_digest`) and dedups the
    // walk (single-ownership means no diamonds, but the set keeps it safe).
    let mut subtree: std::collections::BTreeSet<ElementId> = std::collections::BTreeSet::new();
    subtree.insert(id.clone());
    let mut stack = vec![id.clone()];
    while let Some(cur) = stack.pop() {
        for child in graph.children_of(&cur) {
            if subtree.insert(child.id.clone()) {
                stack.push(child.id.clone());
            }
        }
    }

    let mut hasher = Sha256::new();
    for eid in &subtree {
        if let Some(el) = graph.get_element(eid) {
            hash_element_into(&mut hasher, el);
        }
    }

    // Relationships INTERNAL to the subtree (both endpoints owned within
    // it): a rewire among the case's own elements is subtree content; an
    // edge to something outside is not part of this subtree's identity.
    let mut triples: Vec<RelTriple> = triple_set(graph)
        .into_iter()
        .filter(|(_, source, target)| subtree.contains(source) && subtree.contains(target))
        .collect();
    triples.sort_by(|a, b| triple_sort_key(a).cmp(&triple_sort_key(b)));
    for triple in &triples {
        hash_triple_into(&mut hasher, triple);
    }

    Some(finalize_hex(hasher))
}

/// `std::io::Write` adapter feeding bytes straight into a SHA-256 hasher.
#[cfg(feature = "serde")]
struct HashWriter<'a>(&'a mut sha2::Sha256);

#[cfg(feature = "serde")]
impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relationship::Relationship;
    use sysml_span::Span;

    fn id(n: u128) -> ElementId {
        ElementId::from_string(format!("diff-test-{n}"))
    }

    fn element(n: u128, kind: ElementKind, name: &str) -> Element {
        let mut el = Element::new(id(n), kind);
        el.name = Some(name.to_string());
        el
    }

    fn graph(elements: Vec<Element>, rels: Vec<Relationship>) -> ModelGraph {
        let mut g = ModelGraph::new();
        for el in elements {
            g.add_element(el);
        }
        for r in rels {
            g.add_relationship(r);
        }
        g
    }

    /// The content-digest invariant: digest equality ⟺ empty diff.
    /// Specifically: span moves and relationship-id churn (both invisible
    /// to the diff) leave the digest unchanged, while a prop edit — the
    /// canonical suspect case — changes it.
    #[cfg(feature = "serde")]
    #[test]
    fn content_digest_matches_diff_blindness() {
        let mut doc = element(1, ElementKind::Documentation, "d");
        doc.set_prop("body", "trip within 40 ms");
        doc.spans = vec![Span::new("f.sysml", 0, 10)];
        let part = element(2, ElementKind::PartUsage, "p");
        // Same triple, freshly-minted (different) relationship ids — the
        // reload case: elaboration re-mints relationship ids every run.
        let rel_a = Relationship::new(RelationshipKind::Satisfy, id(2), id(1));
        let rel_b = Relationship::new(RelationshipKind::Satisfy, id(2), id(1));
        assert_ne!(rel_a.id, rel_b.id);

        let old = graph(vec![doc.clone(), part.clone()], vec![rel_a]);

        // Reload-shaped copy: span moved, relationship id re-minted.
        let mut moved_doc = doc.clone();
        moved_doc.spans = vec![Span::new("f.sysml", 500, 510)];
        let reloaded = graph(vec![moved_doc, part.clone()], vec![rel_b]);
        assert!(diff_graphs(&old, &reloaded).is_empty());
        assert_eq!(old.content_digest(), reloaded.content_digest());

        // Content edit: digest must move with the diff.
        let mut edited_doc = doc.clone();
        edited_doc.set_prop("body", "trip within 25 ms");
        let edited = graph(
            vec![edited_doc, part],
            vec![Relationship::new(RelationshipKind::Satisfy, id(2), id(1))],
        );
        assert!(!diff_graphs(&old, &edited).is_empty());
        assert_ne!(old.content_digest(), edited.content_digest());
    }

    /// P6 (test-management model study): the subtree digest folds a root
    /// element's content with its transitive owned children. A child-value
    /// edit must move the ROOT's subtree digest; an edit to an element
    /// OUTSIDE the subtree must not.
    #[cfg(feature = "serde")]
    #[test]
    fn subtree_digest_tracks_children_not_outside_elements() {
        // Root case owns one child (an in-case check); a third element sits
        // outside the subtree entirely.
        fn build() -> ModelGraph {
            let case = element(1, ElementKind::VerificationCaseDefinition, "TripTest");
            let mut child = element(2, ElementKind::RequirementUsage, "checkReq");
            child.owner = Some(id(1));
            child.set_prop("body", "trip within 40 ms");
            let outside = element(3, ElementKind::PartUsage, "unrelated");
            graph(vec![case, child, outside], vec![])
        }

        let base = build();
        let root_digest = base.subtree_digest(&id(1)).expect("root is in the graph");

        // Editing the child's content moves the root's subtree digest.
        let mut child_edited = build();
        child_edited
            .get_element_mut(&id(2))
            .unwrap()
            .set_prop("body", "trip within 25 ms");
        assert_ne!(
            root_digest,
            child_edited.subtree_digest(&id(1)).unwrap(),
            "a child edit must move the parent's subtree digest"
        );

        // Editing an element outside the subtree leaves the digest alone.
        let mut outside_edited = build();
        outside_edited
            .get_element_mut(&id(3))
            .unwrap()
            .set_prop("body", "changed");
        assert_eq!(
            root_digest,
            outside_edited.subtree_digest(&id(1)).unwrap(),
            "an edit outside the subtree must not move its digest"
        );

        // The child's OWN subtree digest is a strict subset — editing the
        // outside element never touches it either.
        assert_eq!(
            base.subtree_digest(&id(2)).unwrap(),
            outside_edited.subtree_digest(&id(2)).unwrap()
        );
    }

    /// A relationship rewired BETWEEN two elements inside the subtree moves
    /// the subtree digest (internal endpoints are subtree content); the same
    /// blindness `content_digest` has, scoped.
    #[cfg(feature = "serde")]
    #[test]
    fn subtree_digest_sees_internal_relationship_rewire() {
        use crate::relationship::Relationship;
        fn build(rel_target: u128) -> ModelGraph {
            let root = element(1, ElementKind::PartDefinition, "Root");
            let mut a = element(2, ElementKind::PartUsage, "a");
            a.owner = Some(id(1));
            let mut b = element(3, ElementKind::PartUsage, "b");
            b.owner = Some(id(1));
            let mut rel = Relationship::new(RelationshipKind::Dependency, id(2), id(rel_target));
            rel.id = id(99);
            graph(vec![root, a, b], vec![rel])
        }
        // Both targets (elem-2 self-loop vs elem-3) are inside the subtree.
        assert_ne!(
            build(2).subtree_digest(&id(1)).unwrap(),
            build(3).subtree_digest(&id(1)).unwrap()
        );
    }

    /// An unknown root id yields `None` — honest "unknown", never a digest
    /// of nothing.
    #[cfg(feature = "serde")]
    #[test]
    fn subtree_digest_unknown_root_is_none() {
        let g = graph(vec![element(1, ElementKind::PartDefinition, "P")], vec![]);
        assert!(g.subtree_digest(&id(999)).is_none());
    }

    #[test]
    fn identical_graphs_produce_empty_diff() {
        let make = || {
            graph(
                vec![element(1, ElementKind::RequirementUsage, "req")],
                vec![],
            )
        };
        assert!(diff_graphs(&make(), &make()).is_empty());
    }

    #[test]
    fn added_and_removed_by_id() {
        let old = graph(vec![element(1, ElementKind::PartUsage, "a")], vec![]);
        let new = graph(vec![element(2, ElementKind::PartUsage, "b")], vec![]);
        let d = diff_graphs(&old, &new);
        assert_eq!(d.added, vec![id(2)]);
        assert_eq!(d.removed, vec![id(1)]);
        assert!(d.modified.is_empty());
    }

    #[test]
    fn doc_body_change_is_prop_changed_and_span_changes_are_invisible() {
        let mut old_doc = element(1, ElementKind::Documentation, "d");
        old_doc.set_prop("body", "trip within 40 ms");
        old_doc.spans = vec![Span::new("f.sysml", 0, 10)];
        let mut new_doc = old_doc.clone();
        new_doc.set_prop("body", "trip within 25 ms");
        new_doc.spans = vec![Span::new("f.sysml", 500, 510)]; // moved in the file

        let d = diff_graphs(&graph(vec![old_doc], vec![]), &graph(vec![new_doc], vec![]));
        assert_eq!(d.modified.len(), 1);
        assert_eq!(d.modified[0].id, id(1));
        assert!(matches!(
            &d.modified[0].changed_fields[..],
            [FieldDelta::PropChanged { key, .. }] if key == "body"
        ));

        // Span-only change → empty diff.
        let mut moved = element(2, ElementKind::PartUsage, "p");
        moved.spans = vec![Span::new("f.sysml", 0, 5)];
        let mut moved_b = moved.clone();
        moved_b.spans = vec![Span::new("f.sysml", 100, 105)];
        assert!(diff_graphs(
            &graph(vec![moved], vec![]),
            &graph(vec![moved_b], vec![])
        )
        .is_empty());
    }

    #[test]
    fn relationship_change_by_triple_ignores_relationship_ids() {
        let a = element(1, ElementKind::PartUsage, "part");
        let b = element(2, ElementKind::RequirementUsage, "req");
        // Same triple, freshly minted (different) relationship ids on each side.
        let old = graph(
            vec![a.clone(), b.clone()],
            vec![Relationship::new(RelationshipKind::Satisfy, id(1), id(2))],
        );
        let new = graph(
            vec![a.clone(), b.clone()],
            vec![Relationship::new(RelationshipKind::Satisfy, id(1), id(2))],
        );
        assert!(diff_graphs(&old, &new).is_empty());

        // Dropping the edge reports a delta on BOTH surviving endpoints.
        let new_no_edge = graph(vec![a, b], vec![]);
        let d = diff_graphs(&old, &new_no_edge);
        assert_eq!(d.modified.len(), 2);
        for m in &d.modified {
            assert!(m.changed_fields.is_empty());
            assert_eq!(m.relationship_deltas.len(), 1);
            assert!(!m.relationship_deltas[0].added);
            assert_eq!(m.relationship_deltas[0].kind, RelationshipKind::Satisfy);
        }
        let dirs: Vec<RelDirection> = d
            .modified
            .iter()
            .map(|m| m.relationship_deltas[0].direction)
            .collect();
        assert!(dirs.contains(&RelDirection::Outgoing) && dirs.contains(&RelDirection::Incoming));
    }

    #[test]
    fn edge_to_removed_element_is_not_attributed() {
        let a = element(1, ElementKind::PartUsage, "part");
        let b = element(2, ElementKind::RequirementUsage, "req");
        let old = graph(
            vec![a.clone(), b],
            vec![Relationship::new(RelationshipKind::Satisfy, id(1), id(2))],
        );
        let new = graph(vec![a], vec![]); // b (and its edge) gone
        let d = diff_graphs(&old, &new);
        assert_eq!(d.removed, vec![id(2)]);
        // The surviving endpoint still learns its edge went away…
        assert_eq!(d.modified.len(), 1);
        assert_eq!(d.modified[0].id, id(1));
        // …but nothing is attributed to the removed element.
        assert!(d.modified.iter().all(|m| m.id != id(2)));
    }

    #[test]
    fn name_change_on_same_id_is_modified() {
        let old = graph(vec![element(1, ElementKind::RequirementUsage, "a")], vec![]);
        let new = graph(vec![element(1, ElementKind::RequirementUsage, "b")], vec![]);
        let d = diff_graphs(&old, &new);
        assert_eq!(d.modified.len(), 1);
        assert!(matches!(
            &d.modified[0].changed_fields[..],
            [FieldDelta::Name { .. }]
        ));
    }
}
