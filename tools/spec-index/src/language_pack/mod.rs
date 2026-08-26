//! SysML/KerML language knowledge-pack generator.
//!
//! Pipeline driver for the deterministic manifest -> IR -> concept -> card ->
//! export path. The first end-to-end vertical was the single card
//! `sysml.behavior.transition-usage`, green against all eight acceptance
//! checks; bulk 650-concept generation builds on the same primitives.

use std::fmt;
use std::path::{Path, PathBuf};

pub mod cards;
pub mod citations;
pub mod completeness;
pub mod concepts;
pub mod denominator;
pub mod evals;
pub mod examples;
pub mod export;
pub mod info;
pub mod known_gaps;
pub mod manifest;
pub mod metamodel;
pub mod obligations;
pub mod pilot;
pub mod render_mdbook;
pub mod report;
pub mod retrieval;
pub mod retriever;
pub mod schema;
pub mod stdlib;
pub mod support;
pub mod xtext_ir;

use cards::{ClauseRef, ExamplesRef, GrammarRuleRef, LanguageCard, Provenance};
use concepts::{Classification, DenominatorRecord, Mapping};
use examples::{ComposedFile, Example, Expected, ExpectedFailure};
use export::Pack;
use report::Report;
use sysml_codegen::IrNode;
use xtext_ir::Grammars;

/// Errors raised by the language-pack generator. Every failure is a hard error
/// (fail-hard: no soft fallbacks that hide a gap).
#[derive(Debug)]
pub enum LpError {
    Io(String),
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    NotAllowlisted(String),
    ClauseNotFound(String),
    Schema(String),
    Conflict(String),
    Determinism(String),
    Other(String),
}

impl fmt::Display for LpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LpError::Io(m) => write!(f, "io: {m}"),
            LpError::HashMismatch { path, expected, actual } => write!(
                f,
                "source hash mismatch for {path}: pinned {expected}, computed {actual}"
            ),
            LpError::NotAllowlisted(p) => write!(f, "path not on the source allowlist: {p}"),
            LpError::ClauseNotFound(c) => write!(f, "clause {c} not found in derived spec text"),
            LpError::Schema(m) => write!(f, "schema: {m}"),
            LpError::Conflict(m) => write!(f, "conflict: {m}"),
            LpError::Determinism(m) => write!(f, "determinism: {m}"),
            LpError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LpError {}

/// Repo root, resolved from this crate's manifest dir (`tools/spec-index`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Default output location for the generated pack. `SYSML_LP_PACK_DIR`
/// overrides it (used by the gates to prove their skip path without touching
/// the real pack, and useful for pointing tools at an alternate pack).
pub fn default_output_dir(repo_root: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("SYSML_LP_PACK_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    repo_root.join("references/sysmlv2/derived/language-pack")
}

/// The tracked support-evidence seed. The sysml-spec-tests evidence gate
/// (`language_card_examples`, run with `SYSML_LP_UPDATE_EVIDENCE=1`) rewrites
/// this file; generation reads it. It lives under `tools/spec-index/` (not in
/// the untracked pack output) so a fresh clone can build a pack with real
/// support axes without first running the heavy parser-backed evidence gate.
pub fn evidence_seed_path(repo_root: &Path) -> PathBuf {
    repo_root.join("tools/spec-index/data/evidence.jsonl")
}

// --- Source paths ----------------------------------------------------------

const SYSML_XTEXT: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext";
const KERML_XTEXT: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/src/org/omg/kerml/xtext/KerML.xtext";
const EXPR_XTEXT: &str = "references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.kerml.expressions.xtext/src/org/omg/kerml/expressions/xtext/KerMLExpressions.xtext";
const SYSML_SPEC_TXT: &str = "references/sysmlv2/derived/SysML-spec-r2025-04.txt";
const KERML_SPEC_TXT: &str = "references/sysmlv2/derived/KerML-spec-r2025-04.txt";
const DERIVED_XTEXT_RULES: &str = "references/sysmlv2/derived/xtext-rules.toml";
const SEMANTIC_RULES_TOML: &str = "crates/lang/codegen/src/semantic_rules.toml";
const KERML_VOCAB_TTL: &str = "references/sysmlv2/Kerml-Vocab.ttl";
const SYSML_VOCAB_TTL: &str = "references/sysmlv2/SysML-vocab.ttl";
const KERML_SHAPES_TTL: &str = "references/sysmlv2/KerML-shapes.ttl";
const SYSML_SHAPES_TTL: &str = "references/sysmlv2/SysML-shapes.ttl";

#[cfg(test)]
const TU_CARD_ID: &str = "sysml.behavior.transition-usage";
#[cfg(test)]
const TU_CLAUSE: &str = "8.3.18.9";

/// True when the fetched normative sources the generator reads are on disk.
/// A fresh clone has none until `tools/fetch-references/fetch.sh fetch` runs;
/// tests use this to skip (not fail) in that state.
pub fn fetched_sources_present(repo_root: &Path) -> bool {
    repo_root.join(SYSML_XTEXT).exists()
        && repo_root.join(KERML_XTEXT).exists()
        && repo_root.join(EXPR_XTEXT).exists()
        && repo_root.join(KERML_VOCAB_TTL).exists()
        && repo_root.join(SYSML_VOCAB_TTL).exists()
        && repo_root.join(KERML_SHAPES_TTL).exists()
        && repo_root.join(SYSML_SHAPES_TTL).exists()
}

/// True when the derived spec plaintexts exist (written by the default
/// `cargo run -p spec-index` pass; generation-time inputs only, never
/// committed or redistributed).
pub fn derived_spec_text_present(repo_root: &Path) -> bool {
    repo_root.join(SYSML_SPEC_TXT).exists() && repo_root.join(KERML_SPEC_TXT).exists()
}

/// True when everything [`generate`] reads is present.
///
/// The derived xtext index is a generation input like the derived spec text:
/// both are produced by `cargo run -p spec-index` and neither is committed, so
/// a fresh checkout (CI included) has the fetched sources but not these.
pub fn generation_sources_present(repo_root: &Path) -> bool {
    fetched_sources_present(repo_root)
        && derived_spec_text_present(repo_root)
        && repo_root.join(DERIVED_XTEXT_RULES).exists()
}

/// Read an allowlisted source file (hard-rejects any non-allowlisted path).
fn read_allowlisted(repo_root: &Path, rel: &str) -> Result<String, LpError> {
    manifest::assert_allowlisted(rel)?;
    let path = repo_root.join(rel);
    std::fs::read_to_string(&path).map_err(|e| LpError::Io(format!("read {}: {e}", path.display())))
}

/// Load the three allowlisted Xtext grammars.
pub fn load_grammars(repo_root: &Path) -> Result<Grammars, LpError> {
    Ok(Grammars {
        kerml: read_allowlisted(repo_root, KERML_XTEXT)?,
        sysml: read_allowlisted(repo_root, SYSML_XTEXT)?,
        expressions: read_allowlisted(repo_root, EXPR_XTEXT)?,
    })
}

/// Collect literal keyword values from an IR tree (feeds card `keywords`).
fn collect_keywords(node: &IrNode, out: &mut Vec<String>) {
    match node {
        IrNode::Keyword { value } => {
            if !out.contains(value) {
                out.push(value.clone());
            }
        }
        IrNode::Sequence { items } | IrNode::Choice { items } | IrNode::UnorderedGroup { items } => {
            for it in items {
                collect_keywords(it, out);
            }
        }
        IrNode::Optional { item }
        | IrNode::ZeroOrMore { item }
        | IrNode::OneOrMore { item }
        | IrNode::Assignment { item, .. } => collect_keywords(item, out),
        _ => {}
    }
}

/// The xtext grammar file for a grammar label.
fn grammar_path(grammar: &str) -> &'static str {
    match grammar {
        "kerml" => KERML_XTEXT,
        "expressions" => EXPR_XTEXT,
        _ => SYSML_XTEXT,
    }
}

/// The derived spec-text file for a citation document.
fn spec_text_path(document: &str) -> &'static str {
    if document == "KerML" {
        KERML_SPEC_TXT
    } else {
        SYSML_SPEC_TXT
    }
}

/// Build the pack in memory, deriving support axes from the tracked evidence
/// seed at [`evidence_seed_path`] (written by the sysml-spec-tests evidence
/// gate). With no seed, support is all `unknown`.
pub fn generate(repo_root: &Path) -> Result<Pack, LpError> {
    let evidence = export::read_evidence_file(&evidence_seed_path(repo_root))?;
    generate_with_evidence(repo_root, &evidence)
}

