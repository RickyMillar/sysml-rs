//! Deterministic lexical (BM25) retriever over the shipped retrieval layer
//! (a deterministic regression floor over the shipped index).
//!
//! This is the missing evaluation primitive: the existing `evals/` gate proves
//! the *reference answers* are correct, but nothing exercised retrieval itself.
//! Here a dependency-light BM25 scorer runs over the already-shipped
//! `indexes/keywords.json` (term postings + per-card chunk length + `avgdl`)
//! and `indexes/dependencies.json` (one-hop expansion), so an eval can measure
//! whether the pack surfaces the right card for a query.
//!
//! Design constraints (all load-bearing for a regression floor):
//!
//! - **Deterministic.** Query terms are tokenized with the identical rule the
//!   index was built with (`retrieval::index_terms`); scores accumulate in a
//!   fixed (sorted term) order; ranking sorts by score then card-id, so equal
//!   scores never reorder nondeterministically across platforms.
//! - **Dependency-light.** Reads only the two committed JSON index files — no
//!   embeddings, no parser, no float-ordering surprises. The keyword index is a
//!   binary posting (a card either carries a term or not), so term frequency is
//!   1 for a present term; BM25's length normalization still discriminates.
//! - **Neutral.** The retriever is a *reader* of the shipped index; it never
//!   re-derives from cards, so it measures exactly what a consumer would embed
//!   or index.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::LpError;

// BM25 free parameters. The Robertson/Spärck-Jones defaults; k1 in [1.2,2.0],
// b=0.75 are the conventional starting points and we have no corpus-specific
// reason to move them.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// One term posting as shipped in `keywords.json`.
#[derive(Debug, Clone, Deserialize)]
struct TermPosting {
    document_frequency: usize,
    cards: Vec<String>,
}

/// The shipped `keywords.json` shape (read-only view; the writer lives in
/// `retrieval::KeywordIndex`).
#[derive(Debug, Clone, Deserialize)]
struct KeywordIndexFile {
    total_cards: usize,
    average_chunk_tokens: usize,
    chunk_lengths: BTreeMap<String, usize>,
    terms: BTreeMap<String, TermPosting>,
}

/// One card's one-hop neighbourhood as shipped in `dependencies.json`.
#[derive(Debug, Clone, Deserialize, Default)]
struct ExpansionFile {
    #[serde(default)]
    grammar_dependencies: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
    #[serde(default)]
    referenced_by: Vec<String>,
}

/// The shipped `dependencies.json` shape (read-only view).
#[derive(Debug, Clone, Deserialize)]
struct DependencyFile {
    cards: BTreeMap<String, ExpansionFile>,
}

/// A scored retrieval hit.
#[derive(Debug, Clone)]
pub struct Scored {
    pub card_id: String,
    pub score: f64,
}

/// Deterministic BM25 retriever over the shipped retrieval index.
#[derive(Debug, Clone)]
pub struct Retriever {
    n: f64,
    avgdl: f64,
    doc_len: BTreeMap<String, f64>,
    terms: BTreeMap<String, TermPosting>,
    expansion: BTreeMap<String, ExpansionFile>,
    all_cards: BTreeSet<String>,
}

impl Retriever {
    /// Load the retriever from a shipped pack directory (`keywords.json` +
    /// `dependencies.json`). Both are required; a missing/garbled file is a
    /// hard error (fail-hard, not silent empty index).
    pub fn load(pack_dir: &Path) -> Result<Self, LpError> {
        let kw_path = pack_dir.join("indexes/keywords.json");
        let kw_text = std::fs::read_to_string(&kw_path)
            .map_err(|e| LpError::Io(format!("read {}: {e}", kw_path.display())))?;
        let kw: KeywordIndexFile = serde_json::from_str(&kw_text)
            .map_err(|e| LpError::Other(format!("parse {}: {e}", kw_path.display())))?;

        let dep_path = pack_dir.join("indexes/dependencies.json");
        let dep_text = std::fs::read_to_string(&dep_path)
            .map_err(|e| LpError::Io(format!("read {}: {e}", dep_path.display())))?;
        let dep: DependencyFile = serde_json::from_str(&dep_text)
            .map_err(|e| LpError::Other(format!("parse {}: {e}", dep_path.display())))?;

        let avgdl = if kw.average_chunk_tokens == 0 {
            1.0
        } else {
            kw.average_chunk_tokens as f64
        };
        let doc_len: BTreeMap<String, f64> = kw
            .chunk_lengths
            .iter()
            .map(|(k, v)| (k.clone(), (*v).max(1) as f64))
            .collect();
        let all_cards: BTreeSet<String> = kw.chunk_lengths.keys().cloned().collect();
        Ok(Self {
            n: (kw.total_cards.max(1)) as f64,
            avgdl,
            doc_len,
            terms: kw.terms,
            expansion: dep.cards,
            all_cards,
        })
    }

