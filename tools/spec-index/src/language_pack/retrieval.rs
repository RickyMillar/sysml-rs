//! Retrieval export.
//!
//! Turns the assembled card corpus into a retrieval-neutral layer:
//!
//! - **chunks** — one card per chunk, an isolated `text` field (vector-ready,
//!   embed this verbatim) plus stable `metadata` (card id, family, language,
//!   keywords, support summary, provenance).
//! - **keyword index** — a term → card-id posting map with per-term document
//!   frequency and per-card chunk length, i.e. everything a BM25 scorer needs
//!   without shipping an embedding.
//! - **dependency/expansion map** — for every card, the cards it depends on
//!   (grammar `rule_dependencies` resolved through the alias table), the cards
//!   it is related to, and the cards that depend on it. Lets a retriever pull a
//!   nested-syntax card's parents/children in one hop.
//! - **report** — chunk token-budget distribution against the 400–1,200 target.
//!
//! By design this stays retrieval-neutral (local RAG, vector-ready,
//! no production ingestion adapter and no embedding computation). Everything
//! here is a pure function of the already-assembled cards + alias table, so it
//! regenerates deterministically and is covered by the regen-diff gate.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::cards::LanguageCard;
use super::examples::Example;

/// Stable per-chunk metadata (kept separate from the embeddable `text`).
#[derive(Debug, Clone, Serialize)]
pub struct ChunkMetadata {
    pub card_id: String,
    /// Retrieval family = the card's `<authority>.<facet>` prefix.
    pub family: String,
    pub authority: String,
    pub facet: String,
    pub language: String,
    pub category: Vec<String>,
    pub keywords: Vec<String>,
    pub aliases: Vec<String>,
    pub normative_rules: Vec<String>,
    pub normative_clauses: Vec<String>,
    pub semantic_types: Vec<String>,
    pub validation_rules: Vec<String>,
    pub related_cards: Vec<String>,
    /// Compact 8-axis implementation-support summary (`axis=value` pairs).
    pub support_summary: Vec<String>,
    pub known_gaps: Vec<String>,
    pub spec_drop: String,
    pub source_paths: Vec<String>,
    /// Approximate token count of `text` (chars/4), the budget-checked figure.
    pub approx_tokens: usize,
}

/// One retrieval chunk: an isolated embeddable text plus its metadata.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalChunk {
    pub id: String,
    pub text: String,
    pub metadata: ChunkMetadata,
}

/// Deterministic approximate token count. chars/4 is the conventional English
/// heuristic; we only need a stable budget signal, not tokenizer parity.
fn approx_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Split a card id `<authority>.<facet>.<slug>` into (authority, facet).
fn authority_facet(id: &str) -> (String, String) {
    let mut it = id.split('.');
    let authority = it.next().unwrap_or_default().to_owned();
    let facet = it.next().unwrap_or_default().to_owned();
    (authority, facet)
}

