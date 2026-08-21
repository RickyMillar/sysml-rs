//! Completeness report: the
//! honest metrics. Every equation publishes a numerator AND a denominator, never
//! a bare percentage; `unknown`/`blocked`/`uncarded`/excluded concepts stay in
//! the denominator. Each metric's definition is emitted alongside it (`notes`)
//! so a reader can audit exactly what was counted.
//!
//! Metrics are computed from the COMPLETE denominator (every
//! one of the 650 grammar names plus the metamodel/SHACL/stdlib slots), keyed on
//! the record's `mapping` field — so an in-scope card-bearing concept with no
//! card (`mapping == "uncarded"`) stays in the denominator and lowers coverage.
//! The previous metric derived the denominator only from records that already
//! carried a concept id, which made coverage tautologically 100%.
//!
//! Computed purely from the in-memory pack (cards + denominator records +
//! published normalization report), so it is deterministic and regenerates in
//! lockstep with the pack.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::cards::LanguageCard;
use super::concepts::DenominatorRecord;
use super::denominator::{ConflictRecord, DenominatorReport};
use super::export::Pack;

/// A numerator/denominator pair (never a bare percentage).
#[derive(Debug, Clone, Serialize)]
pub struct Metric {
    pub numerator: usize,
    pub denominator: usize,
}

impl Metric {
    fn new(numerator: usize, denominator: usize) -> Self {
        Metric { numerator, denominator }
    }
    fn ratio_str(&self) -> String {
        if self.denominator == 0 {
            "0/0 (n/a)".to_owned()
        } else {
            format!(
                "{}/{} ({:.1}%)",
                self.numerator,
                self.denominator,
                100.0 * self.numerator as f64 / self.denominator as f64
            )
        }
    }
}

/// One excluded/blocked/uncarded concept, with the rationale + pointer the report
/// requires. `mapping` records why it is not covered.
#[derive(Debug, Clone, Serialize)]
pub struct GapRecord {
    pub source_id: String,
    pub raw_name: String,
    pub source_kind: String,
    pub classification: String,
    pub mapping: String,
    pub rationale: String,
    pub source_pointer: String,
}

/// The generated completeness artifact (`completeness.json`).
#[derive(Debug, Clone, Serialize)]
pub struct Completeness {
    pub spec_drop: String,
    // Normalization counts, before and after (auditable collapse).
    pub raw_rows: usize,
    pub unique_names: usize,
    pub cross_grammar_shared: usize,
    pub merge_count: usize,
    pub split_count: usize,
    pub conflict_count: usize,
    // The complete-denominator size and its mapping distribution (auditable).
    pub denominator_records: usize,
    pub mapping_counts: BTreeMap<String, usize>,
    // §11.4 headline metrics (all numerator/denominator).
    pub card_coverage: Metric,
    pub user_facing_syntax_coverage: Metric,
    pub carded_parse_lower_coverage: Metric,
    // §11.4 per-axis metrics over the full syntax-concept denominator.
    pub explicit_parse_coverage: Metric,
    pub generic_fallback_rate: Metric,
    pub lowering_coverage: Metric,
    pub resolution_coverage: Metric,
    pub elaboration_coverage: Metric,
    pub validation_coverage: Metric,
    pub execution_coverage: Metric,
    pub formatting_coverage: Metric,
    pub lsp_coverage: Metric,
    pub example_coverage: Metric,
    pub negative_coverage: Metric,
    pub e2e_coverage: Metric,
    // Per source-kind coverage — makes the metamodel/SHACL/stdlib gaps VISIBLE.
    pub coverage_by_source_kind: BTreeMap<String, Metric>,
    // The complete disposition of every metamodel class (182) and every
    // distinct constrained SHACL/OSLC shape (175) — the authoritative accounting
    // that replaces the review's opaque metamodel 0/182, shacl 0/257 slots.
    pub metamodel_disposition: BTreeMap<String, usize>,
    pub shacl_disposition: BTreeMap<String, usize>,
    // Card totals by family (grammar-syntax / operator / validation / …).
    pub card_count_by_family: BTreeMap<String, usize>,
    // Lists.
    pub covered: Vec<String>,
    pub unknown_support: Vec<String>,
    pub blocked_support: Vec<String>,
    pub uncarded: Vec<GapRecord>,
    pub blocked: Vec<GapRecord>,
    pub excluded: Vec<GapRecord>,
    pub conflicts: Vec<ConflictRecord>,
    pub notes: Vec<String>,
}

