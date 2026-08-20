//! B3 steward-required regression fixture — pins `sysml_core::diff::diff_graphs`
//! against the ADR-009 identity contract using REAL parses (not hand-built
//! graphs).
//!
//! Placement (testing-architecture-redesign §3C, steward-ruled): this is an
//! L4 identity gate distinct from spec-tests' `identity_invariants.rs` (it
//! pins the diff CORRELATION contract, not id mint stability), relocated
//! here — `diff_graphs` is sysml-core's own algorithm (lowest reasonable
//! crate). The former `SysmlService` harness was a convenience wrapper; the
//! tests now drive `TreeSitterParser` directly (the parser is already a
//! dev-dependency; canonical-key minting happens during parsing itself).
//!
//! `diff_graphs` (`crates/lang/sysml-core/src/diff.rs`) correlates strictly by
//! §Consequences) documents that deterministic ids are reparse-stable EXCEPT
//! for two edit patterns that regenerate ids for a subtree *by design*:
//!
//! 1. Renaming a containing scope (qualified_name changes for every
//!    descendant).
//! 2. Inserting/removing an anonymous sibling of the same kind
//!    (`sibling_index_among_kind` shifts for later siblings).
//!
//! Those two patterns must surface as `removed` + `added`, never `modified` —
//! `diff_graphs` applies no name-similarity or positional heuristics. An
//! in-place edit that doesn't touch identity-bearing structure (e.g. doc
//! text) DOES correlate as `modified`. This file pins both sides of that
//! contract with real `.sysml` source parsed through `TreeSitterParser`
//! (parse-only, no elaboration).
//!
//! ## Empirical notes
//!
//! - `doc /* ... */` (`ElementKind::Documentation`) gets its `body` prop set
//!   directly by the AST builder at parse time
//!   (`sysml-parser-incremental/src/ast_builder/imports.rs::process_comment`)
//!   — no elaboration pass is needed for the doc-edit-only case to carry a
//!   `PropChanged { key: "body", .. }` delta. Verified empirically below.
//! - The parse-only graph is sufficient here (no resolve/elaborate):
//!   canonical-key minting happens during parsing itself.

use sysml_core::diff::{diff_graphs, FieldDelta};
use sysml_core::{Element, ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

/// Parse `source` through a fresh, independent `TreeSitterParser` and return
/// its parse-only graph. Two calls with the same `uri` mirror two
/// independent parses of the same file.
fn parse_graph(uri: &str, source: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let file = SysmlFile::new(uri.to_string(), source.to_string());
    let result = parser.parse(std::slice::from_ref(&file));
    assert!(
        !result.graph.elements.is_empty(),
        "{uri}: parsed to an empty graph (hard parse failure)"
    );
    result.graph
}

/// Find the single element of `kind` whose declared `name` matches. Panics
/// (with a helpful message) if zero or more than one match — tests must stay
/// unambiguous.
fn find_named<'g>(graph: &'g ModelGraph, kind: ElementKind, name: &str) -> &'g Element {
    let mut matches = graph
        .elements
        .values()
        .filter(|e| e.kind == kind && e.name.as_deref() == Some(name));
    let found = matches
        .next()
        .unwrap_or_else(|| panic!("no {kind:?} named {name:?} found"));
    assert!(
        matches.next().is_none(),
        "expected exactly one {kind:?} named {name:?}"
    );
    found
}

/// Find the single element of `kind` (anonymous — no name) whose `body` prop
/// equals `body`. Used to identify anonymous `Documentation` elements by
/// content in the sibling-insert test, where names don't exist.
fn find_by_body<'g>(graph: &'g ModelGraph, kind: ElementKind, body: &str) -> &'g Element {
    let mut matches = graph.elements.values().filter(|e| {
        e.kind == kind
            && e.get_prop("body")
                .and_then(|v| v.as_str())
                .map(|b| b == body)
                .unwrap_or(false)
    });
    let found = matches
        .next()
        .unwrap_or_else(|| panic!("no {kind:?} with body {body:?} found"));
    assert!(
        matches.next().is_none(),
        "expected exactly one {kind:?} with body {body:?}"
    );
    found
}