    /// Number of indexed cards.
    pub fn card_count(&self) -> usize {
        self.all_cards.len()
    }

    /// Inverse document frequency (BM25 "probabilistic" form, always positive).
    fn idf(&self, df: usize) -> f64 {
        let df = df as f64;
        (1.0 + (self.n - df + 0.5) / (df + 0.5)).ln()
    }

    /// Score every card that shares at least one term with `query`. Ranking is
    /// deterministic: scores accumulate in sorted-term order, then results sort
    /// by descending score with an ascending card-id tiebreak.
    pub fn score(&self, query: &str) -> Vec<Scored> {
        // Deduplicated, sorted query terms → a fixed accumulation order.
        let mut qterms: Vec<String> = super::retrieval::index_terms(query);
        qterms.sort();
        qterms.dedup();

        let mut scores: BTreeMap<String, f64> = BTreeMap::new();
        for term in &qterms {
            let Some(posting) = self.terms.get(term) else {
                continue;
            };
            let idf = self.idf(posting.document_frequency);
            for card in &posting.cards {
                let dl = self.doc_len.get(card).copied().unwrap_or(self.avgdl);
                // Binary term frequency (tf = 1 for a present term).
                let denom = 1.0 + K1 * (1.0 - B + B * dl / self.avgdl);
                let contribution = idf * (K1 + 1.0) / denom;
                *scores.entry(card.clone()).or_insert(0.0) += contribution;
            }
        }

        let mut ranked: Vec<Scored> = scores
            .into_iter()
            .map(|(card_id, score)| Scored { card_id, score })
            .collect();
        // Descending score; ascending card-id tiebreak (stable, platform-free).
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        ranked
    }

    /// Top-`k` card ids for `query` (BM25 only, no expansion).
    pub fn retrieve(&self, query: &str, k: usize) -> Vec<String> {
        self.score(query)
            .into_iter()
            .take(k)
            .map(|s| s.card_id)
            .collect()
    }

    /// One-hop neighbours of `seed` cards (grammar dependencies ∪ related ∪
    /// referenced_by), excluding the seeds themselves. Order is deterministic:
    /// seeds in input order, each seed's neighbours in the fixed
    /// grammar→related→referenced_by, sorted-within-group order the index
    /// ships; duplicates dropped on first sight.
    pub fn expand_once(&self, seed: &[String]) -> Vec<String> {
        let seed_set: BTreeSet<&str> = seed.iter().map(String::as_str).collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out: Vec<String> = Vec::new();
        for s in seed {
            let Some(exp) = self.expansion.get(s) else {
                continue;
            };
            for group in [&exp.grammar_dependencies, &exp.related, &exp.referenced_by] {
                for card in group {
                    if seed_set.contains(card.as_str()) {
                        continue;
                    }
                    if seen.insert(card.clone()) {
                        out.push(card.clone());
                    }
                }
            }
        }
        out
    }

    /// Top-`k` BM25 hits followed by their one-hop neighbours (deduped, base
    /// hits first). This is the *candidate set* a dependency-expanding retriever
    /// would consider — used to measure expansion's recall contribution.
    pub fn retrieve_expanded(&self, query: &str, k: usize) -> Vec<String> {
        let base = self.retrieve(query, k);
        let neighbours = self.expand_once(&base);
        let mut out = base;
        let present: BTreeSet<String> = out.iter().cloned().collect();
        for card in neighbours {
            if !present.contains(&card) {
                out.push(card);
            }
        }
        out
    }

    /// True when `card_id` is a real card in the shipped index.
    pub fn contains_card(&self, card_id: &str) -> bool {
        self.all_cards.contains(card_id)
    }
}

// --- Retrieval metrics (pure functions over expected vs. ranked) -----------

/// Per-query retrieval metrics against an answer key.
#[derive(Debug, Clone)]
pub struct QueryMetrics {
    /// Number of expected cards found within the top-`k`.
    pub found_at_k: usize,
    /// Total expected cards for the query.
    pub expected: usize,
    /// `found_at_k / expected`.
    pub recall_at_k: f64,
    /// True when every expected card is within the top-`k`.
    pub all_found: bool,
    /// 1-based rank of the first expected card in the full ranked list, if any.
    pub first_rank: Option<usize>,
}