/// Card-bearing denominator classifications (these mint a card).
fn is_card_bearing(classification: &str) -> bool {
    DenominatorRecord::is_card_bearing_classification(classification)
}

/// Is this a grammar-syntax concept (parse/lower/resolve/…-applicable)?
fn is_syntax(rec: &DenominatorRecord) -> bool {
    rec.source_kind == "xtext"
        && matches!(rec.classification.as_str(), "user-facing" | "operator" | "expression")
}

/// A card-bearing concept that counts in the reviewed in-scope denominator: it
/// has a card (`card`), has none yet (`uncarded`), or is blocked (`block`).
/// `helper-fold`/`alias`/`duplicate`/`exclusion` do NOT count (folded away or
/// deliberately out of scope) — exclusion can never inflate coverage (§11.4).
fn is_in_scope(rec: &DenominatorRecord) -> bool {
    is_card_bearing(&rec.classification)
        && matches!(rec.mapping.as_str(), "card" | "uncarded" | "block")
}

/// Family label for a denominator record (for the by-family card totals).
fn family_of(source_kind: &str, classification: &str) -> &'static str {
    match (source_kind, classification) {
        ("xtext", "operator") => "operator",
        ("xtext", "user-facing") => "grammar-syntax",
        ("validation", _) => "validation-rule",
        ("obligation", _) => "obligation",
        ("stdlib", _) => "stdlib",
        ("metamodel", _) => "metamodel",
        _ => "other",
    }
}

/// Does a card carry all-`unknown` support on every axis?
fn all_unknown(support: &super::support::SupportAxes) -> bool {
    *support == super::support::SupportAxes::all_unknown()
}

/// The set of card ids minted for each distinct `(source_kind, raw_name)`
/// concept (a split name yields two authority-scoped cards under one raw_name).
type ConceptCards<'a> = BTreeMap<(String, String), BTreeSet<&'a str>>;

fn concept_cards<'a>(pack: &'a Pack) -> ConceptCards<'a> {
    let card_ids: BTreeSet<&str> = pack.cards.iter().map(|c| c.id.as_str()).collect();
    let mut out: ConceptCards<'a> = BTreeMap::new();
    for rec in &pack.denominator {
        let entry = out
            .entry((rec.source_kind.clone(), rec.raw_name.clone()))
            .or_default();
        if rec.mapping == "card" {
            if let Some(id) = &rec.normalized_concept_id {
                if let Some(hit) = card_ids.get(id.as_str()) {
                    entry.insert(*hit);
                }
            }
        }
    }
    out
}