// ---------------------------------------------------------------------------
// 1. In-place edit: doc-comment body text changes, nothing else. R9's common
//    case — MUST correlate as `modified`, not `removed`+`added`.
// ---------------------------------------------------------------------------

#[test]
fn doc_edit_only_is_modified() {
    let source_a = r#"
package CabinReqs {
    part def Frame;

    requirement def <'REQ-1'> CabinTempReq {
        subject veh : Frame;
        part chassis : Frame;
        part shell : Frame;
        doc /* Cabin temperature shall not exceed 40 degrees C. */
    }
}
"#;
    // Only the doc body text changes.
    let source_b = r#"
package CabinReqs {
    part def Frame;

    requirement def <'REQ-1'> CabinTempReq {
        subject veh : Frame;
        part chassis : Frame;
        part shell : Frame;
        doc /* Cabin temperature shall not exceed 25 degrees C. */
    }
}
"#;

    let old = parse_graph("doc_edit.sysml", source_a);
    let new = parse_graph("doc_edit.sysml", source_b);

    // Sanity: the doc element is present and unambiguous in both graphs.
    let old_doc = find_by_body(
        &old,
        ElementKind::Documentation,
        "Cabin temperature shall not exceed 40 degrees C.",
    );
    let new_doc = find_by_body(
        &new,
        ElementKind::Documentation,
        "Cabin temperature shall not exceed 25 degrees C.",
    );
    assert_eq!(
        old_doc.id, new_doc.id,
        "an in-place doc-text edit must NOT regenerate the Documentation element's id \
         (sibling_index/qualified_name are structural, not content-derived)"
    );

    let diff = diff_graphs(&old, &new);

    assert!(
        diff.added.is_empty(),
        "doc-text-only edit must not add any elements, got: {:?}",
        diff.added
    );
    assert!(
        diff.removed.is_empty(),
        "doc-text-only edit must not remove any elements, got: {:?}",
        diff.removed
    );

    // Exactly the Documentation element (and nothing else) is modified.
    assert_eq!(
        diff.modified.len(),
        1,
        "expected exactly one modified element (the Documentation body edit), got: {:?}",
        diff.modified
    );
    let m = &diff.modified[0];
    assert_eq!(m.id, old_doc.id);
    assert_eq!(m.kind, ElementKind::Documentation);
    assert!(
        matches!(
            &m.changed_fields[..],
            [FieldDelta::PropChanged { key, .. }] if key == "body"
        ),
        "expected exactly one PropChanged{{key: \"body\"}} delta, got: {:?}",
        m.changed_fields
    );
}

// ---------------------------------------------------------------------------
// 2. Scope rename: the CONTAINING package is renamed. ADR-009's documented
//    consequence — the requirement and its descendants regenerate ids and
//    surface as removed+added, NEVER modified. No name-similarity heuristic
//    may paper over this.
// ---------------------------------------------------------------------------

