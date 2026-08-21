//! Normalized-denominator closure and the cross-grammar body-equivalence
//! merge. This is the load-bearing normalization step: it classifies
//! every raw grammar rule and decides, for each name shared across grammars,
//! whether the productions are equivalent (one merged card) or divergent (two
//! authority-scoped cards + a conflict record) — by comparing the normalized
//! grammar IR, never by picking one grammar as authority.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sysml_codegen::{GrammarRuleIr, IrNode};

use super::cards::LanguageCard;
use super::concepts::{classify_rule, Classification, DenominatorRecord, Mapping};
use super::xtext_ir::Grammars;

/// Merge decision for a cross-grammar shared name (§6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeDecision {
    /// IR-equivalent in every grammar that declares it → one merged card.
    Merge,
    /// Divergent bodies → authority-scoped cards + a conflict record.
    Split,
}

/// One cross-grammar-shared rule name and its merge decision.
#[derive(Debug, Clone)]
pub struct SharedName {
    pub name: String,
    /// Grammars declaring it, sorted (`kerml` < `sysml` etc.).
    pub grammars: Vec<String>,
    pub decision: MergeDecision,
}

/// A structured conflict for a divergent cross-grammar pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictRecord {
    pub kind: String, // "cross-grammar-divergence"
    pub raw_name: String,
    pub grammars: Vec<String>,
}

/// Serializable published denominator report. The
/// 725->650 collapse and the §6.2 merge/split counts are auditable here.
#[derive(Debug, Clone, Serialize)]
pub struct DenominatorReport {
    pub raw_rows: usize,
    pub unique_names: usize,
    pub cross_grammar_shared: usize,
    pub merge_count: usize,
    pub split_count: usize,
    /// Cards minted from the shared set: merge names -> 1, split names -> 2.
    pub cards_from_shared: usize,
    pub split_set: Vec<String>,
    /// Divergent names that classify user-facing — these mint kerml.*+sysml.*
    /// authority-scoped card pairs (approved split policy).
    pub split_user_facing: Vec<String>,
    /// Divergent names that are helper fragments — they fold into their parent
    /// cards, but their conflict records still ship for auditability.
    pub split_helpers: Vec<String>,
    pub conflicts: Vec<ConflictRecord>,
    /// Divergent names given an explicit reviewed classification:
    /// name -> rationale for folding into a parent card instead of minting one.
    pub reviewed_reclassifications: BTreeMap<String, String>,
    pub classification_counts: BTreeMap<String, usize>,
}

/// Published denominator statistics.
#[derive(Debug, Clone)]
pub struct DenominatorStats {
    /// Total raw rule rows across the three grammars.
    pub raw_rows: usize,
    /// Distinct rule names.
    pub unique_names: usize,
    /// Names declared in more than one grammar.
    pub shared_names: Vec<SharedName>,
    /// Shared names whose bodies are IR-equivalent (collapse to one card).
    pub merge_set: Vec<String>,
    /// Shared names whose bodies diverge (two authority-scoped cards).
    pub split_set: Vec<String>,
    pub conflicts: Vec<ConflictRecord>,
    /// Count of raw names per §6.1 classification.
    pub classification_counts: BTreeMap<String, usize>,
}

impl DenominatorStats {
    /// Card-count delta from the shared-name collapse: merge names yield 1 card,
    /// split names yield 2. Returns (before_unique, after_cards_from_shared).
    pub fn shared_card_delta(&self) -> (usize, usize) {
        let after = self.merge_set.len() + self.split_set.len() * 2;
        (self.shared_names.len(), after)
    }