impl QueryMetrics {
    /// Reciprocal rank of the first expected hit (0 when none found).
    pub fn reciprocal_rank(&self) -> f64 {
        self.first_rank.map_or(0.0, |r| 1.0 / r as f64)
    }
}

/// Evaluate a ranked card list against an expected answer key at cutoff `k`.
pub fn evaluate(ranked: &[String], expected: &[String], k: usize) -> QueryMetrics {
    let expected_set: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let topk: BTreeSet<&str> = ranked.iter().take(k).map(String::as_str).collect();
    let found_at_k = expected_set.iter().filter(|e| topk.contains(*e)).count();
    let first_rank = ranked
        .iter()
        .position(|c| expected_set.contains(c.as_str()))
        .map(|p| p + 1);
    let expected_n = expected.len();
    QueryMetrics {
        found_at_k,
        expected: expected_n,
        recall_at_k: if expected_n == 0 {
            0.0
        } else {
            found_at_k as f64 / expected_n as f64
        },
        all_found: expected_n > 0 && found_at_k == expected_n,
        first_rank,
    }
}

/// Recall of `expected` within a candidate *set* (membership only, no ranking).
/// Used to measure the dependency-expansion contribution, where the expanded
/// candidates form a set rather than a meaningful ranking.
pub fn set_recall(candidates: &[String], expected: &[String]) -> (usize, usize) {
    let cand: BTreeSet<&str> = candidates.iter().map(String::as_str).collect();
    let found = expected.iter().filter(|e| cand.contains(e.as_str())).count();
    (found, expected.len())
}

// --- Held-out retrieval query set -------------------------------------------

/// One held-out retrieval query with its expected-card answer key. Authored
/// data (not derived from the index), spanning every major family, so the
/// deterministic retrieval floor exercises real discrimination rather than a
/// re-projection of the pack. The `sysml-spec-tests::retrieval_eval` gate proves
/// every `expected_card_ids` entry is a real card and measures recall/MRR.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalQuery {
    pub id: String,
    pub query: String,
    /// Card ids a correct retrieval must surface (all must exist in the pack).
    pub expected_card_ids: Vec<String>,
    /// Coarse family tag, used to report per-family recall.
    pub family: String,
}

fn q(id: &str, family: &str, query: &str, expected: &[&str]) -> RetrievalQuery {
    RetrievalQuery {
        id: id.to_owned(),
        query: query.to_owned(),
        expected_card_ids: expected.iter().map(|s| (*s).to_owned()).collect(),
        family: family.to_owned(),
    }
}

