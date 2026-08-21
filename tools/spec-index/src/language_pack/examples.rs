//! Example records. Examples are separate,
//! schema-checked records referenced by a card; the same bytes are exported and
//! (in a later slice) executed. Negative examples are authored/mutated from
//! purpose-built positives, never from confidential source.

use serde::Serialize;

/// Provenance for an example.
#[derive(Debug, Clone, Serialize)]
pub struct ExampleProvenance {
    pub origin: String,       // synthetic | purpose-built-public | official-corpus
    pub license_mode: String, // public-safe | citation-only | official-corpus-approved
    pub content_hash: String,
}

/// Declared per-phase expectations for a positive/composed example.
#[derive(Debug, Clone, Serialize)]
pub struct Expected {
    pub syntax_errors: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub element_kinds: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<String>,
    pub resolution_errors: u32,
    pub semantic_diagnostics: Vec<String>,
    pub runtime: String, // optional | required | none
}

/// The intended single fault of a negative example.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectedFailure {
    pub phase: String,          // parse | lower | resolve | elaborate | validate | runtime
    pub mutation_class: String, // keyword-shape | cardinality | reference | typing | relationship | runtime-precondition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
}

/// One named fragment of a composed multi-file example.
#[derive(Debug, Clone, Serialize)]
pub struct ComposedFile {
    pub name: String,
    pub role: String, // primary | supporting | library-stub
    pub source: String,
}

/// One example record (matches `example.schema.json`).
#[derive(Debug, Clone, Serialize)]
pub struct Example {
    pub id: String,
    pub card_id: String,
    pub kind: String, // positive | negative | composed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<ComposedFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Expected>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_failure: Option<ExpectedFailure>,
    pub provenance: ExampleProvenance,
}

impl Example {
    /// A single-file positive example (smallest standalone snippet).
    pub fn positive(id: &str, card_id: &str, source: &str, expected: Expected) -> Self {
        Example {
            id: id.to_owned(),
            card_id: card_id.to_owned(),
            kind: "positive".to_owned(),
            source: Some(source.to_owned()),
            files: None,
            expected: Some(expected),
            expected_failure: None,
            provenance: Self::provenance(source.as_bytes()),
        }
    }

    /// A single-file negative example (one intended fault).
    pub fn negative(id: &str, card_id: &str, source: &str, failure: ExpectedFailure) -> Self {
        Example {
            id: id.to_owned(),
            card_id: card_id.to_owned(),
            kind: "negative".to_owned(),
            source: Some(source.to_owned()),
            files: None,
            expected: None,
            expected_failure: Some(failure),
            provenance: Self::provenance(source.as_bytes()),
        }
    }

    /// A composed multi-file positive example (>=2 fragments).
    pub fn composed(id: &str, card_id: &str, files: Vec<ComposedFile>, expected: Expected) -> Self {
        // Content hash spans every fragment in order, so a change to any file
        // changes the hash.
        let mut acc = Vec::new();
        for f in &files {
            acc.extend_from_slice(f.name.as_bytes());
            acc.push(0);
            acc.extend_from_slice(f.source.as_bytes());
            acc.push(b'\n');
        }
        Example {
            id: id.to_owned(),
            card_id: card_id.to_owned(),
            kind: "composed".to_owned(),
            source: None,
            files: Some(files),
            expected: Some(expected),
            expected_failure: None,
            provenance: Self::provenance(&acc),
        }
    }

    fn provenance(bytes: &[u8]) -> ExampleProvenance {
        ExampleProvenance {
            origin: "synthetic".to_owned(),
            license_mode: "public-safe".to_owned(),
            content_hash: crate::sha256_hex(bytes),
        }
    }
}