    /// A serializable, deterministic report (split set + conflicts sorted).
    pub fn to_report(&self) -> DenominatorReport {
        let (_, cards_from_shared) = self.shared_card_delta();
        let mut split_set = self.split_set.clone();
        split_set.sort();
        let mut conflicts = self.conflicts.clone();
        conflicts.sort_by(|a, b| a.raw_name.cmp(&b.raw_name));
        // Split policy (approved): only user-facing divergent names mint
        // authority-scoped card pairs; helper fragments fold into parents.
        let mut split_user_facing: Vec<String> = split_set
            .iter()
            .filter(|n| classify_rule(n).is_card_bearing())
            .cloned()
            .collect();
        split_user_facing.sort();
        let split_helpers: Vec<String> = split_set
            .iter()
            .filter(|n| !classify_rule(n).is_card_bearing())
            .cloned()
            .collect();
        // Publish the reviewed-reclassification rationale for any split-set name
        // that carries one (reclassifications are recorded).
        let reviewed_reclassifications: BTreeMap<String, String> = split_set
            .iter()
            .filter_map(|n| {
                super::concepts::reviewed_rationale(n).map(|r| (n.clone(), r.to_owned()))
            })
            .collect();
        DenominatorReport {
            raw_rows: self.raw_rows,
            unique_names: self.unique_names,
            cross_grammar_shared: self.shared_names.len(),
            merge_count: self.merge_set.len(),
            split_count: self.split_set.len(),
            cards_from_shared,
            split_set,
            split_user_facing,
            split_helpers,
            conflicts,
            reviewed_reclassifications,
            classification_counts: self.classification_counts.clone(),
        }
    }
}

/// Structural IR equivalence: the normalized expression trees are equal. Because
/// the IR parser already discards whitespace/comments during tokenization,
/// structural `PartialEq` is the normalized comparison the merge asks
/// for (not a raw byte compare).
fn ir_equivalent(a: &IrNode, b: &IrNode) -> bool {
    a == b
}

/// Run the denominator closure + §6.2 merge over the three grammars.
pub fn analyze(grammars: &Grammars) -> DenominatorStats {
    let all: Vec<GrammarRuleIr> = grammars.all_ir();
    let raw_rows = all.len();

    // name -> [(grammar, ir)]
    let mut by_name: BTreeMap<String, Vec<GrammarRuleIr>> = BTreeMap::new();
    for ir in all {
        by_name.entry(ir.rule.clone()).or_default().push(ir);
    }
    let unique_names = by_name.len();

    let mut shared_names = Vec::new();
    let mut merge_set = Vec::new();
    let mut split_set = Vec::new();
    let mut conflicts = Vec::new();

    for (name, defs) in &by_name {
        if defs.len() < 2 {
            continue;
        }
        let mut grammars_for: Vec<String> = defs.iter().map(|d| d.grammar.clone()).collect();
        grammars_for.sort();
        grammars_for.dedup();
        if grammars_for.len() < 2 {
            continue; // same name twice in one grammar is not cross-grammar
        }
        // IR-equivalent across every declaring grammar?
        let Some(first) = defs.first().map(|d| &d.expression) else {
            continue;
        };
        let all_equal = defs.iter().all(|d| ir_equivalent(&d.expression, first));
        let decision = if all_equal {
            merge_set.push(name.clone());
            MergeDecision::Merge
        } else {
            split_set.push(name.clone());
            conflicts.push(ConflictRecord {
                kind: "cross-grammar-divergence".to_owned(),
                raw_name: name.clone(),
                grammars: grammars_for.clone(),
            });
            MergeDecision::Split
        };
        shared_names.push(SharedName {
            name: name.clone(),
            grammars: grammars_for,
            decision,
        });
    }

    // Classification tally over all unique raw names.
    let mut classification_counts: BTreeMap<String, usize> = BTreeMap::new();
    for name in by_name.keys() {
        let c: Classification = classify_rule(name);
        *classification_counts.entry(c.as_str().to_owned()).or_insert(0) += 1;
    }

    DenominatorStats {
        raw_rows,
        unique_names,
        shared_names,
        merge_set,
        split_set,
        conflicts,
        classification_counts,
    }
}

/// Every distinct grammar rule name in the universe, mapped to the grammars
/// that declare it (sorted). This is the authoritative 650-name inventory.
pub fn rule_name_grammars(grammars: &Grammars) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ir in grammars.all_ir() {
        let e = out.entry(ir.rule.clone()).or_default();
        if !e.contains(&ir.grammar) {
            e.push(ir.grammar.clone());
        }
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
}

/// A representative grammar for a name (deterministic: `kerml` < `sysml` <
/// `expressions` by sort order in [`rule_name_grammars`]).
fn representative_grammar(grammars_for: &[String]) -> &str {
    grammars_for.first().map(String::as_str).unwrap_or("kerml")
}