/// The full authored query set. Queries use domain vocabulary (the terms a
/// modeller would actually type); the floor is pinned *below* the observed
/// recall, so occasional misses are expected and keep the eval honest.
pub fn retrieval_queries() -> Vec<RetrievalQuery> {
    vec![
        // --- KerML core / relationships --------------------------------
        q("kc.type", "kerml-core", "What is the base Type in KerML that can specialize and classify?", &["kerml.structure.type"]),
        q("kc.classifier", "kerml-core", "KerML classifier that classifies instances", &["kerml.structure.classifier"]),
        q("kc.specialization", "kerml-core", "how one type specializes another with specialization", &["kerml.structure.specialization"]),
        q("kc.subsetting", "kerml-core", "subsetting a feature that subsets another", &["kerml.structure.subsetting"]),
        q("kc.redefinition", "kerml-core", "redefinition that redefines an inherited feature", &["kerml.structure.redefinition"]),
        q("kc.feature-typing", "kerml-core", "typing a feature so it is typed by a type", &["kerml.structure.feature-typing", "sysml.structure.feature-typing"]),
        q("kc.multiplicity", "kerml-core", "multiplicity bound and cardinality on a feature", &["kerml.structure.multiplicity"]),
        q("kc.namespace", "kerml-core", "namespace scope holding members", &["kerml.structure.namespace"]),
        q("kc.package", "kerml-core", "package grouping members in a namespace", &["kerml.structure.package"]),
        q("kc.import", "kerml-core", "import to make another namespace's members visible", &["kerml.structure.import"]),
        q("kc.association", "kerml-core", "association linking two ends", &["kerml.structure.association"]),
        q("kc.connector", "kerml-core", "connector connecting from one feature to another", &["kerml.structure.connector"]),
        q("kc.structure", "kerml-core", "structure with composition", &["kerml.structure.structure"]),
        q("kc.datatype", "kerml-core", "data type carrying a data value", &["kerml.structure.data-type"]),
        q("kc.class", "kerml-core", "class as an occurrence with identity", &["kerml.structure.class"]),
        // --- Expressions ----------------------------------------------
        q("ex.literal-integer", "expressions", "integer literal number in an expression", &["kerml.expression.literal-integer"]),
        q("ex.literal-string", "expressions", "string literal text value", &["kerml.expression.literal-string"]),
        q("ex.literal-boolean", "expressions", "boolean literal true or false", &["kerml.expression.literal-boolean"]),
        q("ex.additive", "expressions", "additive operator for plus and minus", &["kerml.expression.additive-operator"]),
        q("ex.multiplicative", "expressions", "multiplicative operator that multiplies", &["kerml.expression.multiplicative-operator"]),
        q("ex.relational", "expressions", "relational operator less than greater than", &["kerml.expression.relational-operator"]),
        q("ex.conditional", "expressions", "conditional if operator in an expression", &["kerml.expression.conditional-operator"]),
        q("ex.invocation", "expressions", "invocation expression with arguments", &["kerml.expression.invocation"]),
        q("ex.feature-reference", "expressions", "feature reference expression", &["kerml.expression.feature-reference"]),
        // --- Action nodes ---------------------------------------------
        q("an.action-def", "action-nodes", "how do you write an action def", &["sysml.behavior.action-definition"]),
        q("an.accept", "action-nodes", "accept node that receives a payload", &["sysml.behavior.accept-node"]),
        q("an.send", "action-nodes", "send node dispatching to a target", &["sysml.behavior.send-node"]),
        q("an.decision", "action-nodes", "decision node with a guard", &["sysml.behavior.decision-node"]),
        q("an.fork", "action-nodes", "fork node for concurrent control flow", &["sysml.behavior.fork-node"]),
        q("an.join", "action-nodes", "join node synchronizing control flow", &["sysml.behavior.join-node"]),
        q("an.merge", "action-nodes", "merge node for alternative control flow", &["sysml.behavior.merge-node"]),
        q("an.for-loop", "action-nodes", "for loop node iterating over items", &["sysml.behavior.for-loop-node"]),
        q("an.while-loop", "action-nodes", "while loop node repeating until a test", &["sysml.behavior.while-loop-node"]),
        q("an.assignment", "action-nodes", "assignment node that assigns a value", &["sysml.behavior.assignment-node"]),
        q("an.perform", "action-nodes", "perform action that invokes a step", &["sysml.behavior.perform-action"]),
        // --- States ---------------------------------------------------
        q("st.state-def", "states", "declaring a state def machine", &["sysml.behavior.state-definition"]),
        q("st.transition", "states", "state transition with a trigger, guard, first and then", &["sysml.behavior.transition-usage"]),
        q("st.exhibit", "states", "exhibit a state behavior in a part", &["sysml.behavior.exhibit-state"]),
        q("st.succession", "states", "succession ordering with first then", &["sysml.behavior.succession"]),
        // --- Requirements / cases -------------------------------------
        q("rq.requirement-def", "requirements-cases", "requirement def with a subject and a constraint", &["sysml.requirements.requirement-definition"]),
        q("rq.subject", "requirements-cases", "subject under consideration of a requirement", &["sysml.requirements.subject"]),
        q("rq.actor", "requirements-cases", "actor as an external role", &["sysml.requirements.actor"]),
        q("rq.stakeholder", "requirements-cases", "stakeholder with an interest in a concern", &["sysml.requirements.stakeholder"]),
        q("rq.concern-def", "requirements-cases", "concern def for a stakeholder", &["sysml.requirements.concern-definition"]),
        q("rq.constraint-def", "requirements-cases", "constraint def with a boolean condition", &["sysml.requirements.constraint-definition"]),
        q("rq.satisfaction", "requirements-cases", "satisfy a requirement by a part", &["sysml.requirements.satisfaction"]),
        q("cs.verification-case", "requirements-cases", "verification case that verifies a requirement and yields a verdict", &["sysml.cases.verification-case"]),
        q("cs.analysis-case", "requirements-cases", "analysis case producing a result", &["sysml.cases.analysis-case"]),
        q("cs.use-case-def", "requirements-cases", "use case def with an actor", &["sysml.cases.use-case-definition"]),
        q("cs.objective", "requirements-cases", "objective goal of a case", &["sysml.cases.objective"]),
        // --- Views ----------------------------------------------------
        q("vw.view-def", "views", "view def that renders a viewpoint", &["sysml.views.view-definition"]),
        q("vw.viewpoint", "views", "viewpoint def framing a concern", &["sysml.views.viewpoint-definition"]),
        q("vw.rendering", "views", "rendering def notation", &["sysml.views.rendering-definition"]),
        q("vw.expose", "views", "expose members into a view", &["sysml.views.membership-expose", "sysml.views.namespace-expose"]),
        // --- Metamodel semantics --------------------------------------
        q("mm.metaclass", "metamodel", "metaclass used for metadata", &["kerml.structure.metaclass"]),
        q("mm.metadata-def", "metamodel", "metadata def annotation", &["sysml.metadata.metadata-definition"]),
        q("mm.comment", "metamodel", "comment note about an element", &["kerml.metadata.comment"]),
        q("mm.documentation", "metamodel", "documentation description of an element", &["kerml.metadata.documentation"]),
        q("mm.membership", "metamodel", "membership metamodel relationship", &["kerml.structure.membership"]),
        q("mm.subject-first-param", "metamodel", "rule that a requirement subject is the first parameter", &["sysml.validation.subject-is-first-parameter", "sysml.validation.requirement-subject-must-be-first-parameter"]),
        // --- Standard library -----------------------------------------
        q("sl.verdict-kind", "stdlib", "verdict kind values pass fail inconclusive error", &["sysml.library.verdict-kind"]),
        q("sl.constraint-check", "stdlib", "ConstraintCheck library boolean evaluation", &["sysml.library.constraint-check"]),
        q("sl.requirement-check", "stdlib", "RequirementCheck library with assumptions and constraints", &["sysml.library.requirement-check"]),
        q("sl.calculation-lib", "stdlib", "Calculation library base for calculations", &["sysml.library.calculation"]),
        q("sl.pass-if", "stdlib", "PassIf library helper returning a verdict", &["sysml.library.pass-if"]),
        q("sl.state-action", "stdlib", "StateAction library state sequencing", &["sysml.library.state-action"]),
        q("sl.boolean-evaluation", "stdlib", "BooleanEvaluation kernel library predicate performance", &["kerml.library.boolean-evaluation"]),
    ]
}