/// Like [`generate`] but derives support axes from `evidence` at the pack's
/// evidence epoch. An empty slice yields all-`unknown`
/// support.
pub fn generate_with_evidence(
    repo_root: &Path,
    evidence: &[support::EvidenceRecord],
) -> Result<Pack, LpError> {
    // Stage 1: manifest — verifies every pinned source hash (AC1).
    let manifest = manifest::resolve_manifest(repo_root)?;
    let epoch = evidence_epoch(&manifest);

    // Stage 2: load only allowlisted grammar + citation sources.
    let grammars = Grammars {
        kerml: read_allowlisted(repo_root, KERML_XTEXT)?,
        sysml: read_allowlisted(repo_root, SYSML_XTEXT)?,
        expressions: read_allowlisted(repo_root, EXPR_XTEXT)?,
    };
    let universe = grammars.rule_name_universe();
    let sysml_spec_txt = read_allowlisted(repo_root, SYSML_SPEC_TXT)?;
    let kerml_spec_txt = read_allowlisted(repo_root, KERML_SPEC_TXT)?;
    let sysml_index = citations::heading_index(&sysml_spec_txt);
    let kerml_index = citations::heading_index(&kerml_spec_txt);
    // Clause contexts (heading + ~40 lines) for the topical-plausibility
    // citation gate, keyed by document label.
    let mut clause_contexts: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, String>,
    > = std::collections::BTreeMap::new();
    clause_contexts.insert("SysML".to_owned(), citations::clause_contexts(&sysml_spec_txt, 40));
    clause_contexts.insert("KerML".to_owned(), citations::clause_contexts(&kerml_spec_txt, 40));

    // Denominator closure + cross-grammar body-equivalence merge.
    let denominator_report = denominator::analyze(&grammars).to_report();

    let mut cards = Vec::new();
    let mut examples = Vec::new();
    let mut denominator = Vec::new();
    let mut aliases = std::collections::BTreeMap::new();
    let mut unknown_nodes = 0usize;

    for spec in pilot::pilot_cards() {
        // Stage 3: grammar IR.
        let ir = xtext_ir::rule_ir(&grammars, spec.grammar, spec.rule).ok_or_else(|| {
            LpError::Other(format!("rule {} not found in {}", spec.rule, spec.grammar))
        })?;
        let dangling = xtext_ir::dangling_dependencies(&ir, &universe);
        if !dangling.is_empty() {
            return Err(LpError::Conflict(format!(
                "{} has dangling rule refs: {dangling:?}",
                spec.rule
            )));
        }
        unknown_nodes += ir.unknown_count;

        // Stage 5: citation.
        let index = if spec.document == "KerML" {
            &kerml_index
        } else {
            &sysml_index
        };
        let (clause, anchor) = citations::resolve_clause(index, spec.clause)?;

        // Stage 6: examples.
        let mut ex_pos = Vec::new();
        let mut ex_neg = Vec::new();
        let mut ex_comp = Vec::new();

        let pos_id = format!("{}.positive.{}", spec.id, spec.pos.slug);
        examples.push(Example::positive(
            &pos_id,
            spec.id,
            spec.pos.source,
            Expected {
                syntax_errors: 0,
                element_kinds: spec.pos.kinds.iter().map(|s| (*s).to_owned()).collect(),
                relationships: Vec::new(),
                resolution_errors: 0,
                semantic_diagnostics: Vec::new(),
                runtime: spec.pos.runtime.to_owned(),
            },
        ));
        ex_pos.push(pos_id);

        let neg_id = format!("{}.negative.{}", spec.id, spec.neg.slug);
        examples.push(Example::negative(
            &neg_id,
            spec.id,
            spec.neg.source,
            ExpectedFailure {
                phase: spec.neg.phase.to_owned(),
                mutation_class: spec.neg.mutation.to_owned(),
                parse_failure_class: spec.neg.parse_failure_class.map(str::to_owned),
                diagnostic_category: spec.neg.diagnostic_category.map(str::to_owned),
                diagnostic_code: spec.neg.diagnostic_code.map(str::to_owned),
            },
        ));
        ex_neg.push(neg_id);

        if let Some(c) = &spec.composed {
            let comp_id = format!("{}.composed.{}", spec.id, c.slug);
            let files = c
                .files
                .iter()
                .map(|(name, role, source)| ComposedFile {
                    name: (*name).to_owned(),
                    role: (*role).to_owned(),
                    source: (*source).to_owned(),
                })
                .collect();
            examples.push(Example::composed(
                &comp_id,
                spec.id,
                files,
                Expected {
                    syntax_errors: 0,
                    element_kinds: c.kinds.iter().map(|s| (*s).to_owned()).collect(),
                    relationships: Vec::new(),
                    resolution_errors: 0,
                    semantic_diagnostics: Vec::new(),
                    runtime: "none".to_owned(),
                },
            ));
            ex_comp.push(comp_id);
        }

        // Stage 7: support axes from evidence at the epoch.
        let support = support::derive_axes(spec.id, &epoch, evidence);

        // Keyword roll-up: authored keywords + grammar keywords + title words.
        let mut keywords: Vec<String> = spec.keywords_extra.iter().map(|s| (*s).to_owned()).collect();
        collect_keywords(&ir.expression, &mut keywords);
        for w in spec.title.split_whitespace() {
            keywords.push(w.to_ascii_lowercase());
        }
        let mut seen = std::collections::HashSet::new();
        keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

        // Normative-rule provenance: the primary grammar rule plus any
        // cross-grammar merge-set siblings (a same-named,
        // IR-equivalent production declared in more than one grammar).
        let mut normative_rules = vec![GrammarRuleRef {
            grammar: spec.grammar.to_owned(),
            name: spec.rule.to_owned(),
        }];
        for (g, r) in spec.extra_rules {
            normative_rules.push(GrammarRuleRef {
                grammar: (*g).to_owned(),
                name: (*r).to_owned(),
            });
        }

        // Provenance: the distinct grammar files (primary + merge siblings),
        // then the spec-text file.
        let xtext_rel = grammar_path(spec.grammar);
        let spec_rel = spec_text_path(spec.document);
        let mut source_paths = vec![xtext_rel.to_owned()];
        for (g, _) in spec.extra_rules {
            let p = grammar_path(g).to_owned();
            if !source_paths.contains(&p) {
                source_paths.push(p);
            }
        }
        source_paths.push(spec_rel.to_owned());
        let source_hashes = source_paths
            .iter()
            .map(|p| manifest::source_hash(repo_root, p))
            .collect::<Result<Vec<_>, _>>()?;

        // Stage 8: assemble the card.
        cards.push(LanguageCard {
            schema_version: cards::SCHEMA_VERSION.to_owned(),
            id: spec.id.to_owned(),
            title: spec.title.to_owned(),
            language: spec.language.to_owned(),
            category: spec.category.iter().map(|s| (*s).to_owned()).collect(),
            summary: spec.summary.to_owned(),
            keywords,
            aliases: vec![spec.rule.to_owned()],
            normative_rules,
            normative_clauses: vec![ClauseRef {
                document: spec.document.to_owned(),
                clause,
                anchor,
                resolution: "exact".to_owned(),
            }],
            normalized_grammar: Some(ir.expression.clone()),
            rule_dependencies: ir.dependencies.clone(),
            semantic_types: spec.semantic_types.iter().map(|s| (*s).to_owned()).collect(),
            validation_rules: Vec::new(),
            examples: ExamplesRef {
                positive: ex_pos,
                negative: ex_neg,
                composed: ex_comp,
            },
            support,
            known_gaps: Vec::new(),
            related_cards: Vec::new(),
            provenance: Provenance {
                spec_drop: manifest::SPEC_DROP.to_owned(),
                source_paths,
                source_hashes,
                generated_by: manifest::GENERATED_BY.to_owned(),
            },
            metamodel_facet: None,
        });

        let mut denom_rec = DenominatorRecord::for_xtext_rule(
            spec.grammar,
            spec.rule,
            Classification::UserFacing,
            Some(spec.id.to_owned()),
            Mapping::Card,
            if spec.extra_rules.is_empty() {
                "modeller-written concept in the 20-card pilot spread"
            } else {
                "cross-grammar merge-set concept: IR-equivalent productions in \
                 more than one grammar collapse to a single card, both source \
                 rows recorded as provenance"
            },
        );
        if !spec.extra_rules.is_empty() {
            let mut merged = vec![format!("xtext:{}:{}", spec.grammar, spec.rule)];
            for (g, r) in spec.extra_rules {
                merged.push(format!("xtext:{g}:{r}"));
            }
            denom_rec.merged_from = merged;
        }
        denominator.push(denom_rec);
        aliases.insert(spec.rule.to_owned(), spec.id.to_owned());
    }

    // Cross-grammar divergent concepts: each mints TWO
    // authority-scoped cards, one per grammar, carrying only that grammar's IR,
    // cross-linked. Clause + example bytes are shared (one abstract-syntax
    // relationship, two concrete syntaxes).
    for pair in pilot::split_pairs() {
        let clause_index = if pair.document == "KerML" {
            &kerml_index
        } else {
            &sysml_index
        };
        let (clause, anchor) = citations::resolve_clause(clause_index, pair.clause)?;
        let spec_rel = spec_text_path(pair.document);

        let kerml_id = format!("kerml.{}.{}", pair.facet, pair.slug);
        let sysml_id = format!("sysml.{}.{}", pair.facet, pair.slug);

        for (authority, sibling_id) in
            [("kerml", sysml_id.clone()), ("sysml", kerml_id.clone())]
        {
            let card_id = format!("{authority}.{}.{}", pair.facet, pair.slug);

            let ir = xtext_ir::rule_ir(&grammars, authority, pair.rule).ok_or_else(|| {
                LpError::Other(format!("split rule {} not found in {authority}", pair.rule))
            })?;
            let dangling = xtext_ir::dangling_dependencies(&ir, &universe);
            if !dangling.is_empty() {
                return Err(LpError::Conflict(format!(
                    "{} ({authority}) has dangling rule refs: {dangling:?}",
                    pair.rule
                )));
            }
            unknown_nodes += ir.unknown_count;

            let pos_id = format!("{card_id}.positive.{}", pair.pos.slug);
            examples.push(Example::positive(
                &pos_id,
                &card_id,
                pair.pos.source,
                Expected {
                    syntax_errors: 0,
                    element_kinds: pair.pos.kinds.iter().map(|s| (*s).to_owned()).collect(),
                    relationships: Vec::new(),
                    resolution_errors: 0,
                    semantic_diagnostics: Vec::new(),
                    runtime: pair.pos.runtime.to_owned(),
                },
            ));
            let neg_id = format!("{card_id}.negative.{}", pair.neg.slug);
            examples.push(Example::negative(
                &neg_id,
                &card_id,
                pair.neg.source,
                ExpectedFailure {
                    phase: pair.neg.phase.to_owned(),
                    mutation_class: pair.neg.mutation.to_owned(),
                    parse_failure_class: pair.neg.parse_failure_class.map(str::to_owned),
                    diagnostic_category: pair.neg.diagnostic_category.map(str::to_owned),
                    diagnostic_code: pair.neg.diagnostic_code.map(str::to_owned),
                },
            ));

            let support = support::derive_axes(&card_id, &epoch, evidence);

            let mut keywords: Vec<String> =
                pair.keywords_extra.iter().map(|s| (*s).to_owned()).collect();
            collect_keywords(&ir.expression, &mut keywords);
            for w in pair.title.split_whitespace() {
                keywords.push(w.to_ascii_lowercase());
            }
            let mut seen = std::collections::HashSet::new();
            keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

            let xtext_rel = grammar_path(authority);
            let source_paths = vec![xtext_rel.to_owned(), spec_rel.to_owned()];
            let source_hashes = vec![
                manifest::source_hash(repo_root, xtext_rel)?,
                manifest::source_hash(repo_root, spec_rel)?,
            ];

            cards.push(LanguageCard {
                schema_version: cards::SCHEMA_VERSION.to_owned(),
                id: card_id.clone(),
                title: pair.title.to_owned(),
                language: if authority == "kerml" { "KerML" } else { "SysML" }.to_owned(),
                category: pair.category.iter().map(|s| (*s).to_owned()).collect(),
                summary: pair.summary.to_owned(),
                keywords,
                aliases: vec![pair.rule.to_owned()],
                normative_rules: vec![GrammarRuleRef {
                    grammar: authority.to_owned(),
                    name: pair.rule.to_owned(),
                }],
                normative_clauses: vec![ClauseRef {
                    document: pair.document.to_owned(),
                    clause: clause.clone(),
                    anchor: anchor.clone(),
                    resolution: "exact".to_owned(),
                }],
                normalized_grammar: Some(ir.expression.clone()),
                rule_dependencies: ir.dependencies.clone(),
                semantic_types: pair.semantic_types.iter().map(|s| (*s).to_owned()).collect(),
                validation_rules: Vec::new(),
                examples: ExamplesRef {
                    positive: vec![pos_id],
                    negative: vec![neg_id],
                    composed: Vec::new(),
                },
                support,
                known_gaps: Vec::new(),
                related_cards: vec![sibling_id.clone()],
                provenance: Provenance {
                    spec_drop: manifest::SPEC_DROP.to_owned(),
                    source_paths,
                    source_hashes,
                    generated_by: manifest::GENERATED_BY.to_owned(),
                },
                metamodel_facet: None,
            });

            denominator.push(DenominatorRecord::for_xtext_rule(
                authority,
                pair.rule,
                Classification::UserFacing,
                Some(card_id.clone()),
                Mapping::Card,
                "user-facing cross-grammar divergence: authority-scoped card carrying \
                 only this grammar's IR, cross-linked to its sibling",
            ));
            aliases.insert(format!("{authority}:{}", pair.rule), card_id);
        }
    }

    // Semantic-rule (validation-facet) cards. The 97
    // `S0xx` rules in `semantic_rules.toml` are grouped by their check function
    // into concept-oriented cards (S106/S107/S108 = one "connector owned by
    // type" constraint over three element types), matching the design's
    // `sysml.validation.requirement-constraint` example rather than minting 97
    // near-duplicate per-id cards. Backed by `validation_rules` (S-ids), not a
    // grammar rule; support stays honest-`unknown` (no per-rule example here —
    // the concept cards that trip these validators carry the firing negatives).
    {
        // element_type -> concept card id, for `related_cards` cross-links.
        let mut concept_by_type: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for c in &cards {
            for t in &c.semantic_types {
                concept_by_type.entry(t.clone()).or_insert_with(|| c.id.clone());
            }
        }

        let rules_src = read_allowlisted(repo_root, SEMANTIC_RULES_TOML)?;
        let rules = sysml_codegen::semantic_rule_parser::parse_semantic_rules(&rules_src)
            .map_err(|e| LpError::Other(format!("parse semantic_rules.toml: {e}")))?;

        // Group by check function (deterministic order).
        let mut by_check: std::collections::BTreeMap<String, Vec<&sysml_codegen::SemanticRule>> =
            std::collections::BTreeMap::new();
        for r in &rules {
            by_check.entry(r.check.clone()).or_default().push(r);
        }

        let semrules_hash = manifest::source_hash(repo_root, SEMANTIC_RULES_TOML)?;

        for (check, group) in &by_check {
            // KerML authority only if *every* rule in the group is KerML-cited.
            let all_kerml = group.iter().all(|r| r.spec_ref.starts_with("KerML"));
            let authority = if all_kerml { "kerml" } else { "sysml" };
            let document = if all_kerml { "KerML" } else { "SysML" };

            let slug = check.replace('_', "-");
            let card_id = format!("{authority}.validation.{slug}");
            let title = check
                .split('_')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            let mut sids: Vec<String> = group.iter().map(|r| r.id.clone()).collect();
            sids.sort();
            let mut sem_types: Vec<String> =
                group.iter().map(|r| r.element_type.clone()).collect();
            sem_types.sort();
            sem_types.dedup();

            // Clause: try every rule in the group; use the first spec_ref whose
            // leading clause-number token resolves to a real heading (or its
            // deepest existing ancestor). Never fake a citation. Even when no
            // clause resolves, these cards carry a metamodel-element locator via
            // `semantic_types`, so the citation gate is still satisfied.
            let Some(first) = group.first() else { continue };
            let mut normative_clauses = Vec::new();
            for r in group {
                let Some((doc_label, rest)) = r.spec_ref.split_once(' ') else { continue };
                let Some(clause_num) = citations::clause_number_token(rest) else { continue };
                let idx = if doc_label == "KerML" { &kerml_index } else { &sysml_index };
                if let Ok((clause, anchor, resolution)) =
                    citations::resolve_clause_or_ancestor(idx, clause_num)
                {
                    normative_clauses.push(ClauseRef {
                        document: doc_label.to_owned(),
                        clause,
                        anchor,
                        resolution: resolution.to_owned(),
                    });
                    break;
                }
            }

            // Cross-link to the concept cards this rule constrains.
            let mut related: Vec<String> = sem_types
                .iter()
                .filter_map(|t| concept_by_type.get(t).cloned())
                .collect();
            related.sort();
            related.dedup();

            let mut keywords: Vec<String> = check.split('_').map(str::to_owned).collect();
            keywords.extend(sem_types.iter().map(|t| t.to_ascii_lowercase()));
            keywords.extend(sids.iter().map(|s| s.to_ascii_lowercase()));
            keywords.push("validation".to_owned());
            let mut seen = std::collections::HashSet::new();
            keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

            cards.push(LanguageCard {
                schema_version: cards::SCHEMA_VERSION.to_owned(),
                id: card_id.clone(),
                title,
                language: if authority == "kerml" { "KerML" } else { "SysML" }.to_owned(),
                category: vec!["validation".to_owned()],
                summary: first.message.clone(),
                keywords,
                aliases: sids.clone(),
                normative_rules: Vec::new(),
                normative_clauses,
                normalized_grammar: None,
                rule_dependencies: Vec::new(),
                semantic_types: sem_types,
                validation_rules: sids,
                examples: ExamplesRef {
                    positive: Vec::new(),
                    negative: Vec::new(),
                    composed: Vec::new(),
                },
                support: support::derive_axes(&card_id, &epoch, evidence),
                known_gaps: Vec::new(),
                related_cards: related,
                provenance: Provenance {
                    spec_drop: manifest::SPEC_DROP.to_owned(),
                    source_paths: vec![
                        SEMANTIC_RULES_TOML.to_owned(),
                        spec_text_path(document).to_owned(),
                    ],
                    source_hashes: vec![
                        semrules_hash.clone(),
                        manifest::source_hash(repo_root, spec_text_path(document))?,
                    ],
                    generated_by: manifest::GENERATED_BY.to_owned(),
                },
                metamodel_facet: None,
            });

            let mut denom_rec = DenominatorRecord::for_xtext_rule(
                authority,
                check,
                Classification::SemanticOnly,
                Some(card_id.clone()),
                Mapping::Card,
                "semantic constraint (S0xx) grouped by check function into one \
                 validation-facet card",
            );
            denom_rec.source_id = format!("validation:{check}");
            denom_rec.source_kind = "validation".to_owned();
            denom_rec.source_pointer =
                format!("crates/lang/codegen/src/semantic_rules.toml#{check}");
            denominator.push(denom_rec);
            aliases.insert(format!("check:{check}"), card_id);
        }
    }

    // Expression operator cards (facet `expression`).
    // One card per operator rule in KerMLExpressions.xtext, carrying the 16-level
    // precedence codegen already parses plus associativity (additive/
    // multiplicative/relational/… left-associative; exponentiation right-
    // associative per KerMLExpressions.xtext — the grammar is the authority, the
    // recent runtime associativity fix is only implementation evidence). Backed
    // by the operator's grammar rule + IR; no per-operator example here, so
    // support stays honest-unknown.
    {
        let operators = sysml_codegen::parse_xtext_operators(&grammars.expressions);
        for op in &operators {
            let slug = concepts::slugify(&op.name);
            let card_id = format!("kerml.expression.{slug}");
            let title = op
                .name
                .strip_suffix("Operator")
                .map(|s| format!("{s} Operator"))
                .unwrap_or_else(|| op.name.clone());

            // Associativity: exponentiation is right-associative, the unary
            // operator is prefix; every other binary operator is left-associative.
            let assoc = if op.category == "exponentiation" {
                "right-associative"
            } else if op.category == "unary" {
                "a prefix unary operator"
            } else {
                "left-associative"
            };
            let symbols = op.symbols.join(" ");
            let summary = format!(
                "The {} groups expression operands with precedence level {} of 16 (higher binds \
                 tighter); it is {}. Symbols: {}.",
                title.to_ascii_lowercase(),
                op.precedence,
                assoc,
                if symbols.is_empty() { "(rule reference)" } else { &symbols }
            );

            // Attach the operator rule's IR where it parses cleanly; else null.
            let ir = xtext_ir::rule_ir(&grammars, "expressions", &op.name);
            let (normalized_grammar, rule_dependencies) = match &ir {
                Some(ir) if xtext_ir::dangling_dependencies(ir, &universe).is_empty() => {
                    unknown_nodes += ir.unknown_count;
                    (Some(ir.expression.clone()), ir.dependencies.clone())
                }
                _ => (None, Vec::new()),
            };

            let mut keywords: Vec<String> = op.symbols.clone();
            keywords.push(op.category.clone());
            keywords.push("operator".to_owned());
            keywords.push("expression".to_owned());
            for w in title.split_whitespace() {
                keywords.push(w.to_ascii_lowercase());
            }
            let mut seen = std::collections::HashSet::new();
            keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

            let xtext_rel = EXPR_XTEXT;
            cards.push(LanguageCard {
                schema_version: cards::SCHEMA_VERSION.to_owned(),
                id: card_id.clone(),
                title,
                language: "Expressions".to_owned(),
                category: vec!["expression".to_owned()],
                summary,
                keywords,
                aliases: vec![op.name.clone()],
                normative_rules: vec![GrammarRuleRef {
                    grammar: "expressions".to_owned(),
                    name: op.name.clone(),
                }],
                normative_clauses: Vec::new(),
                normalized_grammar,
                rule_dependencies,
                semantic_types: Vec::new(),
                validation_rules: Vec::new(),
                examples: ExamplesRef {
                    positive: Vec::new(),
                    negative: Vec::new(),
                    composed: Vec::new(),
                },
                support: support::derive_axes(&card_id, &epoch, evidence),
                known_gaps: Vec::new(),
                related_cards: Vec::new(),
                provenance: Provenance {
                    spec_drop: manifest::SPEC_DROP.to_owned(),
                    source_paths: vec![xtext_rel.to_owned()],
                    source_hashes: vec![manifest::source_hash(repo_root, xtext_rel)?],
                    generated_by: manifest::GENERATED_BY.to_owned(),
                },
                metamodel_facet: None,
            });

            denominator.push(DenominatorRecord::for_xtext_rule(
                "expressions",
                &op.name,
                Classification::Operator,
                Some(card_id.clone()),
                Mapping::Card,
                "expression operator with 16-level precedence (expression facet)",
            ));
            aliases.insert(format!("expressions:{}", op.name), card_id);
        }
    }

    // Obligation cards (semantic-only). One validation-
    // facet card per *reviewed-valid* obligation from spec-obligations/*.md:
    // carded ONLY when the obligation has a `// OBL:` marker on a currently-
    // green (non-`#[ignore]`d) gate test (the executable re-validation signal —
    // never a copied tracker verdict). Deferred/ungated/open-gap obligations
    // stay in the denominator as `excluded` with the tracker's status cell as
    // rationale. Where an obligation names the same concept as an already-minted
    // validation card (a shared S0xx well-formedness check), the obligation id
    // folds into that card's `validation_rules` — one home, no duplicate card.
    {
        let obls = obligations::parse_obligations(|rel| read_allowlisted(repo_root, rel))?;
        let gated_green = obligations::gated_green_ids(repo_root)?;

        // facet+slug -> index of an existing validation card (authority-agnostic
        // merge target), built once over the cards minted so far.
        let mut validation_by_slug: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for (i, c) in cards.iter().enumerate() {
            if let Some(slug) = c.id.strip_prefix("kerml.validation.").or_else(|| c.id.strip_prefix("sysml.validation.")) {
                validation_by_slug.entry(slug.to_owned()).or_insert(i);
            }
        }

        // Obligation-id -> stdlib card id, from the static stdlib table's
        // declared `validation_rules`. An obligation whose
        // normative home is a standard-library symbol folds into that stdlib
        // card (its locator) instead of being blocked. The stdlib cards are
        // minted later in this function; the fold only needs the card *id*, which
        // the table already fixes, and the minted card already declares the
        // obligation id in its `validation_rules`, so no reordering is required.
        let mut stdlib_fold: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for spec in stdlib::stdlib_cards() {
            for rule in spec.validation_rules {
                stdlib_fold.entry((*rule).to_owned()).or_insert_with(|| spec.id.to_owned());
            }
        }

        for obl in &obls {
            let clause_index = if obl.document == "KerML" { &kerml_index } else { &sysml_index };
            let resolved_clause = obl
                .clause
                .as_deref()
                .and_then(|c| citations::resolve_clause_or_ancestor(clause_index, c).ok());

            if !gated_green.contains(&obl.id) {
                // Not re-validated by a green gate: stays in the denominator as
                // an excluded record with the tracker's own status as rationale.
                let mut rec = DenominatorRecord::for_xtext_rule(
                    obl.authority.as_str(),
                    &obl.id,
                    Classification::Excluded,
                    None,
                    Mapping::Exclusion,
                    &obligations::exclusion_rationale(&obl.status),
                );
                rec.source_id = format!("obligation:{}", obl.id);
                rec.source_kind = "obligation".to_owned();
                rec.source_pointer =
                    format!("crates/testing/sysml-spec-tests/spec-obligations/{}.md#{}", obl.area, obl.id);
                rec.review_state = "reviewed".to_owned();
                denominator.push(rec);
                continue;
            }

            // Reviewed-valid. Merge into an existing validation card if this
            // concept is already carded (a shared S0xx check), else mint one.
            if let Some(card) = validation_by_slug.get(&obl.id).and_then(|&idx| cards.get_mut(idx)) {
                if !card.validation_rules.contains(&obl.id) {
                    card.validation_rules.push(obl.id.clone());
                    card.validation_rules.sort();
                }
                if !card.aliases.contains(&obl.id) {
                    card.aliases.push(obl.id.clone());
                }
                let concept_id = card.id.clone();
                let mut rec = DenominatorRecord::for_xtext_rule(
                    obl.authority.as_str(),
                    &obl.id,
                    Classification::SemanticOnly,
                    None,
                    Mapping::Duplicate,
                    "reviewed-valid obligation folded into the existing validation card for \
                     the same concept (shared well-formedness check); re-validated by a \
                     currently-green `// OBL:` gate marker. Not a distinct carded \
                     concept — it collapses into the target and does not inflate card_coverage",
                );
                rec.source_id = format!("obligation:{}", obl.id);
                rec.source_kind = "obligation".to_owned();
                rec.source_pointer =
                    format!("crates/testing/sysml-spec-tests/spec-obligations/{}.md#{}", obl.area, obl.id);
                rec.review_state = "reviewed".to_owned();
                rec.mapping_target = Some(concept_id.clone());
                denominator.push(rec);
                aliases.insert(format!("obl:{}", obl.id), concept_id);
                continue;
            }

            // Fold into a standard-library card whose definition IS this
            // obligation's normative home. The stdlib card
            // declares this obligation in its `validation_rules` and is minted
            // later; the obligation collapses into it (its locator) rather than
            // being blocked. Not a distinct carded concept — no coverage inflation.
            if let Some(stdlib_id) = stdlib_fold.get(&obl.id) {
                let mut rec = DenominatorRecord::for_xtext_rule(
                    obl.authority.as_str(),
                    &obl.id,
                    Classification::SemanticOnly,
                    None,
                    Mapping::Duplicate,
                    "reviewed-valid obligation folded into the standard-library card that defines \
                     its normative semantics; the stdlib card \
                     is its locator. Not a distinct carded concept — it does not inflate coverage",
                );
                rec.source_id = format!("obligation:{}", obl.id);
                rec.source_kind = "obligation".to_owned();
                rec.source_pointer =
                    format!("crates/testing/sysml-spec-tests/spec-obligations/{}.md#{}", obl.area, obl.id);
                rec.review_state = "reviewed".to_owned();
                rec.mapping_target = Some(stdlib_id.clone());
                denominator.push(rec);
                aliases.insert(format!("obl:{}", obl.id), stdlib_id.clone());
                continue;
            }

            // Citation gate: a newly-minted obligation
            // card has no grammar rule and no metamodel semantic_types, so its
            // only possible normative locator is a spec clause. If none resolves
            // (a library-only obligation whose normative home is a standard-
            // library symbol), BLOCK it — do not fake a clause. It stays a
            // visible blocked denominator slot pending stdlib-symbol locator
            // integration. This is the "else mark blocked-with-rationale" path.
            if resolved_clause.is_none() {
                let mut rec = DenominatorRecord::for_xtext_rule(
                    obl.authority.as_str(),
                    &obl.id,
                    Classification::SemanticOnly,
                    None,
                    Mapping::Block,
                    &format!(
                        "reviewed-valid obligation blocked from carding: no resolvable spec-clause \
                         locator (citation cell: `{}`); its normative home is a standard-library \
                         symbol, with no structured locator source yet. Surfaced as a blocked \
                         slot rather than carded without a normative locator",
                        obl.citation.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect::<String>()
                    ),
                );
                rec.source_id = format!("obligation:{}", obl.id);
                rec.source_kind = "obligation".to_owned();
                rec.source_pointer =
                    format!("crates/testing/sysml-spec-tests/spec-obligations/{}.md#{}", obl.area, obl.id);
                rec.review_state = "reviewed".to_owned();
                denominator.push(rec);
                continue;
            }

            let card_id = format!("{}.validation.{}", obl.authority, obl.id);

            let title = {
                let mut words: Vec<String> = obl
                    .id
                    .split('-')
                    .map(|w| {
                        let mut ch = w.chars();
                        match ch.next() {
                            Some(f) => f.to_ascii_uppercase().to_string() + ch.as_str(),
                            None => String::new(),
                        }
                    })
                    .collect();
                let mut t = words.join(" ");
                if t.len() > 120 {
                    t.truncate(117);
                    while !t.is_char_boundary(t.len()) {
                        t.pop();
                    }
                    t.push_str("...");
                }
                words.clear();
                t
            };

            let summary = {
                let s = if obl.text.trim().is_empty() {
                    obl.id.replace('-', " ")
                } else {
                    obl.text.clone()
                };
                let mut s = s;
                if s.chars().count() > 1200 {
                    s = s.chars().take(1197).collect::<String>();
                    s.push_str("...");
                }
                s
            };

            let mut category = Vec::new();
            if let Some(c) = obligations::area_category(&obl.area) {
                category.push(c.to_owned());
            }
            category.push("validation".to_owned());
            category.dedup();

            let mut keywords: Vec<String> = obl.id.split('-').map(str::to_owned).collect();
            keywords.push(obl.area.replace('-', " "));
            keywords.push("obligation".to_owned());
            keywords.push("validation".to_owned());
            keywords.push(obl.document.to_ascii_lowercase());
            let mut seen = std::collections::HashSet::new();
            keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

            let spec_rel = spec_text_path(&obl.document);
            let area_rel = match obl.area.as_str() {
                "actions" => manifest::OBLIGATION_ACTIONS,
                "calculations" => manifest::OBLIGATION_CALCULATIONS,
                "constraints-expressions" => manifest::OBLIGATION_CONSTRAINTS,
                "flows-ports" => manifest::OBLIGATION_FLOWS_PORTS,
                "occurrences-clocks" => manifest::OBLIGATION_OCCURRENCES,
                "ode-physics" => manifest::OBLIGATION_ODE_PHYSICS,
                "requirements" => manifest::OBLIGATION_REQUIREMENTS,
                "state-machines" => manifest::OBLIGATION_STATE_MACHINES,
                "structural" => manifest::OBLIGATION_STRUCTURAL,
                _ => manifest::OBLIGATION_VERIFICATION,
            };
            let source_paths = vec![area_rel.to_owned(), spec_rel.to_owned()];
            let source_hashes = vec![
                manifest::source_hash(repo_root, area_rel)?,
                manifest::source_hash(repo_root, spec_rel)?,
            ];

            let normative_clauses = match &resolved_clause {
                Some((clause, anchor, resolution)) => vec![ClauseRef {
                    document: obl.document.clone(),
                    clause: clause.clone(),
                    anchor: anchor.clone(),
                    resolution: (*resolution).to_owned(),
                }],
                None => Vec::new(),
            };

            cards.push(LanguageCard {
                schema_version: cards::SCHEMA_VERSION.to_owned(),
                id: card_id.clone(),
                title,
                language: obl.document.clone(),
                category,
                summary,
                keywords,
                aliases: vec![obl.id.clone()],
                normative_rules: Vec::new(),
                normative_clauses,
                normalized_grammar: None,
                rule_dependencies: Vec::new(),
                semantic_types: Vec::new(),
                validation_rules: vec![obl.id.clone()],
                examples: ExamplesRef {
                    positive: Vec::new(),
                    negative: Vec::new(),
                    composed: Vec::new(),
                },
                support: support::derive_axes(&card_id, &epoch, evidence),
                known_gaps: Vec::new(),
                related_cards: Vec::new(),
                provenance: Provenance {
                    spec_drop: manifest::SPEC_DROP.to_owned(),
                    source_paths,
                    source_hashes,
                    generated_by: manifest::GENERATED_BY.to_owned(),
                },
                metamodel_facet: None,
            });
            validation_by_slug.insert(obl.id.clone(), cards.len() - 1);

            let mut rec = DenominatorRecord::for_xtext_rule(
                obl.authority.as_str(),
                &obl.id,
                Classification::SemanticOnly,
                Some(card_id.clone()),
                Mapping::Card,
                "reviewed-valid semantic-conformance obligation, re-validated by a currently-green \
                 `// OBL:` gate marker in sysml-spec-tests",
            );
            rec.source_id = format!("obligation:{}", obl.id);
            rec.source_kind = "obligation".to_owned();
            rec.source_pointer =
                format!("crates/testing/sysml-spec-tests/spec-obligations/{}.md#{}", obl.area, obl.id);
            rec.review_state = "reviewed".to_owned();
            denominator.push(rec);
            aliases.insert(format!("obl:{}", obl.id), card_id);
        }
    }

    // Tooling implementation-limitation cards. One
    // `tooling.implementation.*` card per known-gap-registry record; each links
    // the normative card(s) it qualifies (never replaces them) and carries the
    // gap id in `known_gaps`. Support stays honest-`unknown` (the limitation is
    // recorded via known_gaps, not a fabricated axis value). Grounded in the
    // grammar file that declares the affected concept.
    let known_gaps = known_gaps::registry();
    for gap in &known_gaps {
        let xtext_rel = grammar_path(&gap.authority);
        let mut keywords: Vec<String> = vec![
            concepts::slugify(&gap.concept),
            "tooling".to_owned(),
            "implementation".to_owned(),
            "limitation".to_owned(),
            "known-gap".to_owned(),
        ];
        let mut seen = std::collections::HashSet::new();
        keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

        cards.push(LanguageCard {
            schema_version: cards::SCHEMA_VERSION.to_owned(),
            id: gap.tooling_card.clone(),
            title: gap.title.clone(),
            language: "Tooling".to_owned(),
            category: vec!["implementation".to_owned()],
            summary: gap.summary.clone(),
            keywords,
            aliases: vec![gap.concept.clone()],
            normative_rules: Vec::new(),
            normative_clauses: Vec::new(),
            normalized_grammar: None,
            rule_dependencies: Vec::new(),
            semantic_types: Vec::new(),
            validation_rules: Vec::new(),
            examples: ExamplesRef {
                positive: Vec::new(),
                negative: Vec::new(),
                composed: Vec::new(),
            },
            support: support::derive_axes(&gap.tooling_card, &epoch, evidence),
            known_gaps: vec![gap.id.clone()],
            related_cards: gap.related_cards.clone(),
            provenance: Provenance {
                spec_drop: manifest::SPEC_DROP.to_owned(),
                source_paths: vec![xtext_rel.to_owned()],
                source_hashes: vec![manifest::source_hash(repo_root, xtext_rel)?],
                generated_by: manifest::GENERATED_BY.to_owned(),
            },
            metamodel_facet: None,
        });
        aliases.insert(format!("gap:{}", gap.id), gap.tooling_card.clone());
    }

    // Standard-library semantic cards (Tier 4 sources /
    // `library-defined`). Curated load-bearing constructs whose meaning lives in
    // the normative model library (root CLAUDE.md precedence 2): the verdict /
    // case / requirement-check / constraint-check machinery. Each cites its
    // defining library file (Tier 4) + the spec clause (try-resolved, never
    // faked); no grammar IR (`normalized_grammar` = null); support honest-
    // `unknown` (no parse examples — the meaning is the library definition). The
    // VerdictKind card carries the `verdict-kind-enumeration` obligation id and
    // cross-links the existing `sysml.validation.verdict-kind-enumeration` card:
    // the obligation stays the CHECK's one home, this card is the DEFINITION's
    // one home, bidirectionally linked (a reviewed merge decision).
    let stdlib_start = cards.len();
    for spec in stdlib::stdlib_cards() {
        let authority = spec.id.split('.').next().unwrap_or("sysml");

        // Clause: try-resolve against the derived spec-text heading index; ship
        // it only if it resolves to a real heading (never fake a citation).
        let index = if spec.document == "KerML" { &kerml_index } else { &sysml_index };
        let normative_clauses = match citations::resolve_clause(index, spec.clause) {
            Ok((clause, anchor)) => vec![ClauseRef {
                document: spec.document.to_owned(),
                clause,
                anchor,
                resolution: "exact".to_owned(),
            }],
            Err(_) => Vec::new(),
        };

        let mut keywords: Vec<String> = vec![
            spec.element.to_ascii_lowercase(),
            "library".to_owned(),
            authority.to_owned(),
        ];
        keywords.extend(spec.keywords_extra.iter().map(|s| s.to_string()));
        for w in spec.title.split_whitespace() {
            keywords.push(w.to_ascii_lowercase());
        }
        let mut seen = std::collections::HashSet::new();
        keywords.retain(|k| !k.is_empty() && seen.insert(k.clone()));

        let spec_rel = spec_text_path(spec.document);
        let source_paths = vec![spec.library_path.to_owned(), spec_rel.to_owned()];
        let source_hashes = vec![
            manifest::source_hash(repo_root, spec.library_path)?,
            manifest::source_hash(repo_root, spec_rel)?,
        ];

        cards.push(LanguageCard {
            schema_version: cards::SCHEMA_VERSION.to_owned(),
            id: spec.id.to_owned(),
            title: spec.title.to_owned(),
            language: spec.language.to_owned(),
            category: vec!["library".to_owned()],
            summary: spec.summary.to_owned(),
            keywords,
            aliases: vec![spec.element.to_owned()],
            normative_rules: Vec::new(),
            normative_clauses,
            normalized_grammar: None,
            rule_dependencies: Vec::new(),
            semantic_types: Vec::new(),
            validation_rules: spec.validation_rules.iter().map(|s| s.to_string()).collect(),
            examples: ExamplesRef {
                positive: Vec::new(),
                negative: Vec::new(),
                composed: Vec::new(),
            },
            support: support::derive_axes(spec.id, &epoch, evidence),
            known_gaps: Vec::new(),
            related_cards: spec.related_cards.iter().map(|s| s.to_string()).collect(),
            provenance: Provenance {
                spec_drop: manifest::SPEC_DROP.to_owned(),
                source_paths,
                source_hashes,
                generated_by: manifest::GENERATED_BY.to_owned(),
            },
            metamodel_facet: None,
        });

        let mut rec = DenominatorRecord::for_xtext_rule(
            authority,
            spec.element,
            Classification::LibraryDefined,
            Some(spec.id.to_owned()),
            Mapping::Card,
            "library-defined semantic construct: its meaning comes from the normative standard \
             model library, not the grammar (root CLAUDE.md source precedence 2)",
        );
        rec.source_id = format!("stdlib:{}", spec.element);
        rec.source_kind = "stdlib".to_owned();
        rec.source_pointer = format!("{}#{}", spec.library_path, spec.element);
        rec.review_state = "reviewed".to_owned();
        denominator.push(rec);
        aliases.insert(format!("stdlib:{}", spec.element), spec.id.to_owned());
    }

    // Make every stdlib cross-link bidirectional: add the stdlib card's id back
    // into each target card's `related_cards` (dedup + sort). Targets are either
    // sibling stdlib cards or already-minted normative cards (e.g. the verdict
    // obligation card), so the reverse edge always resolves.
    let stdlib_backlinks: Vec<(String, Vec<String>)> = cards
        .iter()
        .skip(stdlib_start)
        .map(|c| (c.id.clone(), c.related_cards.clone()))
        .collect();
    for (src_id, targets) in stdlib_backlinks {
        for target in targets {
            if let Some(card) = cards.iter_mut().find(|c| c.id == target) {
                if !card.related_cards.contains(&src_id) {
                    card.related_cards.push(src_id.clone());
                    card.related_cards.sort();
                    card.related_cards.dedup();
                }
            }
        }
    }

    // Drop any cross-link to a card that was not minted (e.g. an obligation
    // blocked by the citation gate above). Keeps `related_cards` closed under
    // the dangling-reference gate. Deterministic (order preserved, dedup kept).
    {
        let minted: std::collections::BTreeSet<String> =
            cards.iter().map(|c| c.id.clone()).collect();
        for card in &mut cards {
            card.related_cards.retain(|r| minted.contains(r));
        }
    }

    // Complete the denominator to the FULL 650-name inventory + the metamodel/
    // SHACL/standard-library-source slots. The card
    // loops above emitted a record only for concepts that got a card; this adds
    // one auditable-mapping record for every remaining unique grammar rule name
    // (Uncarded for in-scope card-bearing gaps, HelperFold for helpers) so an
    // in-scope concept with no card LOWERS coverage instead of being dropped.
    {
        let already_covered: std::collections::BTreeSet<String> = denominator
            .iter()
            .filter(|r| r.source_kind == "xtext")
            .map(|r| r.raw_name.clone())
            .collect();
        denominator.extend(denominator::complete_xtext_closure(
            &grammars,
            &cards,
            &already_covered,
        ));

        // Fold the metamodel (182 classes) + the
        // SHACL/OSLC shapes (257 raw declarations → 175 distinct constrained
        // types) into the pack. Each metaclass/shape becomes a REVIEWED denominator
        // record with an auditable disposition, and the common case ENRICHES the
        // one concept card that lowers to the metaclass with its metamodel facet
        // (inheritance, owned/reference properties + multiplicities, relationship
        // endpoints, applicable SHACL constraints) — never a duplicate
        // grammar+metamodel card. Replaces the opaque `block`-mapped slots the
        // review flagged (metamodel 0/182, shacl 0/257). Reuses the codegen
        // TTL/OSLC parsers (one home; no second TTL parser).
        {
            let idx = metamodel::MetamodelIndex::load(
                &read_allowlisted(repo_root, KERML_VOCAB_TTL)?,
                &read_allowlisted(repo_root, SYSML_VOCAB_TTL)?,
                &read_allowlisted(repo_root, KERML_SHAPES_TTL)?,
                &read_allowlisted(repo_root, SYSML_SHAPES_TTL)?,
                KERML_VOCAB_TTL,
                SYSML_VOCAB_TTL,
                KERML_SHAPES_TTL,
                SYSML_SHAPES_TTL,
            )
            .map_err(LpError::Other)?;
            let integration = metamodel::integrate(&idx, &mut cards, &kerml_index, repo_root)?;
            denominator.extend(integration.records);
        }

        // The reviewed standard-library
        // denominator — one aggregate-root record per library package in the
        // explicit manifest (93 packages), replacing the former single opaque
        // `unknown-blocks-completion` slot. The load-bearing member constructs are
        // carded selectively (stdlib cards); a package's rationale records whether
        // it contributes one.
        let carded_paths: std::collections::BTreeSet<String> =
            stdlib::stdlib_cards().iter().map(|c| c.library_path.to_owned()).collect();
        denominator.extend(denominator::library_package_records(&carded_paths));
    }

    // Gates: duplicate IDs + dangling references + the
    // citation gate: a normative card without a locator.
    let mut conflicts = report::duplicate_ids(&cards, &examples);
    conflicts.extend(report::dangling_references(&cards, &examples));
    conflicts.extend(report::cards_without_locator(&cards));
    conflicts.extend(report::cards_missing_topical_citation(&cards, &clause_contexts));
    if !conflicts.is_empty() {
        return Err(LpError::Conflict(conflicts.join("; ")));
    }

    let report = Report {
        spec_drop: manifest::SPEC_DROP.to_owned(),
        card_count: cards.len(),
        example_count: examples.len(),
        denominator_count: denominator.len(),
        unknown_grammar_nodes: unknown_nodes,
        conflicts,
        notes: vec![
            "Support evidence `commit` is a stable 40-hex spec-drop evidence epoch (a digest of \
             the pinned source set), not a git OID, so the tracked pack stays regen-diff-stable \
             across commits while stale evidence auto-invalidates when the sources change \
             (the same content-stability reasoning applied to the evidence axis)."
                .to_owned(),
            "Schema version 1 is frozen unchanged; the one permitted revision was not \
             required."
                .to_owned(),
            "Provisional min-evidence not fully met on some axes and honestly reported as \
             `unknown` (never satisfied by generic fallback): resolve for transition-usage and \
             succession/state-usage/multiplicity/requirement-definition (their positives carry no \
             resolvable cross-references and their engine paths do not resolve endpoints); \
             validate for specialization/multiplicity/port-usage/succession/transition-usage/\
             satisfaction/analysis-case/view-usage/feature-value/attribute-definition/part-usage/\
             action-usage/invocation/import/verification-case (no concept-specific semantic \
             validator exists yet). Four cards reach real semantic validation: namespace (S001), \
             flow-connection (S108), state-usage (S068), requirement-definition (S060)."
                .to_owned(),
        ],
        tree_hash: String::new(),
    };

    // Grammar rule-ref graph for transitive helper-fold resolution in the
    // dependency-expansion map: rule name -> direct refs.
    let mut rule_refs: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for ir in grammars.all_ir() {
        let mut refs = Vec::new();
        xtext_ir::rule_ref_names(&ir.expression, &mut refs);
        rule_refs.entry(ir.rule).or_insert(refs);
    }

    Ok(Pack {
        manifest,
        cards,
        examples,
        denominator,
        denominator_report,
        aliases,
        rule_refs,
        evidence: evidence.to_vec(),
        known_gaps,
        report,
    })
}