#[test]
fn scope_rename_is_remove_add() {
    // `part <name>;` without a type errors inside a requirement body (a
    // known tree-sitter grammar gap for untyped usages there — see
    // `sysml-parser-incremental`'s `grammar-gaps-inventory.md`), and `frame`
    // is itself a contextual keyword inside `requirement_body` (`frame
    // concern`). Both are worked around here with a typed part and
    // non-keyword names — irrelevant to what this test pins.
    let source_a = r#"
package CabinReqsA {
    part def Frame;
    requirement def <'REQ-1'> CabinTempReq {
        subject veh : Frame;
        part chassis : Frame;
        part shell : Frame;
        doc /* Cabin temperature shall not exceed 40 degrees C. */
    }
}
"#;
    // ONLY the containing package's name changes (A -> B); everything nested
    // is byte-identical.
    let source_b = r#"
package CabinReqsB {
    part def Frame;
    requirement def <'REQ-1'> CabinTempReq {
        subject veh : Frame;
        part chassis : Frame;
        part shell : Frame;
        doc /* Cabin temperature shall not exceed 40 degrees C. */
    }
}
"#;

    let old = parse_graph("scope_rename.sysml", source_a);
    let new = parse_graph("scope_rename.sysml", source_b);

    let old_pkg = find_named(&old, ElementKind::Package, "CabinReqsA");
    let new_pkg = find_named(&new, ElementKind::Package, "CabinReqsB");
    let old_req = find_named(&old, ElementKind::RequirementDefinition, "CabinTempReq");
    let new_req = find_named(&new, ElementKind::RequirementDefinition, "CabinTempReq");
    let old_chassis = find_named(&old, ElementKind::PartUsage, "chassis");
    let new_chassis = find_named(&new, ElementKind::PartUsage, "chassis");
    let old_shell = find_named(&old, ElementKind::PartUsage, "shell");
    let new_shell = find_named(&new, ElementKind::PartUsage, "shell");
    let old_doc = find_by_body(
        &old,
        ElementKind::Documentation,
        "Cabin temperature shall not exceed 40 degrees C.",
    );
    let new_doc = find_by_body(
        &new,
        ElementKind::Documentation,
        "Cabin temperature shall not exceed 40 degrees C.",
    );

    // The package itself renamed (different name -> different canonical
    // key), and every descendant's canonical key embeds the parent's key, so
    // all of them regenerate too.
    assert_ne!(old_pkg.id, new_pkg.id);
    assert_ne!(old_req.id, new_req.id);
    assert_ne!(old_chassis.id, new_chassis.id);
    assert_ne!(old_shell.id, new_shell.id);
    assert_ne!(old_doc.id, new_doc.id);

    let diff = diff_graphs(&old, &new);

    for old_id in [
        &old_pkg.id,
        &old_req.id,
        &old_chassis.id,
        &old_shell.id,
        &old_doc.id,
    ] {
        assert!(
            diff.removed.contains(old_id),
            "expected {old_id:?} in diff.removed (scope rename regenerates the whole subtree)"
        );
    }
    for new_id in [
        &new_pkg.id,
        &new_req.id,
        &new_chassis.id,
        &new_shell.id,
        &new_doc.id,
    ] {
        assert!(
            diff.added.contains(new_id),
            "expected {new_id:?} in diff.added (scope rename regenerates the whole subtree)"
        );
    }

    // The pin that matters most: nothing in `modified` papers over the
    // rename with a Name-similarity match. Since every element in this
    // fixture lives inside the renamed package, the whole graph should have
    // regenerated — `modified` should be empty, and in particular must
    // contain no Name delta for a rename that diff_graphs is supposed to
    // report as remove+add.
    assert!(
        diff.modified.is_empty(),
        "scope rename must not correlate any element as modified, got: {:?}",
        diff.modified
    );
    assert!(
        !diff
            .modified
            .iter()
            .any(|m| m.changed_fields.iter().any(|f| matches!(f, FieldDelta::Name { .. }))),
        "diff_graphs must never paper over a scope rename with a Name-delta correlation"
    );
}

// ---------------------------------------------------------------------------
// 3. Anonymous sibling insert: a new anonymous doc comment is inserted
//    BEFORE an existing anonymous doc comment of the same kind at the same
//    level. ADR-009 documents that this shifts `sibling_index_among_kind`
//    for the later sibling, regenerating its id — and `diff.rs`'s module
//    docs claim that shows up as `removed` + `added`, "never modified".
//
// ## What actually happens (empirically observed, not what was expected)
//
// The anonymous canonical key is `parent_key/Documentation[sibling_index]` —
// a pure function of STRUCTURAL POSITION, not content. So inserting a new
// anonymous doc at index 1 doesn't mint a fresh id for "the inserted one"
// while the old index-1 element gets a fresh id too: instead, the id that
// used to name "second note" (index 1) now names whatever CONTENT sits at
// index 1 after the edit — the newly-inserted doc. The doc that used to be
// second `slides` to index 2 and mints a genuinely new id there.
//
// `diff_graphs` correlates strictly by id, so it sees: same id, different
// `body` prop -> `PropChanged`. That is a real, if misleading, `modified`
// entry — not the `removed`+`added` pair the module docs describe. Only the
// TAIL of the shifted run (the last element, which has no successor to
// inherit an id from) shows up as a genuine `added` with no matching
// `removed` anywhere (total Documentation count at this level grew by one,
// so nothing actually disappears).
//
// This is the honest, reproduced behaviour for a 2-sibling insert-in-the-
// middle case; it does not match the "removed + added" phrasing in
// `diff.rs`'s doc comment for this specific shape (an id-slot get reused by
// different content rather than the shifted element's old id vanishing).
// Pinned as observed rather than forced into the documented shape.
// ---------------------------------------------------------------------------