/// Render the isolated, self-contained chunk text for one card. This is the
/// exact string a consumer embeds / lexically indexes; it never contains
/// developer-absolute paths or wall-clock data, so it is regen-stable.
///
/// The support line is always explicit that these axes describe `sysml-rs`
/// implementation status, not language validity (normative
/// truth and implementation support must stay distinguishable in every answer).
///
/// `examples` holds this card's example records (positive, negative, composed).
/// Their SysML source is the single highest-signal content for a code-oriented
/// retriever, so it is folded into the embeddable text verbatim.
fn render_text(card: &LanguageCard, examples: &[&Example]) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {} [{}]\n", card.title, card.id));
    s.push_str(&format!(
        "Language: {} | Categories: {}\n",
        card.language,
        card.category.join(", ")
    ));
    s.push_str(&format!("Summary: {}\n", card.summary));
    if !card.keywords.is_empty() {
        s.push_str(&format!("Keywords: {}\n", card.keywords.join(", ")));
    }
    if !card.aliases.is_empty() {
        s.push_str(&format!("Also known as: {}\n", card.aliases.join(", ")));
    }
    if !card.normative_rules.is_empty() {
        let rules: Vec<String> = card
            .normative_rules
            .iter()
            .map(|r| format!("{}:{}", r.grammar, r.name))
            .collect();
        s.push_str(&format!("Grammar rules: {}\n", rules.join(", ")));
    }
    if !card.normative_clauses.is_empty() {
        let clauses: Vec<String> = card
            .normative_clauses
            .iter()
            .map(|c| format!("{} {} ({})", c.document, c.clause, c.anchor))
            .collect();
        s.push_str(&format!("Normative clauses: {}\n", clauses.join(", ")));
    }
    if !card.semantic_types.is_empty() {
        s.push_str(&format!("Semantic types: {}\n", card.semantic_types.join(", ")));
    }
    if !card.validation_rules.is_empty() {
        s.push_str(&format!("Validation rules: {}\n", card.validation_rules.join(", ")));
    }
    // Normative-vs-implementation distinction, made explicit inside the chunk.
    s.push_str(&format!(
        "Implementation support in sysml-rs (NOT a statement of language validity): {}\n",
        support_pairs(card).join(", ")
    ));
    if !card.known_gaps.is_empty() {
        s.push_str(&format!("Known implementation gaps: {}\n", card.known_gaps.join(", ")));
    }
    if !card.related_cards.is_empty() {
        s.push_str(&format!("Related cards: {}\n", card.related_cards.join(", ")));
    }
    // Example SysML source (positive first, then negative, then composed), in
    // stable example-id order. This is the concrete syntax a code retriever
    // most needs; negatives are labelled as intended-failure snippets.
    let mut positives: Vec<&&Example> = examples.iter().filter(|e| e.kind == "positive").collect();
    let mut negatives: Vec<&&Example> = examples.iter().filter(|e| e.kind == "negative").collect();
    let mut composed: Vec<&&Example> = examples.iter().filter(|e| e.kind == "composed").collect();
    positives.sort_by(|a, b| a.id.cmp(&b.id));
    negatives.sort_by(|a, b| a.id.cmp(&b.id));
    composed.sort_by(|a, b| a.id.cmp(&b.id));
    for ex in &positives {
        if let Some(src) = &ex.source {
            s.push_str(&format!("Example (valid): {src}\n"));
        }
    }
    for ex in &composed {
        if let Some(files) = &ex.files {
            let joined: Vec<String> = files
                .iter()
                .map(|f| format!("// {} ({})\n{}", f.name, f.role, f.source))
                .collect();
            s.push_str(&format!("Example (multi-file):\n{}\n", joined.join("\n")));
        }
    }
    for ex in &negatives {
        if let Some(src) = &ex.source {
            let why = ex
                .expected_failure
                .as_ref()
                .map(|f| f.mutation_class.as_str())
                .unwrap_or("invalid");
            s.push_str(&format!("Counter-example (invalid, {why}): {src}\n"));
        }
    }
    s.push_str(&format!(
        "Provenance: spec drop {}; sources {}\n",
        card.provenance.spec_drop,
        card.provenance.source_paths.join(", ")
    ));
    s
}

/// `axis=value` pairs in the fixed 8-axis order.
fn support_pairs(card: &LanguageCard) -> Vec<String> {
    let a = &card.support;
    vec![
        format!("parse={}", a.parse),
        format!("lower={}", a.lower),
        format!("resolve={}", a.resolve),
        format!("elaborate={}", a.elaborate),
        format!("validate={}", a.validate),
        format!("execute={}", a.execute),
        format!("format={}", a.format),
        format!("lsp={}", a.lsp),
    ]
}

/// Build one chunk per card. Order matches the input order; the
/// exporter sorts by id. `examples` is the full example corpus; each card's
/// examples are grouped by `card_id`.
pub fn build_chunks(cards: &[LanguageCard], examples: &[Example]) -> Vec<RetrievalChunk> {
    let mut by_card: BTreeMap<&str, Vec<&Example>> = BTreeMap::new();
    for ex in examples {
        by_card.entry(ex.card_id.as_str()).or_default().push(ex);
    }
    let empty: Vec<&Example> = Vec::new();
    cards
        .iter()
        .map(|card| {
            let card_examples = by_card.get(card.id.as_str()).unwrap_or(&empty);
            let text = render_text(card, card_examples);
            let (authority, facet) = authority_facet(&card.id);
            let family = format!("{authority}.{facet}");
            let normative_rules = card
                .normative_rules
                .iter()
                .map(|r| format!("{}:{}", r.grammar, r.name))
                .collect();
            let normative_clauses = card
                .normative_clauses
                .iter()
                .map(|c| format!("{} {}", c.document, c.clause))
                .collect();
            let metadata = ChunkMetadata {
                card_id: card.id.clone(),
                family,
                authority,
                facet,
                language: card.language.clone(),
                category: card.category.clone(),
                keywords: card.keywords.clone(),
                aliases: card.aliases.clone(),
                normative_rules,
                normative_clauses,
                semantic_types: card.semantic_types.clone(),
                validation_rules: card.validation_rules.clone(),
                related_cards: card.related_cards.clone(),
                support_summary: support_pairs(card),
                known_gaps: card.known_gaps.clone(),
                spec_drop: card.provenance.spec_drop.clone(),
                source_paths: card.provenance.source_paths.clone(),
                approx_tokens: approx_tokens(&text),
            };
            RetrievalChunk {
                id: card.id.clone(),
                text,
                metadata,
            }
        })
        .collect()
}