/// Stable evidence-epoch sentinel: a hex digest of the manifest source set.
/// Support evidence is tied to the *spec drop*,
/// not a git commit, so the pack stays regen-diff-stable across commits while
/// still auto-invalidating stale evidence when the sources change.
pub fn evidence_epoch(manifest: &manifest::Manifest) -> String {
    let mut acc = Vec::new();
    acc.extend_from_slice(manifest.spec_drop.as_bytes());
    acc.push(0);
    for s in &manifest.sources {
        acc.extend_from_slice(s.path.as_bytes());
        acc.push(0);
        acc.extend_from_slice(s.sha256.as_bytes());
        acc.push(b'\n');
    }
    // Truncated to 40 hex so it fits the evidence-record `commit` pattern
    // (`^[0-9a-f]{7,40}$`); still uniquely identifies the source drop.
    crate::sha256_hex(&acc)[..40].to_owned()
}


/// Validate every card/example in a pack against its committed JSON schema
/// (schema gate). Returns a hard error listing all violations.
pub fn validate_pack(repo_root: &Path, pack: &Pack) -> Result<(), LpError> {
    let schemas = schema::SchemaSet::load(repo_root)?;
    for card in &pack.cards {
        let value = export::to_value(card)?;
        if let Err(errs) = schemas.validate("language-card.schema.json", &value) {
            return Err(LpError::Schema(format!("card {}: {}", card.id, errs.join("; "))));
        }
    }
    for ex in &pack.examples {
        let value = export::to_value(ex)?;
        if let Err(errs) = schemas.validate("example.schema.json", &value) {
            return Err(LpError::Schema(format!("example {}: {}", ex.id, errs.join("; "))));
        }
    }
    for rec in &pack.denominator {
        let value = export::to_value(rec)?;
        if let Err(errs) = schemas.validate("denominator-record.schema.json", &value) {
            return Err(LpError::Schema(format!(
                "denominator {}: {}",
                rec.source_id,
                errs.join("; ")
            )));
        }
    }
    for rec in &pack.evidence {
        let value = export::to_value(rec)?;
        if let Err(errs) = schemas.validate("evidence-record.schema.json", &value) {
            return Err(LpError::Schema(format!(
                "evidence {}/{}: {}",
                rec.card_id,
                rec.axis,
                errs.join("; ")
            )));
        }
    }
    Ok(())
}