/// Compute the completeness report from a fully-assembled pack.
pub fn compute(pack: &Pack, report: &DenominatorReport) -> Completeness {
    let card_by_id: BTreeMap<&str, &LanguageCard> =
        pack.cards.iter().map(|c| (c.id.as_str(), c)).collect();
    let concepts = concept_cards(pack);

    // --- mapping distribution (auditable) ---
    let mut mapping_counts: BTreeMap<String, usize> = BTreeMap::new();
    for rec in &pack.denominator {
        *mapping_counts.entry(rec.mapping.clone()).or_insert(0) += 1;
    }

    // --- card_coverage: carded / all in-scope card-bearing concepts, over the
    // FULL denominator (a `uncarded`/`block` in-scope concept lowers it). Deduped
    // by (source_kind, raw_name) so a split name counts once. ---
    let mut in_scope: BTreeSet<(String, String)> = BTreeSet::new();
    let mut in_scope_carded: BTreeSet<(String, String)> = BTreeSet::new();
    for rec in &pack.denominator {
        if !is_in_scope(rec) {
            continue;
        }
        let key = (rec.source_kind.clone(), rec.raw_name.clone());
        in_scope.insert(key.clone());
        if rec.mapping == "card" {
            in_scope_carded.insert(key);
        }
    }
    let card_coverage = Metric::new(in_scope_carded.len(), in_scope.len());

    // --- user_facing_syntax_coverage: the review's 55/305 headline. Distinct
    // user-facing grammar names with a card / all distinct user-facing names. ---
    let (mut uf_total, mut uf_carded): (BTreeSet<String>, BTreeSet<String>) =
        (BTreeSet::new(), BTreeSet::new());
    for rec in &pack.denominator {
        if rec.source_kind == "xtext"
            && rec.classification == "user-facing"
            && matches!(rec.mapping.as_str(), "card" | "uncarded")
        {
            uf_total.insert(rec.raw_name.clone());
            if rec.mapping == "card" {
                uf_carded.insert(rec.raw_name.clone());
            }
        }
    }
    let user_facing_syntax_coverage = Metric::new(uf_carded.len(), uf_total.len());

    // --- Per-axis coverage over the full syntax-concept denominator. A syntax
    // concept (user-facing/operator/expression grammar rule) is the unit; the
    // numerator counts concepts whose minted card has that axis `validated`.
    // Uncarded concepts contribute 0 — they stay in the denominator. ---
    let mut syntax_names: BTreeSet<(String, String)> = BTreeSet::new();
    for rec in &pack.denominator {
        if is_syntax(rec) && matches!(rec.mapping.as_str(), "card" | "uncarded" | "block") {
            syntax_names.insert((rec.source_kind.clone(), rec.raw_name.clone()));
        }
    }
    let syntax_total = syntax_names.len();

    // For each syntax concept, best axis value across its card(s).
    let axis_validated = |concept: &(String, String), axis: &str| -> bool {
        concepts
            .get(concept)
            .into_iter()
            .flatten()
            .filter_map(|id| card_by_id.get(id))
            .any(|c| axis_value(&c.support, axis) == "validated")
    };
    let axis_partial = |concept: &(String, String), axis: &str| -> bool {
        concepts
            .get(concept)
            .into_iter()
            .flatten()
            .filter_map(|id| card_by_id.get(id))
            .any(|c| axis_value(&c.support, axis) == "partial")
    };
    let count_axis = |axis: &str| -> usize {
        syntax_names.iter().filter(|c| axis_validated(c, axis)).count()
    };

    let explicit_parse_coverage = Metric::new(count_axis("parse"), syntax_total);
    let generic_fallback_rate = Metric::new(
        syntax_names.iter().filter(|c| axis_partial(c, "parse")).count(),
        syntax_total,
    );
    let lowering_coverage = Metric::new(count_axis("lower"), syntax_total);
    let resolution_coverage = Metric::new(count_axis("resolve"), syntax_total);
    let elaboration_coverage = Metric::new(count_axis("elaborate"), syntax_total);
    let validation_coverage = Metric::new(count_axis("validate"), syntax_total);
    let execution_coverage = Metric::new(count_axis("execute"), syntax_total);
    let formatting_coverage = Metric::new(count_axis("format"), syntax_total);
    let lsp_coverage = Metric::new(count_axis("lsp"), syntax_total);

    // e2e: syntax concepts whose card proves parse AND lower and has no degraded
    // (partial/unsupported) axis — "reproduces cleanly as far as evidenced".
    let e2e_num = syntax_names
        .iter()
        .filter(|concept| {
            concepts
                .get(*concept)
                .into_iter()
                .flatten()
                .filter_map(|id| card_by_id.get(id))
                .any(|c| e2e_clean(&c.support))
        })
        .count();
    let e2e_coverage = Metric::new(e2e_num, syntax_total);

    // example / negative coverage over syntax concepts.
    let example_num = syntax_names
        .iter()
        .filter(|concept| {
            concepts
                .get(*concept)
                .into_iter()
                .flatten()
                .filter_map(|id| card_by_id.get(id))
                .any(|c| !c.examples.positive.is_empty())
        })
        .count();
    let example_coverage = Metric::new(example_num, syntax_total);

    let example_bearing: Vec<&LanguageCard> =
        pack.cards.iter().filter(|c| !c.examples.positive.is_empty()).collect();
    let with_negative = example_bearing.iter().filter(|c| !c.examples.negative.is_empty()).count();
    let negative_coverage = Metric::new(with_negative, example_bearing.len());

    // carded_parse_lower_coverage: the RENAMED old metric — over syntactic CARDS
    // only (non-null normalized_grammar), how many are parse AND lower validated.
    let syntactic_cards: Vec<&LanguageCard> =
        pack.cards.iter().filter(|c| c.normalized_grammar.is_some()).collect();
    let carded_pl = syntactic_cards
        .iter()
        .filter(|c| c.support.parse == "validated" && c.support.lower == "validated")
        .count();
    let carded_parse_lower_coverage = Metric::new(carded_pl, syntactic_cards.len());

    // --- coverage by source-kind (metamodel/SHACL/stdlib gaps become visible) ---
    let mut sk_total: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut sk_carded: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rec in &pack.denominator {
        // Every non-folded, non-alias/duplicate record is a "slot" for its kind.
        if matches!(rec.mapping.as_str(), "helper-fold" | "alias" | "duplicate") {
            continue;
        }
        sk_total
            .entry(rec.source_kind.clone())
            .or_default()
            .insert(rec.raw_name.clone());
        if rec.mapping == "card" {
            sk_carded
                .entry(rec.source_kind.clone())
                .or_default()
                .insert(rec.raw_name.clone());
        }
    }
    let mut coverage_by_source_kind: BTreeMap<String, Metric> = BTreeMap::new();
    for (kind, total) in &sk_total {
        let carded = sk_carded.get(kind).map(BTreeSet::len).unwrap_or(0);
        coverage_by_source_kind.insert(kind.clone(), Metric::new(carded, total.len()));
    }

    // --- Metamodel/SHACL disposition tallies (every class + shape) ---
    let disposition_label = |rec: &DenominatorRecord| -> String {
        match rec.mapping.as_str() {
            "card" => "carded".to_owned(),
            "duplicate" => {
                if rec.mapping_target.as_deref().is_some_and(|t| t.starts_with("tooling.")) {
                    "known-gap-fold".to_owned()
                } else {
                    "fold-enriched".to_owned()
                }
            }
            "exclusion" => "abstract-only".to_owned(),
            other => other.to_owned(),
        }
    };
    let mut metamodel_disposition: BTreeMap<String, usize> = BTreeMap::new();
    let mut shacl_disposition: BTreeMap<String, usize> = BTreeMap::new();
    for rec in &pack.denominator {
        match rec.source_kind.as_str() {
            "metamodel" => *metamodel_disposition.entry(disposition_label(rec)).or_insert(0) += 1,
            "shacl" => *shacl_disposition.entry(disposition_label(rec)).or_insert(0) += 1,
            _ => {}
        }
    }

    // --- families ---
    let mut card_count_by_family: BTreeMap<String, usize> = BTreeMap::new();
    let mut counted: BTreeSet<String> = BTreeSet::new();
    for rec in &pack.denominator {
        if rec.mapping != "card" {
            continue;
        }
        let Some(id) = &rec.normalized_concept_id else { continue };
        if !card_by_id.contains_key(id.as_str()) {
            continue;
        }
        if counted.insert(id.clone()) {
            let fam = family_of(&rec.source_kind, &rec.classification);
            *card_count_by_family.entry(fam.to_owned()).or_insert(0) += 1;
        }
    }
    let tooling = pack.cards.iter().filter(|c| c.id.starts_with("tooling.")).count();
    if tooling > 0 {
        card_count_by_family.insert("tooling-limitation".to_owned(), tooling);
    }

    // --- lists ---
    let mut covered: Vec<String> = pack
        .cards
        .iter()
        .filter(|c| c.support.parse == "validated" && c.support.lower == "validated")
        .map(|c| c.id.clone())
        .collect();
    covered.sort();

    let mut unknown_support: Vec<String> = pack
        .cards
        .iter()
        .filter(|c| all_unknown(&c.support))
        .map(|c| c.id.clone())
        .collect();
    unknown_support.sort();

    let mut blocked_support: Vec<String> = pack
        .cards
        .iter()
        .filter(|c| {
            let s = &c.support;
            [
                &s.parse, &s.lower, &s.resolve, &s.elaborate, &s.validate, &s.execute, &s.format,
                &s.lsp,
            ]
            .iter()
            .any(|a| a.as_str() == "unsupported")
        })
        .map(|c| c.id.clone())
        .collect();
    blocked_support.sort();

    let gap = |rec: &DenominatorRecord| GapRecord {
        source_id: rec.source_id.clone(),
        raw_name: rec.raw_name.clone(),
        source_kind: rec.source_kind.clone(),
        classification: rec.classification.clone(),
        mapping: rec.mapping.clone(),
        rationale: rec.classification_rationale.clone(),
        source_pointer: rec.source_pointer.clone(),
    };
    let mut uncarded: Vec<GapRecord> =
        pack.denominator.iter().filter(|r| r.mapping == "uncarded").map(gap).collect();
    uncarded.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let mut blocked: Vec<GapRecord> =
        pack.denominator.iter().filter(|r| r.mapping == "block").map(gap).collect();
    blocked.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let mut excluded: Vec<GapRecord> =
        pack.denominator.iter().filter(|r| r.mapping == "exclusion").map(gap).collect();
    excluded.sort_by(|a, b| a.source_id.cmp(&b.source_id));

    Completeness {
        spec_drop: pack.manifest.spec_drop.clone(),
        raw_rows: report.raw_rows,
        unique_names: report.unique_names,
        cross_grammar_shared: report.cross_grammar_shared,
        merge_count: report.merge_count,
        split_count: report.split_count,
        conflict_count: report.conflicts.len(),
        denominator_records: pack.denominator.len(),
        mapping_counts,
        card_coverage,
        user_facing_syntax_coverage,
        carded_parse_lower_coverage,
        explicit_parse_coverage,
        generic_fallback_rate,
        lowering_coverage,
        resolution_coverage,
        elaboration_coverage,
        validation_coverage,
        execution_coverage,
        formatting_coverage,
        lsp_coverage,
        example_coverage,
        negative_coverage,
        e2e_coverage,
        coverage_by_source_kind,
        metamodel_disposition,
        shacl_disposition,
        card_count_by_family,
        covered,
        unknown_support,
        blocked_support,
        uncarded,
        blocked,
        excluded,
        conflicts: report.conflicts.clone(),
        notes: notes(),
    }
}