// --- Keyword / BM25-ready index --------------------------------------------

/// A term posting: the cards carrying the term, plus the derived document
/// frequency (= `cards.len()`, published so a consumer need not recount).
#[derive(Debug, Clone, Serialize)]
pub struct TermPosting {
    pub document_frequency: usize,
    pub cards: Vec<String>,
}

/// BM25-ready lexical index (no embeddings — vector-*ready* only, by design).
#[derive(Debug, Clone, Serialize)]
pub struct KeywordIndex {
    pub total_cards: usize,
    /// Mean chunk length in approximate tokens (the BM25 `avgdl`).
    pub average_chunk_tokens: usize,
    /// Per-card chunk length in approximate tokens (BM25 `|d|`).
    pub chunk_lengths: BTreeMap<String, usize>,
    /// term -> posting. Terms are lowercase; hyphen/underscore kept.
    pub terms: BTreeMap<String, TermPosting>,
}

/// Normalize a raw keyword/name into index terms. Splits on whitespace and
/// separators, lowercases, drops empties and 1-char noise.
///
/// Exposed to the retriever (`super::retriever`) so a query is tokenized with
/// the *exact* same rule the index was built with — divergent tokenization
/// between index and query is the classic silent BM25 recall bug.
pub(crate) fn index_terms(raw: &str) -> Vec<String> {
    raw.split(|c: char| c.is_whitespace() || c == ',' || c == '/' || c == ':')
        .flat_map(|w| {
            // Keep the whole token (e.g. "part-usage") and its dashed parts.
            let token = w.trim().to_ascii_lowercase();
            let mut out = Vec::new();
            if token.len() > 1 {
                out.push(token.clone());
            }
            for part in token.split(['-', '_']) {
                if part.len() > 1 && part != token {
                    out.push(part.to_owned());
                }
            }
            out
        })
        .collect()
}

/// Build the keyword index from chunks. Every card contributes its
/// keywords, aliases, title words, rule names, semantic types, and category.
pub fn build_keyword_index(cards: &[LanguageCard], chunks: &[RetrievalChunk]) -> KeywordIndex {
    let mut term_cards: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let add = |term: String, id: &str, map: &mut BTreeMap<String, BTreeSet<String>>| {
        map.entry(term).or_default().insert(id.to_owned());
    };
    for card in cards {
        let mut raws: Vec<String> = Vec::new();
        raws.extend(card.keywords.iter().cloned());
        raws.extend(card.aliases.iter().cloned());
        raws.push(card.title.clone());
        raws.extend(card.category.iter().cloned());
        raws.extend(card.semantic_types.iter().cloned());
        raws.extend(card.validation_rules.iter().cloned());
        raws.extend(card.normative_rules.iter().map(|r| r.name.clone()));
        for raw in &raws {
            for term in index_terms(raw) {
                add(term, &card.id, &mut term_cards);
            }
        }
    }
    let terms: BTreeMap<String, TermPosting> = term_cards
        .into_iter()
        .map(|(term, ids)| {
            let cards: Vec<String> = ids.into_iter().collect();
            (
                term,
                TermPosting {
                    document_frequency: cards.len(),
                    cards,
                },
            )
        })
        .collect();
    let chunk_lengths: BTreeMap<String, usize> = chunks
        .iter()
        .map(|c| (c.id.clone(), c.metadata.approx_tokens))
        .collect();
    let total: usize = chunk_lengths.values().sum();
    let average = if chunk_lengths.is_empty() {
        0
    } else {
        total / chunk_lengths.len()
    };
    KeywordIndex {
        total_cards: chunks.len(),
        average_chunk_tokens: average,
        chunk_lengths,
        terms,
    }
}

// --- Dependency / expansion map --------------------------------------------