/// Complete the xtext denominator to the FULL 650-name inventory.
/// The card-minting loops in `mod.rs` produce records
/// only for concepts that got a card; this adds one record for every remaining
/// unique grammar rule name with an auditable [`Mapping`]:
///
/// - card-bearing name with no card yet → `Uncarded` (lowers `card_coverage`);
/// - lexical/structural helper → `HelperFold` (target = a carded parent that
///   references it via `rule_dependencies`, else none — it folds once its
///   parent is carded);
///
/// `already_covered` is the set of raw rule names that the card loops already
/// emitted a record for (so merge/split/operator/pilot names are not doubled).
pub fn complete_xtext_closure(
    grammars: &Grammars,
    cards: &[LanguageCard],
    already_covered: &BTreeSet<String>,
) -> Vec<DenominatorRecord> {
    let inventory = rule_name_grammars(grammars);

    // helper rule name -> a carded parent concept that references it (smallest
    // card id wins, for determinism). Only carded parents; the transitive
    // helper-through-helper resolution is the dependency-map's job.
    let mut helper_parent: BTreeMap<String, String> = BTreeMap::new();
    for card in cards {
        for dep in &card.rule_dependencies {
            if !classify_rule(dep).is_card_bearing() {
                helper_parent
                    .entry(dep.clone())
                    .and_modify(|cur| {
                        if card.id < *cur {
                            *cur = card.id.clone();
                        }
                    })
                    .or_insert_with(|| card.id.clone());
            }
        }
    }

    let mut out = Vec::new();
    for (name, grammars_for) in &inventory {
        if already_covered.contains(name) {
            continue;
        }
        let grammar = representative_grammar(grammars_for);
        let classification = classify_rule(name);
        if classification.is_card_bearing() {
            // In-scope, card-bearing, but no card yet — the honest gap. If a
            // known-gap registry record documents why our lowering cannot card it
            // faithfully, annotate the record with that gap so the
            // coverage-lowering entry carries an auditable reason instead of the
            // generic "no card yet".
            let rationale = match super::known_gaps::uncarded_gap_note(name) {
                Some((gap_id, tooling_card, status)) => format!(
                    "in-scope card-bearing grammar concept our tree-sitter lowering does not \
                     materialize as its distinct element kind ({status} implementation gap \
                     {gap_id}, documented by tooling card {tooling_card}); stays uncarded and \
                     lowers card_coverage"
                ),
                None => "in-scope card-bearing grammar concept with no card yet (bulk syntax \
                         authoring pending); stays in the denominator and lowers card_coverage"
                    .to_owned(),
            };
            let mut rec = DenominatorRecord::for_xtext_rule(
                grammar,
                name,
                classification,
                None,
                Mapping::Uncarded,
                &rationale,
            );
            rec.review_state = "reviewed".to_owned();
            out.push(rec);
        } else {
            // Lexical/structural helper: folds into a parent concept.
            let target = helper_parent.get(name).cloned();
            let rationale = super::concepts::reviewed_rationale(name)
                .map(|r| format!("reviewed helper reclassification: {r}"))
                .unwrap_or_else(|| match &target {
                    Some(p) => format!(
                        "{} grammar helper; folds into carded concept {p} (referenced by its \
                         rule_dependencies)",
                        classification.as_str()
                    ),
                    None => format!(
                        "{} grammar helper; folds into its parent concept once that concept is \
                         carded (no carded parent references it yet)",
                        classification.as_str()
                    ),
                });
            let mut rec = DenominatorRecord::for_xtext_rule(
                grammar,
                name,
                classification,
                None,
                Mapping::HelperFold,
                &rationale,
            );
            rec.mapping_target = target;
            rec.review_state = "reviewed".to_owned();
            out.push(rec);
        }
    }
    out
}

/// Parse `local:Name a rdfs:Class ;` declarations out of a vocab TTL, returning
/// the local names (design source 3 metamodel). A cheap, deterministic count of
/// the metamodel concept universe — enough to surface the gap, not a full RDF
/// parse (metamodel card integration is tracked separately).
pub fn parse_ttl_class_names(ttl: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in ttl.lines() {
        let t = line.trim();
        if let Some(head) = t.strip_suffix("a rdfs:Class ;").map(str::trim_end) {
            if let Some((_, local)) = head.trim().split_once(':') {
                let name = local.trim();
                if !name.is_empty() && !out.contains(&name.to_owned()) {
                    out.push(name.to_owned());
                }
            }
        }
    }
    out
}