fn axis_value<'a>(s: &'a super::support::SupportAxes, axis: &str) -> &'a str {
    match axis {
        "parse" => &s.parse,
        "lower" => &s.lower,
        "resolve" => &s.resolve,
        "elaborate" => &s.elaborate,
        "validate" => &s.validate,
        "execute" => &s.execute,
        "format" => &s.format,
        "lsp" => &s.lsp,
        _ => "unknown",
    }
}

fn e2e_clean(s: &super::support::SupportAxes) -> bool {
    let axes = [
        &s.parse, &s.lower, &s.resolve, &s.elaborate, &s.validate, &s.execute, &s.format, &s.lsp,
    ];
    let no_degraded = axes
        .iter()
        .all(|a| matches!(a.as_str(), "validated" | "unknown" | "not-applicable" | "spec-silent"));
    s.parse == "validated" && s.lower == "validated" && no_degraded
}

/// The auditable definition of every metric (honesty: say exactly what counted).
fn notes() -> Vec<String> {
    vec![
        "DENOMINATOR IS THE COMPLETE 650-name inventory plus the metamodel/SHACL/stdlib source \
         slots. Each record carries a `mapping`: card | uncarded | helper-fold | alias | \
         duplicate | exclusion | block. An in-scope card-bearing concept with no card is \
         `uncarded` and stays in the denominator, so it LOWERS coverage (it is not silently \
         omitted — omitting it would make the metric circular)."
            .to_owned(),
        "card_coverage = in-scope card-bearing concepts with a card / all in-scope card-bearing \
         concepts (mapping card|uncarded|block; classification user-facing|expression|operator|\
         semantic-only|library-defined), deduped by (source_kind, raw_name). helper-fold/alias/\
         duplicate/exclusion do not count (folded or reviewed out of scope; exclusion can never \
         inflate coverage)."
            .to_owned(),
        "user_facing_syntax_coverage = distinct user-facing grammar names with a card / all \
         distinct user-facing grammar names — the headline normative-syntax figure."
            .to_owned(),
        "explicit_parse_coverage / lowering_coverage / resolution_coverage / elaboration_coverage \
         / validation_coverage / execution_coverage / formatting_coverage / lsp_coverage = \
         syntax concepts whose minted card has that axis `validated` / ALL in-scope syntax \
         concepts (user-facing + operator + expression grammar rules). Uncarded concepts have no \
         card and count 0 in every numerator while staying in the denominator."
            .to_owned(),
        "generic_fallback_rate = syntax concepts whose card's parse axis is `partial` (a generic/\
         fallback acceptance is capped at partial by policy; a validated parse requires \
         zero unknown IR nodes) / all in-scope syntax concepts."
            .to_owned(),
        "e2e_coverage = syntax concepts whose card proves parse AND lower validated with no \
         `partial`/`unsupported` axis / all in-scope syntax concepts."
            .to_owned(),
        "example_coverage = in-scope syntax concepts with a card carrying >=1 positive example / \
         all in-scope syntax concepts. negative_coverage = example-bearing cards that also carry \
         a negative case / all example-bearing cards."
            .to_owned(),
        "carded_parse_lower_coverage = the RENAMED former parse+lower metric: syntactic cards \
         (non-null normalized_grammar) with parse AND lower validated / all syntactic cards. It \
         measures the QUALITY of the cards that exist, not coverage of the concept universe — \
         read it alongside card_coverage, never instead of it."
            .to_owned(),
        "coverage_by_source_kind counts carded / (carded + block + uncarded + reviewed-abstract) \
         non-folded slots per source. For metamodel/SHACL the headline is NOT this ratio: the \
         129+ metaclasses whose concept is grammar-carded fold onto that card and enrich its \
         metamodel_facet (counted in xtext coverage, `duplicate`-mapped here so not \
         double-counted); the small metamodel numerator is the metamodel-only membership cards. \
         The complete 182-class / 175-shape review is `metamodel_disposition` / \
         `shacl_disposition`, which replaces the earlier opaque metamodel 0/182, \
         shacl 0/257 slots."
            .to_owned(),
        "unknown_support lists cards whose support is honestly all-`unknown` (obligation, \
         S0xx-validation, operator, and library cards have no per-card example evidence; their \
         conformance is carried by validation_rules + the concept cards that trip them)."
            .to_owned(),
    ]
}