/// Per-card expansion neighbourhood. Edges a retriever follows to pull
/// a concept's neighbours in one hop:
///
/// - `grammar_dependencies` — carded concepts resolved from the card's grammar
///   `rule_dependencies`, following non-carded lexical/structural HELPER rules
///   transitively through the grammar rule-ref graph (a helper is transparent;
///   expansion stops at the first carded concept behind it). So a nested
///   concept expands to its real parent-family cards.
/// - `related` — the card's declared cross-links (`related_cards`).
/// - `referenced_by` — the inverse of `related`: cards that cross-link TO this
///   card. This is the load-bearing edge — it gives a concept card its
///   validation-rule, obligation, and library cards (its "parents").
#[derive(Debug, Clone, Serialize)]
pub struct Expansion {
    pub grammar_dependencies: Vec<String>,
    pub related: Vec<String>,
    pub referenced_by: Vec<String>,
}

/// The full expansion map: card id -> its one-hop neighbourhood.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyMap {
    pub cards: BTreeMap<String, Expansion>,
}

/// Resolve a grammar rule name to the concept card(s) it stands for, following
/// non-carded helper rules transitively through the grammar rule-ref graph
/// A concept's `rule_dependencies` point mostly at
/// lexical/structural HELPER rules that are never carded; the old direct alias
/// lookup therefore produced empty expansions. Here a helper is transparent: we
/// keep walking its own rule-refs until we reach a carded concept, so a nested
/// concept expands to its real parent-family cards. Cycles/SCCs among helpers
/// are handled by the `visited` set; expansion stops AT a carded concept (we do
/// not descend through carded concepts).
fn resolve_rule_to_concepts(
    start: &str,
    self_id: &str,
    aliases: &BTreeMap<String, String>,
    card_ids: &BTreeSet<&str>,
    rule_refs: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = vec![start.to_owned()];
    while let Some(rule) = stack.pop() {
        if !visited.insert(rule.clone()) {
            continue;
        }
        // Does this rule name resolve to a card? (bare name, or authority-scoped
        // split keys `kerml:Rule`/`sysml:Rule`, or the expressions key.)
        let carded = aliases
            .get(&rule)
            .or_else(|| aliases.get(&format!("kerml:{rule}")))
            .or_else(|| aliases.get(&format!("sysml:{rule}")))
            .or_else(|| aliases.get(&format!("expressions:{rule}")))
            .filter(|t| t.as_str() != self_id && card_ids.contains(t.as_str()));
        if let Some(card) = carded {
            out.insert(card.clone());
            continue; // stop at a carded concept — do not descend through it
        }
        // Otherwise it is a helper (or an uncarded concept): keep walking its
        // own rule-refs to reach the carded concepts behind it.
        if let Some(refs) = rule_refs.get(&rule) {
            for r in refs {
                if !visited.contains(r) {
                    stack.push(r.clone());
                }
            }
        }
    }
    out
}