/// Parse `:NameShape a oslc:ResourceShape ;` declarations out of a shapes TTL.
/// The pilot's SHACL surface is expressed as OSLC `ResourceShape`s.
pub fn parse_shacl_shape_names(ttl: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in ttl.lines() {
        let t = line.trim();
        if let Some(head) = t.strip_suffix("a oslc:ResourceShape ;").map(str::trim_end) {
            let head = head.trim();
            let name = head.strip_prefix(':').unwrap_or(head).trim();
            if !name.is_empty() && !out.contains(&name.to_owned()) {
                out.push(name.to_owned());
            }
        }
    }
    out
}

/// Emit one `block`-mapped denominator record per metamodel/SHACL source concept
/// so those gaps are VISIBLE in completeness. Population
/// (carding) is tracked separately; here they are surfaced as uncovered, blocked
/// slots grounded in the allowlisted, hashed TTL sources.
pub fn source_concept_slots(
    metamodel_names: &[(&'static str, String, &'static str)],
    shacl_names: &[(&'static str, String, &'static str)],
) -> Vec<DenominatorRecord> {
    let mut out = Vec::new();
    // Dedup by name: the SysML vocab re-declares every KerML base class, so a
    // name shared across the two TTLs is ONE metamodel concept (first-wins keeps
    // the KerML-origin pointer). The source_id must stay unique.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (source_pointer, name, doc) in metamodel_names {
        if !seen.insert(format!("metamodel:{name}")) {
            continue;
        }
        out.push(source_slot_record("metamodel", name, source_pointer, Classification::AbstractOnly, &format!(
            "{doc} metamodel element (abstract syntax); not yet integrated into a card — surfaced \
             here as an uncovered slot grounded in the allowlisted, hashed vocabulary TTL"
        )));
    }
    for (source_pointer, name, doc) in shacl_names {
        if !seen.insert(format!("shacl:{name}")) {
            continue;
        }
        out.push(source_slot_record("shacl", name, source_pointer, Classification::AbstractOnly, &format!(
            "{doc} SHACL/OSLC resource-shape constraint; not yet integrated into a card — surfaced \
             here as an uncovered slot grounded in the allowlisted, hashed shapes TTL"
        )));
    }
    out
}

fn source_slot_record(
    source_kind: &str,
    name: &str,
    source_pointer: &str,
    classification: Classification,
    rationale: &str,
) -> DenominatorRecord {
    DenominatorRecord {
        source_id: format!("{source_kind}:{name}"),
        source_kind: source_kind.to_owned(),
        raw_name: name.to_owned(),
        normalized_concept_id: None,
        classification: classification.as_str().to_owned(),
        classification_rationale: rationale.to_owned(),
        review_state: "reviewed".to_owned(),
        source_pointer: source_pointer.to_owned(),
        mapping: Mapping::Block.as_str().to_owned(),
        mapping_target: None,
        merged_from: Vec::new(),
    }
}

/// The reviewed standard-library denominator:
/// one record per aggregate-root library package in the explicit manifest
/// [`super::manifest::STDLIB_LIBRARY_PACKAGES`] (93 packages across the Kernel,
/// Systems, and Domain libraries — enumerated, never glob-discovered). This
/// REPLACES the single opaque `unknown-blocks-completion` slot with a real,
/// grounded library denominator: the count is now known (93 aggregate roots),
/// and the load-bearing member constructs are carded selectively
/// ([`super::stdlib`]) — the review's "record every reviewed source concept,
/// card only the useful normalized ones" at aggregate-root granularity.
///
/// `carded_paths` is the set of library file paths that host ≥1 stdlib card, so a
/// package's rationale states whether it contributes a carded construct. Each
/// record is a reviewed `exclusion` (the package is a namespace container; its
/// members, not the package, are the carded leaf concepts) — never inflating
/// card_coverage, and honest that member-level carding is selective.
pub fn library_package_records(carded_paths: &BTreeSet<String>) -> Vec<DenominatorRecord> {
    let mut out = Vec::new();
    for (group, package, path) in super::manifest::STDLIB_LIBRARY_PACKAGES {
        let has_card = carded_paths.contains(&(*path).to_owned());
        let rationale = if has_card {
            format!(
                "standard-library aggregate root: the `{package}` library package ({group}); a \
                 load-bearing member construct of this package is carded as a library-defined \
                 concept (see the stdlib cards citing this file). Reviewed source concept recorded \
                 at aggregate-root granularity; remaining members are carded selectively."
            )
        } else {
            format!(
                "standard-library aggregate root: the `{package}` library package ({group}). \
                 Reviewed and recorded in the denominator (replacing the former \
                 `unknown-blocks-completion` slot); its member constructs are load-bearing library \
                 semantics but not individually carded yet — bulk member-level carding is \
                 documented, selective future work, never a silent omission."
            )
        };
        out.push(DenominatorRecord {
            source_id: format!("stdlib:pkg:{package}"),
            source_kind: "stdlib".to_owned(),
            raw_name: (*package).to_owned(),
            normalized_concept_id: None,
            classification: Classification::LibraryDefined.as_str().to_owned(),
            classification_rationale: rationale,
            review_state: "reviewed".to_owned(),
            source_pointer: (*path).to_owned(),
            mapping: Mapping::Exclusion.as_str().to_owned(),
            mapping_target: None,
            merged_from: Vec::new(),
        });
    }
    out
}