#[test]
fn sibling_insert_shifts_anonymous_ids() {
    let source_a = r#"
package Notes {
    doc /* first note */
    doc /* second note */
}
"#;
    // Insert a new anonymous doc comment BETWEEN the two existing ones.
    let source_b = r#"
package Notes {
    doc /* first note */
    doc /* inserted note */
    doc /* second note */
}
"#;

    let old = parse_graph("sibling_insert.sysml", source_a);
    let new = parse_graph("sibling_insert.sysml", source_b);

    let old_pkg = find_named(&old, ElementKind::Package, "Notes");
    let new_pkg = find_named(&new, ElementKind::Package, "Notes");
    // The package itself is untouched — no rename, no reordering of a
    // different-kind sibling above it.
    assert_eq!(old_pkg.id, new_pkg.id);

    let old_first = find_by_body(&old, ElementKind::Documentation, "first note");
    let old_second = find_by_body(&old, ElementKind::Documentation, "second note");
    let new_first = find_by_body(&new, ElementKind::Documentation, "first note");
    let new_inserted = find_by_body(&new, ElementKind::Documentation, "inserted note");
    let new_second = find_by_body(&new, ElementKind::Documentation, "second note");

    // The untouched leading sibling (index 0) is unaffected by an insert
    // after it — same id.
    assert_eq!(
        old_first.id, new_first.id,
        "the doc before the insertion point must keep its id"
    );

    // The id that used to belong to sibling_index 1 ("second note") is
    // REUSED by the new occupant of that slot ("inserted note") — the
    // canonical key is positional, not content-derived. This is the crux of
    // the empirical finding documented above.
    assert_eq!(
        old_second.id, new_inserted.id,
        "sibling_index 1's id must be reused by whatever content now occupies that slot \
         (this is what 'sibling_index shifts' cashes out to in practice)"
    );

    // The old "second note" content survives, but slides to sibling_index 2
    // and mints a genuinely fresh id there.
    assert_ne!(
        old_second.id, new_second.id,
        "the shifted doc's content must end up under a NEW id at its new slot"
    );

    let diff = diff_graphs(&old, &new);

    // Nothing is removed: the id-slot at index 1 is reoccupied (not freed),
    // and every other slot is untouched or newly created.
    assert!(
        diff.removed.is_empty(),
        "no Documentation id actually disappears in a pure insert, got: {:?}",
        diff.removed
    );

    // The tail of the shift is a genuine `added` id — no predecessor 'gave
    // it up', it's brand new.
    assert!(
        diff.added.contains(&new_second.id),
        "expected the shifted-to-index-2 doc's new id in diff.added, got: {:?}",
        diff.added
    );

    // The reused slot shows up as `modified` — same id, `body` prop changed
    // from the old occupant's text to the new occupant's text. This is the
    // "false modified" the empirical note above describes: diff_graphs
    // cannot distinguish "this element's text was edited" from "a
    // structurally-unrelated element now occupies this id-slot" because
    // both look identical at the (id, changed_fields) level it operates on.
    assert_eq!(
        diff.modified.len(),
        1,
        "expected exactly one modified entry (the reused index-1 slot), got: {:?}",
        diff.modified
    );
    let m = &diff.modified[0];
    assert_eq!(m.id, old_second.id);
    assert_eq!(m.kind, ElementKind::Documentation);
    assert!(
        matches!(
            &m.changed_fields[..],
            [FieldDelta::PropChanged { key, from, to }]
                if key == "body"
                    && from.as_str() == Some("second note")
                    && to.as_str() == Some("inserted note")
        ),
        "expected body PropChanged from 'second note' to 'inserted note', got: {:?}",
        m.changed_fields
    );
}