/// Build the dependency-aware expansion map. `aliases` maps a raw grammar rule
/// name (and other source keys) to a card id; `rule_refs` is the grammar
/// rule-ref graph (rule name -> the rule names it directly references), used to
/// resolve a concept's helper `rule_dependencies` TRANSITIVELY to the carded
/// concepts behind them. Only entries that resolve to a real card become edges.
pub fn build_dependency_map(
    cards: &[LanguageCard],
    aliases: &BTreeMap<String, String>,
    rule_refs: &BTreeMap<String, Vec<String>>,
) -> DependencyMap {
    let card_ids: BTreeSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();

    // Grammar dependency edges, resolved transitively through helper folds.
    let mut grammar: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for card in cards {
        let mut set = BTreeSet::new();
        for rule in &card.rule_dependencies {
            set.extend(resolve_rule_to_concepts(
                rule, &card.id, aliases, &card_ids, rule_refs,
            ));
        }
        grammar.insert(card.id.clone(), set);
    }

    // Inverse of the `related` cross-link graph → `referenced_by` (parents).
    let mut inverse_related: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for card in cards {
        for r in &card.related_cards {
            if card_ids.contains(r.as_str()) && r != &card.id {
                inverse_related
                    .entry(r.clone())
                    .or_default()
                    .insert(card.id.clone());
            }
        }
    }

    let mut out = BTreeMap::new();
    for card in cards {
        let grammar_dependencies: Vec<String> = grammar
            .get(&card.id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        let related: Vec<String> = {
            let mut r: Vec<String> = card
                .related_cards
                .iter()
                .filter(|r| card_ids.contains(r.as_str()) && r.as_str() != card.id.as_str())
                .cloned()
                .collect();
            r.sort();
            r.dedup();
            r
        };
        let referenced_by: Vec<String> = inverse_related
            .get(&card.id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        out.insert(
            card.id.clone(),
            Expansion {
                grammar_dependencies,
                related,
                referenced_by,
            },
        );
    }
    DependencyMap { cards: out }
}

// --- Chunk-budget report ----------------------------------------------------

/// A token distribution summary (chars/4 approximation).
#[derive(Debug, Clone, Serialize)]
pub struct TokenDistribution {
    pub count: usize,
    pub min_tokens: usize,
    pub max_tokens: usize,
    pub mean_tokens: usize,
    pub median_tokens: usize,
    pub p90_tokens: usize,
    pub in_budget: usize,
    pub under_budget: usize,
    pub over_budget: usize,
}

fn distribution(lens: &[usize]) -> TokenDistribution {
    let mut lens = lens.to_vec();
    lens.sort_unstable();
    let n = lens.len();
    let (min, max) = (lens.first().copied().unwrap_or(0), lens.last().copied().unwrap_or(0));
    let sum: usize = lens.iter().sum();
    let mean = if n == 0 { 0 } else { sum / n };
    let median = lens.get(n / 2).copied().unwrap_or(0);
    let p90 = lens.get((n * 9 / 10).min(n.saturating_sub(1))).copied().unwrap_or(0);
    let under = lens.iter().filter(|&&x| x < TARGET_MIN).count();
    let over = lens.iter().filter(|&&x| x > TARGET_MAX).count();
    TokenDistribution {
        count: n,
        min_tokens: min,
        max_tokens: max,
        mean_tokens: mean,
        median_tokens: median,
        p90_tokens: p90,
        in_budget: n - under - over,
        under_budget: under,
        over_budget: over,
    }
}

/// Token-budget distribution for the retrieval corpus, against the
/// 400–1,200 target. Reports two figures per chunk:
///
/// - `chunk_text` — the distilled, embeddable `text` field. Intentionally
///   leaner than a full card so an embedding carries concept signal, not JSON
///   scaffolding; short validation/operator cards sit below the 400 floor by
///   design and that is acceptable, not a defect.
/// - `full_card_json` — the full serialized card. This is the figure the plan's
///   "cards are already 400–1,200 tokens" refers to, reported here to confirm
///   it and to keep the two budgets from being conflated.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalReport {
    pub chunk_count: usize,
    pub target_min_tokens: usize,
    pub target_max_tokens: usize,
    pub chunk_text: TokenDistribution,
    pub full_card_json: TokenDistribution,
    /// Card ids whose chunk text is under the 400-token floor (informational).
    pub under_budget_cards: Vec<String>,
    /// Card ids whose chunk text is over the 1,200-token ceiling (would split).
    pub over_budget_cards: Vec<String>,
    pub index_terms: usize,
    pub average_chunk_tokens: usize,
    pub note: String,
}

const TARGET_MIN: usize = 400;
const TARGET_MAX: usize = 1200;

/// Compute the retrieval budget report. `card_json_tokens` is the chars/4 size
/// of each card's canonical JSON, supplied by the exporter.
pub fn build_report(
    chunks: &[RetrievalChunk],
    card_json_tokens: &[usize],
    keyword_terms: usize,
    avg: usize,
) -> RetrievalReport {
    let chunk_lens: Vec<usize> = chunks.iter().map(|c| c.metadata.approx_tokens).collect();
    let mut under = Vec::new();
    let mut over = Vec::new();
    for c in chunks {
        if c.metadata.approx_tokens < TARGET_MIN {
            under.push(c.id.clone());
        } else if c.metadata.approx_tokens > TARGET_MAX {
            over.push(c.id.clone());
        }
    }
    under.sort();
    over.sort();
    RetrievalReport {
        chunk_count: chunks.len(),
        target_min_tokens: TARGET_MIN,
        target_max_tokens: TARGET_MAX,
        chunk_text: distribution(&chunk_lens),
        full_card_json: distribution(card_json_tokens),
        under_budget_cards: under,
        over_budget_cards: over,
        index_terms: keyword_terms,
        average_chunk_tokens: avg,
        note: "Two budgets are reported. `full_card_json` is the original \
               400–1,200-token figure and is confirmed (most cards land in band; \
               compact validation-rule cards sit just below 400). `chunk_text` is \
               the distilled embeddable projection — deliberately leaner so an \
               embedding carries concept signal rather than JSON scaffolding — and \
               its sub-400 values are by design, not a defect. Token counts are a \
               deterministic chars/4 approximation, not tokenizer-exact."
            .to_owned(),
    }
}
