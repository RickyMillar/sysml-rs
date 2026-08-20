//! Tracked query for `(text, span)` lookup of an element in a single file.
//!
//! Powers S4 sneak-peeks: given a `(SourceFile, ElementId)`, return the
//! element's declaration span and the source text it covers. The frontend
//! mounts that slice in a read-only Monaco; the diagram, tree, and editor
//! all refer to the same `ElementId`.
//!
//! Single-file scope by design. An `ElementId` can only have source bytes
//! in the file that minted it, so workspace-scoped lookup would just be
//! "find the file then call this query." The service command in
//! `sysml-service` does that resolution and delegates here.
//!
//! Result type `FileSourceSlice` wraps `Arc<FileSourceSliceData>` with
//! pointer-identity equality (`salsa_arc_wrapper!(identity, …)`); salsa
//! returns the same `Arc` on cache hits within a revision.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_id::ElementId;
use sysml_span::Span;

use crate::parse;
use crate::source::SourceFile;
use crate::Db;

/// Salsa-cached `(text, span)` snapshot for a single element.
#[derive(Clone, Debug)]
pub struct FileSourceSlice(Arc<FileSourceSliceData>);

#[derive(Debug)]
struct FileSourceSliceData {
    text: String,
    span: Span,
}

impl FileSourceSlice {
    fn new(text: String, span: Span) -> Self {
        Self(Arc::new(FileSourceSliceData { text, span }))
    }

    /// Source text covered by the element's primary span.
    pub fn text(&self) -> &str {
        &self.0.text
    }

    /// The span the text was sliced from.
    pub fn span(&self) -> &Span {
        &self.0.span
    }
}

salsa_arc_wrapper!(identity, FileSourceSlice, FileSourceSliceData);

/// Return the source slice for `id` in `sf`, if the element lives in this
/// file's parsed graph and carries a usable span.
///
/// Span resolution prefers `Element::spans[0]` (the full declaration); if
/// it's missing, falls back to `name_span`. An out-of-bounds span returns
/// `None` defensively — the parse layer is the source of truth and any
/// drift here is a bug worth observing rather than silently truncating.
///
/// Depends on: `parse_file` (Layer 1) + `source_file.text()` (Layer 0).
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_source_at(db: &dyn Db, sf: SourceFile, id: ElementId) -> Option<FileSourceSlice> {
    let parsed = parse::parse_file(db, sf);
    let element = parsed.graph().elements.get(&id)?;
    let span = element
        .spans
        .first()
        .or(element.name_span.as_ref())?
        .clone();
    let text = sf.text(db);
    let slice = text.get(span.start..span.end)?.to_owned();
    Some(FileSourceSlice::new(slice, span))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    fn db_with_source(text: &str) -> (RootDatabase, SourceFile) {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_owned(), text.to_owned());
        (db, sf)
    }

    #[test]
    fn returns_slice_for_known_element() {
        let src = "package Foo { part def Bar; }";
        let (db, sf) = db_with_source(src);
        let parsed = parse::parse_file(&db, sf);

        // Pick the first element that has a primary span — the parse layer
        // guarantees at least one when the source parses cleanly.
        let element = parsed
            .graph()
            .elements
            .values()
            .find(|e| !e.spans.is_empty())
            .expect("at least one element should carry a span");
        let expected_span = element.spans[0].clone();
        let expected_text = src[expected_span.start..expected_span.end].to_owned();

        let slice =
            file_source_at(&db, sf, element.id.clone()).expect("slice for known element");
        assert_eq!(slice.text(), expected_text.as_str());
        assert_eq!(slice.span(), &expected_span);
    }

    #[test]
    fn returns_none_for_unknown_element() {
        let (db, sf) = db_with_source("package Foo {}");
        let stranger = ElementId::new_v4();
        assert!(file_source_at(&db, sf, stranger).is_none());
    }

    #[test]
    fn cache_hit_returns_same_arc() {
        let (db, sf) = db_with_source("package Foo { part def Bar; }");
        let parsed = parse::parse_file(&db, sf);
        let id = parsed
            .graph()
            .elements
            .values()
            .find(|e| !e.spans.is_empty())
            .expect("element with span")
            .id
            .clone();

        let a = file_source_at(&db, sf, id.clone()).expect("slice");
        let b = file_source_at(&db, sf, id).expect("slice");
        assert!(Arc::ptr_eq(&a.0, &b.0), "salsa should memoize the slice");
    }
}