/// Render the completeness report as Markdown (`completeness.md`).
pub fn to_markdown(c: &Completeness) -> String {
    let mut m = String::new();
    m.push_str("# SysML/KerML Language Pack — Completeness Report\n\n");
    m.push_str(&format!("Spec drop: `{}`\n\n", c.spec_drop));
    m.push_str("Completeness is not \"one card per rule\": it is *every normalized in-scope \
                concept has a validated card OR an explicit exclusion/block with rationale and \
                provenance*. The denominator below is the COMPLETE 650-name grammar \
                inventory plus the metamodel/SHACL/stdlib source slots; every record carries an \
                auditable `mapping`. An in-scope concept with no card is `uncarded` and lowers \
                coverage — it is never silently dropped. Every metric is numerator/denominator, \
                never a bare percentage.\n\n");

    m.push_str("## Normalization (before → after)\n\n");
    m.push_str(&format!(
        "- Raw Xtext rule rows: **{}**\n- Unique rule names: **{}**\n- Cross-grammar shared \
         names: **{}** (merge {} / split {})\n- Cross-grammar divergence conflict records: \
         **{}**\n- Complete denominator records (all sources): **{}**\n\n",
        c.raw_rows, c.unique_names, c.cross_grammar_shared, c.merge_count, c.split_count,
        c.conflict_count, c.denominator_records
    ));

    m.push_str("### Mapping distribution\n\n| Mapping | Records |\n|---|---|\n");
    for (k, n) in &c.mapping_counts {
        m.push_str(&format!("| {k} | {n} |\n"));
    }
    m.push('\n');

    m.push_str("## Headline coverage (§11.4)\n\n");
    m.push_str("| Metric | Value |\n|---|---|\n");
    m.push_str(&format!("| card_coverage | {} |\n", c.card_coverage.ratio_str()));
    m.push_str(&format!(
        "| user_facing_syntax_coverage | {} |\n",
        c.user_facing_syntax_coverage.ratio_str()
    ));
    m.push_str(&format!(
        "| carded_parse_lower_coverage (card quality, not coverage) | {} |\n\n",
        c.carded_parse_lower_coverage.ratio_str()
    ));

    m.push_str("## Per-axis coverage over the full syntax-concept denominator (§11.4)\n\n");
    m.push_str("| Axis | Value |\n|---|---|\n");
    m.push_str(&format!("| explicit_parse_coverage | {} |\n", c.explicit_parse_coverage.ratio_str()));
    m.push_str(&format!("| generic_fallback_rate | {} |\n", c.generic_fallback_rate.ratio_str()));
    m.push_str(&format!("| lowering_coverage | {} |\n", c.lowering_coverage.ratio_str()));
    m.push_str(&format!("| resolution_coverage | {} |\n", c.resolution_coverage.ratio_str()));
    m.push_str(&format!("| elaboration_coverage | {} |\n", c.elaboration_coverage.ratio_str()));
    m.push_str(&format!("| validation_coverage | {} |\n", c.validation_coverage.ratio_str()));
    m.push_str(&format!("| execution_coverage | {} |\n", c.execution_coverage.ratio_str()));
    m.push_str(&format!("| formatting_coverage | {} |\n", c.formatting_coverage.ratio_str()));
    m.push_str(&format!("| lsp_coverage | {} |\n", c.lsp_coverage.ratio_str()));
    m.push_str(&format!("| example_coverage | {} |\n", c.example_coverage.ratio_str()));
    m.push_str(&format!("| negative_coverage | {} |\n", c.negative_coverage.ratio_str()));
    m.push_str(&format!("| e2e_coverage | {} |\n\n", c.e2e_coverage.ratio_str()));

    m.push_str("## Coverage by source kind (metamodel/SHACL/stdlib gaps are visible)\n\n");
    m.push_str("| Source kind | Carded / total |\n|---|---|\n");
    for (kind, metric) in &c.coverage_by_source_kind {
        m.push_str(&format!("| {kind} | {} |\n", metric.ratio_str()));
    }
    m.push('\n');

    m.push_str("## Metamodel & SHACL disposition\n\n");
    m.push_str("Every one of the 182 metamodel classes and every distinct constrained OSLC/SHACL \
                shape (257 raw ResourceShape declarations dedup to 175 distinct constrained types, \
                mirroring the metamodel's KerML/SysML re-declaration dedup) is reviewed. \
                `fold-enriched` = the metaclass is a concept card's `semantic_types`; its \
                inheritance + property constraints are materialized as that card's \
                `metamodel_facet` (no duplicate card). `carded` = a metamodel-only concept (the \
                graph memberships) with its own card. `known-gap-fold` = a written form our \
                lowering does not materialize, folded into an existing tooling limitation card. \
                `abstract-only` = an abstract base or metamodel enumeration with no textual \
                notation, reviewed out of carding scope.\n\n");
    let disp_table = |m: &mut String, label: &str, d: &BTreeMap<String, usize>| {
        let total: usize = d.values().sum();
        m.push_str(&format!("### {label} (total {total})\n\n| Disposition | Count |\n|---|---|\n"));
        for (k, n) in d {
            m.push_str(&format!("| {k} | {n} |\n"));
        }
        m.push('\n');
    };
    disp_table(&mut m, "Metamodel classes", &c.metamodel_disposition);
    disp_table(&mut m, "SHACL/OSLC shapes", &c.shacl_disposition);

    m.push_str("## Cards by family\n\n");
    m.push_str("| Family | Cards |\n|---|---|\n");
    for (fam, n) in &c.card_count_by_family {
        m.push_str(&format!("| {fam} | {n} |\n"));
    }
    m.push('\n');

    m.push_str(&format!(
        "## Support status\n\n- Fully-evidenced (parse+lower validated): **{}** cards\n- \
         Honest all-`unknown` support: **{}** cards\n- Blocked support (an `unsupported` axis \
         with a known-gap ref): **{}** cards\n- Uncarded in-scope concepts: **{}**\n- Blocked \
         concepts (in scope, no card yet — locator/source not integrated): **{}**\n- Excluded \
         (reviewed out of scope, with rationale): **{}**\n\n",
        c.covered.len(),
        c.unknown_support.len(),
        c.blocked_support.len(),
        c.uncarded.len(),
        c.blocked.len(),
        c.excluded.len()
    ));

    m.push_str("## Metric definitions (what was counted)\n\n");
    for n in &c.notes {
        m.push_str(&format!("- {n}\n"));
    }
    m.push('\n');

    m.push_str("Full covered / uncarded / blocked / excluded lists and every conflict record are \
                in `completeness.json`.\n");
    m
}