/// Full generate + validate + export to `out_dir`. Returns the tree hash.
pub fn run(repo_root: &Path, out_dir: &Path) -> Result<String, LpError> {
    let pack = generate(repo_root)?;
    validate_pack(repo_root, &pack)?;
    export::export_pack(&pack, out_dir)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod vertical_slice {
    use super::*;

    const TU_RULE: &str = "TransitionUsage";
    const TU_GRAMMAR: &str = "sysml";

    fn root() -> PathBuf {
        repo_root()
    }

    fn transition_card(pack: &Pack) -> &LanguageCard {
        pack.cards
            .iter()
            .find(|c| c.id == TU_CARD_ID)
            .expect("pilot must contain the transition-usage card")
    }

    /// AC1: a wrong pinned hash aborts with a hard error; the real pin passes.
    #[test]
    fn ac1_manifest_hash_verification() {
        if !fetched_sources_present(&root()) {
            eprintln!("SKIP: references not fetched (run tools/fetch-references/fetch.sh fetch)");
            return;
        }
        let r = root();
        // Real pin verifies.
        assert!(manifest::verify_pinned_hash(&r, SYSML_XTEXT).is_ok());
        // A deliberately wrong hash is a hard HashMismatch.
        let bad = "0".repeat(64);
        let err = manifest::verify_hash_against(&r, SYSML_XTEXT, &bad).unwrap_err();
        assert!(matches!(err, LpError::HashMismatch { .. }), "got {err:?}");
    }

    /// AC2: a non-allowlisted path is rejected.
    #[test]
    fn ac2_allowlist_rejects_unknown_paths() {
        let sneaky = "references/sysmlv2/SysML-v2-Pilot-Implementation/some/other/File.xtext";
        assert!(manifest::allowlisted_kind(sneaky).is_none());
        let err = read_allowlisted(&root(), sneaky).unwrap_err();
        assert!(matches!(err, LpError::NotAllowlisted(_)), "got {err:?}");
    }

    /// AC3: TransitionUsage IR — sequence root, zero unknowns, deps all resolve.
    #[test]
    fn ac3_transition_usage_ir() {
        if !fetched_sources_present(&root()) {
            eprintln!("SKIP: references not fetched (run tools/fetch-references/fetch.sh fetch)");
            return;
        }
        let r = root();
        let grammars = Grammars {
            kerml: read_allowlisted(&r, KERML_XTEXT).unwrap(),
            sysml: read_allowlisted(&r, SYSML_XTEXT).unwrap(),
            expressions: read_allowlisted(&r, EXPR_XTEXT).unwrap(),
        };
        let ir = xtext_ir::rule_ir(&grammars, TU_GRAMMAR, TU_RULE).expect("TransitionUsage IR");
        assert!(matches!(ir.expression, IrNode::Sequence { .. }));
        assert_eq!(ir.unknown_count, 0);
        assert!(!ir.dependencies.is_empty());
        let universe = grammars.rule_name_universe();
        assert!(
            xtext_ir::dangling_dependencies(&ir, &universe).is_empty(),
            "all TransitionUsage rule refs must resolve"
        );
    }

    /// AC4: exactly one card minted; helpers classify as structural helpers.
    #[test]
    fn ac4_concept_classification_and_id() {
        if !generation_sources_present(&root()) {
            eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
            return;
        }
        assert_eq!(concepts::classify_rule(TU_RULE), Classification::UserFacing);
        assert_eq!(concepts::mint_concept_id("sysml", "behavior", TU_RULE), TU_CARD_ID);
        assert_eq!(
            concepts::classify_rule("TransitionSourceMember"),
            Classification::StructuralHelper
        );
        let pack = generate(&root()).unwrap();
        // Pilot rows + two authority-scoped cards per split pair + the
        // check-grouped validation-facet cards (count not fixed here).
        let min_cards = pilot::pilot_cards().len() + pilot::split_pairs().len() * 2;
        assert!(pack.cards.len() >= min_cards, "at least pilot + split cards");
        assert!(pack.cards.len() >= 20, "at least the 20-card pilot spread");
        // No duplicate IDs (the generate() gate enforces this, checked here too).
        let mut ids: Vec<&str> = pack.cards.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(unique, ids.len(), "card ids must be unique");
        // Exactly one card per concept id; helpers never get their own card.
        assert_eq!(
            pack.cards.iter().filter(|c| c.id == TU_CARD_ID).count(),
            1
        );
        assert!(transition_card(&pack)
            .rule_dependencies
            .contains(&"TransitionSourceMember".to_owned()));
    }

    /// AC5: clause resolves; anchor is the kebab-cased heading title.
    #[test]
    fn ac5_citation_resolves() {
        if !derived_spec_text_present(&root()) {
            eprintln!("SKIP: derived spec text absent (run cargo run -p spec-index)");
            return;
        }
        let r = root();
        let spec = read_allowlisted(&r, SYSML_SPEC_TXT).unwrap();
        let index = citations::heading_index(&spec);
        let (clause, anchor) = citations::resolve_clause(&index, TU_CLAUSE).unwrap();
        assert_eq!(clause, TU_CLAUSE);
        assert_eq!(anchor, "transitionusage");
        // A bogus clause is a hard error.
        assert!(matches!(
            citations::resolve_clause(&index, "99.99.99").unwrap_err(),
            LpError::ClauseNotFound(_)
        ));
    }

    /// AC6: positive + negative examples validate against example.schema.json.
    #[test]
    fn ac6_examples_schema_valid() {
        if !generation_sources_present(&root()) {
            eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
            return;
        }
        let r = root();
        let pack = generate(&r).unwrap();
        let schemas = schema::SchemaSet::load(&r).unwrap();
        assert!(pack.examples.len() >= 40, "20 cards -> >=40 examples");
        for ex in &pack.examples {
            let value = export::to_value(ex).unwrap();
            schemas
                .validate("example.schema.json", &value)
                .unwrap_or_else(|e| panic!("example {} invalid: {e:?}", ex.id));
        }
    }

    /// AC7: no axis is `validated` without a matching-commit pass; a stale
    /// commit downgrades to `unknown`; `execute` (no evidence) is `unknown`.
    #[test]
    fn ac7_support_derivation() {
        use support::EvidenceRecord;
        let card = TU_CARD_ID;
        let commit = "abc1234";
        let mk = |axis: &str, result: &str, c: &str| EvidenceRecord {
            card_id: card.to_owned(),
            axis: axis.to_owned(),
            commit: c.to_owned(),
            gate: "sysml-spec-tests::x".to_owned(),
            case_id: "case".to_owned(),
            result: result.to_owned(),
            observed_kinds: Vec::new(),
            known_gap_ref: None,
            justifies: None,
        };
        // No evidence -> everything unknown, including execute.
        let none = support::derive_axes(card, commit, &[]);
        assert_eq!(none, support::SupportAxes::all_unknown());

        // A matching-commit pass -> validated; a stale-commit pass -> unknown.
        let ev = vec![
            mk("parse", "pass", commit),
            mk("lower", "pass", "deadbeef"), // stale commit
        ];
        let axes = support::derive_axes(card, commit, &ev);
        assert_eq!(axes.parse, "validated");
        assert_eq!(axes.lower, "unknown", "stale-commit evidence must not validate");
        assert_eq!(axes.execute, "unknown");

        // The assertions above are pure and always run. The rest generates a
        // pack, so it needs the generation inputs (fetched sources plus the
        // derived artifacts, which are never committed).
        if !generation_sources_present(&root()) {
            eprintln!("SKIP: generation sources absent (fetch references, then cargo run -p spec-index)");
            return;
        }

        // With no evidence at all, every pilot card carries honest all-unknown
        // support (generate() reads the committed evidence.jsonl, so drive the
        // no-evidence case explicitly).
        let pack = generate_with_evidence(&root(), &[]).unwrap();
        assert_eq!(
            transition_card(&pack).support,
            support::SupportAxes::all_unknown()
        );
        // And the real, committed pack derives non-trivial support (parse/lower
        // validated for transition-usage from the executable evidence gate).
        let real = generate(&root()).unwrap();
        assert_eq!(transition_card(&real).support.parse, "validated");
        assert_eq!(transition_card(&real).support.lower, "validated");
    }

    /// AC8: the card validates against language-card.schema.json, and two clean
    /// runs produce an identical tree hash.
    #[test]
    fn ac8_card_valid_and_export_deterministic() {
        if !generation_sources_present(&root()) {
            eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
            return;
        }
        let r = root();
        let pack = generate(&r).unwrap();
        validate_pack(&r, &pack).expect("card must be schema-valid");

        let tmp = std::env::temp_dir().join(format!("lp-slice-{}", std::process::id()));
        let a = tmp.join("run-a/language-pack");
        let b = tmp.join("run-b/language-pack");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        let ha = run(&r, &a).unwrap();
        let hb = run(&r, &b).unwrap();
        assert_eq!(ha, hb, "two clean runs must produce identical tree hashes");
        // The card file exists and round-trips as JSON.
        let card_path = a.join(format!("cards/{TU_CARD_ID}.json"));
        assert!(card_path.exists(), "exported card missing");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