/// The `unknown-blocks-completion` standard-library-source slot. A
/// trustworthy full count of standard-library-defined semantic concepts needs
/// deep parsing of the library (mostly outside the allowlist), so the gap is
/// surfaced as one blocked slot whose rationale records that the count is
/// unknown — which, per §8.18, blocks completion rather than being silently
/// treated as covered.
pub fn stdlib_source_slot(carded: usize) -> DenominatorRecord {
    DenominatorRecord {
        source_id: "stdlib:__standard-library-source-concepts__".to_owned(),
        source_kind: "stdlib".to_owned(),
        raw_name: "<standard-library-source-concepts>".to_owned(),
        normalized_concept_id: None,
        classification: Classification::LibraryDefined.as_str().to_owned(),
        classification_rationale: format!(
            "standard-library-defined semantic concepts beyond the {carded} carded; a trustworthy \
             full count requires deep parsing of the standard library (mostly outside the source \
             allowlist). Count unknown, which blocks completion: the slot \
             is surfaced as blocked, never treated as covered"
        ),
        review_state: "reviewed".to_owned(),
        source_pointer: "references/sysmlv2/SysML-v2-Pilot-Implementation/sysml.library".to_owned(),
        mapping: Mapping::Block.as_str().to_owned(),
        mapping_target: None,
        merged_from: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::print_stdout, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn denominator_stats_snapshot() {
        if !super::super::fetched_sources_present(&super::super::repo_root()) {
            eprintln!("SKIP: references not fetched (run tools/fetch-references/fetch.sh fetch)");
            return;
        }
        let repo = super::super::repo_root();
        let grammars = super::super::load_grammars(&repo).unwrap();
        let s = analyze(&grammars);
        println!("raw_rows={} unique_names={}", s.raw_rows, s.unique_names);
        println!(
            "shared_names={} merge_set={} split_set={}",
            s.shared_names.len(),
            s.merge_set.len(),
            s.split_set.len()
        );
        let (before, after) = s.shared_card_delta();
        println!("shared collapse: {before} shared -> {after} cards");
        println!("SPLIT SET (divergent, mint kerml.*+sysml.*):");
        for n in &s.split_set {
            println!("  - {n}");
        }
        println!("classification counts:");
        for (k, v) in &s.classification_counts {
            println!("  {k}: {v}");
        }
        // Design section 0 grounding: 725 raw rows, 650 unique names, 73 shared.
        assert_eq!(s.raw_rows, 725, "raw rows must match spec-drop grounding");
        assert_eq!(s.unique_names, 650, "unique names must match spec-drop grounding");
        assert_eq!(s.shared_names.len(), 73, "KerML/SysML shared names");
        assert_eq!(
            s.merge_set.len() + s.split_set.len(),
            73,
            "every shared name is merge or split"
        );
        // The design's confirmed split members must be split by the live check.
        for known in ["FeatureTyping", "RootNamespace", "MultiplicityRange", "RelationshipBody"] {
            assert!(
                s.split_set.contains(&known.to_owned()),
                "{known} must be in the split set"
            );
        }
    }
}