/// Serialize the query set to JSONL, one record per line, sorted by id
/// (deterministic — same discipline as `evals::export_evals`).
pub fn export_retrieval_queries() -> Result<String, LpError> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for query in retrieval_queries() {
        let json = serde_json::to_string(&query)
            .map_err(|e| LpError::Other(format!("retrieval query serialize: {e}")))?;
        rows.push((query.id.clone(), json));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (_, json) in rows {
        out.push_str(&json);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn tiny() -> Retriever {
        let mut terms = BTreeMap::new();
        terms.insert(
            "part".to_owned(),
            TermPosting {
                document_frequency: 1,
                cards: vec!["sysml.structure.part-usage".to_owned()],
            },
        );
        terms.insert(
            "connect".to_owned(),
            TermPosting {
                document_frequency: 1,
                cards: vec!["sysml.structure.connection-usage".to_owned()],
            },
        );
        let mut doc_len = BTreeMap::new();
        doc_len.insert("sysml.structure.part-usage".to_owned(), 100.0);
        doc_len.insert("sysml.structure.connection-usage".to_owned(), 100.0);
        let mut expansion = BTreeMap::new();
        expansion.insert(
            "sysml.structure.connection-usage".to_owned(),
            ExpansionFile {
                grammar_dependencies: vec!["sysml.structure.part-usage".to_owned()],
                related: vec![],
                referenced_by: vec![],
            },
        );
        Retriever {
            n: 2.0,
            avgdl: 100.0,
            doc_len,
            terms,
            expansion,
            all_cards: [
                "sysml.structure.part-usage".to_owned(),
                "sysml.structure.connection-usage".to_owned(),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn scores_matching_card() {
        let r = tiny();
        let hits = r.retrieve("connect two part", 5);
        assert!(hits.contains(&"sysml.structure.connection-usage".to_owned()));
        assert!(hits.contains(&"sysml.structure.part-usage".to_owned()));
    }

    #[test]
    fn ranking_is_deterministic_across_runs() {
        let r = tiny();
        let a = r.retrieve("part connect", 5);
        let b = r.retrieve("part connect", 5);
        assert_eq!(a, b);
    }

    #[test]
    fn expansion_pulls_one_hop() {
        let r = tiny();
        let seed = vec!["sysml.structure.connection-usage".to_owned()];
        let nbr = r.expand_once(&seed);
        assert_eq!(nbr, vec!["sysml.structure.part-usage".to_owned()]);
    }

    #[test]
    fn metrics_recall_and_rank() {
        let ranked = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let m = evaluate(&ranked, &["b".to_owned(), "z".to_owned()], 3);
        assert_eq!(m.found_at_k, 1);
        assert_eq!(m.expected, 2);
        assert_eq!(m.first_rank, Some(2));
        assert!((m.reciprocal_rank() - 0.5).abs() < 1e-9);
        assert!(!m.all_found);
    }
}
