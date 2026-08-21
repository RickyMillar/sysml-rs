//! Card assembly. The full field list is normative in
//! `language-card.schema.json`; this mirrors it as a serializable struct so a
//! serialized card is directly schema-checkable.

use serde::Serialize;
use sysml_codegen::IrNode;

use super::metamodel::MetamodelFacet;
use super::support::SupportAxes;

/// Card schema version. Bumped to `1.1` for the additive, optional
/// `metamodel_facet` field: a v1.1 minor bump — every pre-existing
/// field is unchanged and cards without a facet omit it, so `1.1` is a strict
/// superset of `1`. The schema enum accepts both.
pub const SCHEMA_VERSION: &str = "1.1";

/// Grammar citation by rule name.
#[derive(Debug, Clone, Serialize)]
pub struct GrammarRuleRef {
    pub grammar: String, // kerml | sysml | expressions
    pub name: String,
}

/// Spec-clause citation resolved against the derived plaintext heading index.
#[derive(Debug, Clone, Serialize)]
pub struct ClauseRef {
    pub document: String, // SysML | KerML
    pub clause: String,
    pub anchor: String,
    /// Citation precision: `exact` when the
    /// cited clause is itself a heading; `ancestor` when it was resolved to its
    /// deepest existing ancestor heading (the fine sub-clause has no heading in
    /// the derived text). Precision loss must be visible to consumers — the same
    /// idiom as `unknown` != no on the support axes.
    pub resolution: String,
}

/// Example-ID buckets referenced by a card.
#[derive(Debug, Clone, Serialize)]
pub struct ExamplesRef {
    pub positive: Vec<String>,
    pub negative: Vec<String>,
    pub composed: Vec<String>,
}

/// Card provenance. No git commit / wall-clock: content
/// must be regen-diff-stable across commits.
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub spec_drop: String,
    pub source_paths: Vec<String>,
    pub source_hashes: Vec<String>,
    pub generated_by: String,
}

/// One retrieval card (matches `language-card.schema.json`). Field order is the
/// schema's `required` order; serde preserves it for deterministic output.
#[derive(Debug, Clone, Serialize)]
pub struct LanguageCard {
    pub schema_version: String,
    pub id: String,
    pub title: String,
    pub language: String,
    pub category: Vec<String>,
    pub summary: String,
    pub keywords: Vec<String>,
    pub aliases: Vec<String>,
    pub normative_rules: Vec<GrammarRuleRef>,
    pub normative_clauses: Vec<ClauseRef>,
    pub normalized_grammar: Option<IrNode>,
    pub rule_dependencies: Vec<String>,
    pub semantic_types: Vec<String>,
    pub validation_rules: Vec<String>,
    pub examples: ExamplesRef,
    pub support: SupportAxes,
    pub known_gaps: Vec<String>,
    pub related_cards: Vec<String>,
    pub provenance: Provenance,
    /// Metamodel facet: the metamodel/SHACL view of this concept
    /// (inheritance, owned/reference properties + multiplicities, relationship
    /// endpoints, applicable SHACL constraints) folded onto the ONE concept card
    /// rather than minted as a duplicate metamodel card. Absent on cards with no
    /// backing metaclass (validation/obligation/operator/tooling cards). Additive
    /// v1.1 field — omitted (not `null`) when absent, so unenriched cards are
    /// byte-identical to schema-v1 output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metamodel_facet: Option<MetamodelFacet>,
}
