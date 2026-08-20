use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Project metadata (`.meta.json`).
///
/// Contains the symbol index (mapping exported names to source files),
/// creation timestamp, metamodel IRI, and optional file checksums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMeta {
    /// Map of exported symbol names to their source file paths.
    pub index: HashMap<String, String>,

    /// ISO 8601 creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,

    /// Metamodel IRI (e.g. `"https://www.omg.org/spec/SysML/20250201"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metamodel: Option<String>,

    /// Per-file checksums keyed by relative path.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub checksum: HashMap<String, FileChecksum>,
}

/// A checksum entry for a single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChecksum {
    /// Hex-encoded digest value.
    pub value: String,
    /// Algorithm name (e.g. `"SHA256"`).
    pub algorithm: String,
}

impl ProjectMeta {
    /// Parse a `ProjectMeta` from a JSON string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> crate::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Read a `ProjectMeta` from a `.meta.json` file.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    /// Build a [`SymbolIndex`] from this metadata.
    pub fn symbol_index(&self, project_name: &str) -> SymbolIndex {
        let mut index = SymbolIndex::new();
        for (symbol, file) in &self.index {
            index.insert(SymbolEntry {
                symbol: symbol.clone(),
                file: file.clone(),
                project: project_name.to_owned(),
            });
        }
        index
    }
}

/// An entry in the cross-project symbol lookup table.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEntry {
    /// Exported symbol name (e.g. `"Parts"`, `"ScalarValues"`).
    pub symbol: String,
    /// Source file that defines this symbol (e.g. `"Parts.sysml"`).
    pub file: String,
    /// Project that owns this symbol.
    pub project: String,
}

/// Cross-project symbol lookup table.
///
/// Maps symbol names to their defining project and file. Detects conflicts
/// when two projects export the same symbol name.
#[derive(Debug, Clone, Default)]
pub struct SymbolIndex {
    entries: Vec<SymbolEntry>,
    by_name: HashMap<String, Vec<usize>>,
}

impl SymbolIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a symbol entry.
    pub fn insert(&mut self, entry: SymbolEntry) {
        let idx = self.entries.len();
        self.by_name
            .entry(entry.symbol.clone())
            .or_default()
            .push(idx);
        self.entries.push(entry);
    }

    /// Look up all entries for a given symbol name.
    #[allow(clippy::indexing_slicing)] // Indices are maintained by insert(); always valid
    pub fn lookup(&self, name: &str) -> Vec<&SymbolEntry> {
        self.by_name
            .get(name)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    /// Check for symbol conflicts (same name in different projects).
    #[allow(clippy::indexing_slicing)] // Indices are maintained by insert(); always valid, len > 1 guarantees [0] and [1]
    pub fn conflicts(&self) -> Vec<crate::Error> {
        let mut errors = Vec::new();
        for (name, indices) in &self.by_name {
            if indices.len() > 1 {
                let first = &self.entries[indices[0]];
                let second = &self.entries[indices[1]];
                if first.project != second.project {
                    errors.push(crate::Error::SymbolConflict {
                        symbol: name.clone(),
                        first: first.project.clone(),
                        second: second.project.clone(),
                    });
                }
            }
        }
        errors
    }

    /// Total number of symbol entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries.
    pub fn iter(&self) -> impl Iterator<Item = &SymbolEntry> {
        self.entries.iter()
    }

    /// Merge another index into this one.
    pub fn merge(&mut self, other: &SymbolIndex) {
        for entry in &other.entries {
            self.insert(entry.clone());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_meta() {
        let meta = ProjectMeta {
            index: [("Parts".to_string(), "Parts.sysml".to_string())]
                .into_iter()
                .collect(),
            created: Some("2025-03-13T00:00:00Z".to_string()),
            metamodel: Some("https://www.omg.org/spec/SysML/20250201".to_string()),
            checksum: [(
                "Parts.sysml".to_string(),
                FileChecksum {
                    value: "abc123".to_string(),
                    algorithm: "SHA256".to_string(),
                },
            )]
            .into_iter()
            .collect(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: ProjectMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, parsed);
    }

    #[test]
    fn parse_meta_without_checksum() {
        let json = r#"{"index":{"Base":"Base.kerml"},"created":"2025-03-13T00:00:00Z","metamodel":"https://www.omg.org/spec/KerML/20250201"}"#;
        let meta = ProjectMeta::from_str(json).unwrap();
        assert_eq!(meta.index.len(), 1);
        assert!(meta.checksum.is_empty());
    }

    #[test]
    fn symbol_index_lookup() {
        let mut index = SymbolIndex::new();
        index.insert(SymbolEntry {
            symbol: "Parts".to_string(),
            file: "Parts.sysml".to_string(),
            project: "systems-library".to_string(),
        });
        index.insert(SymbolEntry {
            symbol: "Base".to_string(),
            file: "Base.kerml".to_string(),
            project: "semantic-library".to_string(),
        });

        assert_eq!(index.len(), 2);
        let results = index.lookup("Parts");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].project, "systems-library");
        assert!(index.lookup("Missing").is_empty());
    }

    #[test]
    fn symbol_conflict_detection() {
        let mut index = SymbolIndex::new();
        index.insert(SymbolEntry {
            symbol: "Foo".to_string(),
            file: "Foo.sysml".to_string(),
            project: "project-a".to_string(),
        });
        index.insert(SymbolEntry {
            symbol: "Foo".to_string(),
            file: "Foo.sysml".to_string(),
            project: "project-b".to_string(),
        });

        let conflicts = index.conflicts();
        assert_eq!(conflicts.len(), 1);
    }
}
