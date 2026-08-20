//! File exports: tracked query for extracting public definitions.
//!
//! This is a lightweight query that extracts the "public API" of a file --
//! the top-level named elements that can be referenced from other files.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::ElementKind;
use sysml_id::ElementId;

use crate::parse;
use crate::source::SourceFile;
use crate::Db;

/// A single exported definition from a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportedDef {
    pub name: String,
    pub kind: ElementKind,
    pub element_id: ElementId,
}

/// All exports from a single file.
#[derive(Clone, Debug)]
pub struct FileExports(Arc<Vec<ExportedDef>>);

impl FileExports {
    fn new(defs: Vec<ExportedDef>) -> Self {
        Self(Arc::new(defs))
    }

    pub fn defs(&self) -> &[ExportedDef] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

salsa_arc_wrapper!(identity, FileExports, Vec<ExportedDef>);

/// Extract public definitions from a file.
///
/// Returns the top-level named elements (packages, part defs, etc.)
/// that can be referenced from other files.
///
/// Depends on: `parse_file()` (Layer 1)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_exports(db: &dyn Db, source_file: SourceFile) -> FileExports {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting exports extraction"
    );
    let parsed = parse::parse_file(db, source_file);
    let graph = parsed.graph();

    let mut exports = Vec::new();
    // Root elements (no owner) with names are the file's public API
    for element in graph.elements.values() {
        if element.owner.is_none() {
            if let Some(name) = &element.name {
                exports.push(ExportedDef {
                    name: name.clone(),
                    kind: element.kind.clone(),
                    element_id: element.id.clone(),
                });
            }
        }
    }
    exports.sort_by(|a, b| a.name.cmp(&b.name));
    FileExports::new(exports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;
    use crate::RootDatabase;

    #[test]
    fn exports_package_defs() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Foo {} package Bar {}".to_string(),
        );
        let exports = file_exports(&db, sf);
        assert!(!exports.is_empty());
        // Should find at least 2 top-level packages
        let names: Vec<_> = exports.defs().iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"Foo"),
            "Should export Foo, got: {:?}",
            names
        );
        assert!(
            names.contains(&"Bar"),
            "Should export Bar, got: {:?}",
            names
        );
    }

    #[test]
    fn exports_empty_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let exports = file_exports(&db, sf);
        assert!(exports.is_empty());
        assert_eq!(exports.len(), 0);
    }

    #[test]
    fn exports_sorted_by_name() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Zebra {} package Alpha {}".to_string(),
        );
        let exports = file_exports(&db, sf);
        let names: Vec<_> = exports.defs().iter().map(|d| d.name.as_str()).collect();
        // Should be sorted alphabetically
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "Exports should be sorted by name");
    }
}
