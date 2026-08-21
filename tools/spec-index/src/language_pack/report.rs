//! Completeness / conflict reporting and the duplicate-ID + dangling-reference
//! gates. Counts are numerator/denominator, never
//! bare percentages.

use std::collections::BTreeSet;

use serde::Serialize;

use super::cards::LanguageCard;
use super::examples::Example;

/// The generated pack's report artifact.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub spec_drop: String,
    pub card_count: usize,
    pub example_count: usize,
    pub denominator_count: usize,
    /// Total `unknown` grammar-IR nodes across all cards (must be 0 for a
    /// `validated` parse claim).
    pub unknown_grammar_nodes: usize,
    /// Structured conflicts (duplicate IDs, dangling refs, dangling clauses).
    pub conflicts: Vec<String>,
    /// Recorded divergences / auditor notes (substitution rationale, the
    /// evidence-epoch design choice, schema-version status).
    pub notes: Vec<String>,
    /// Filled in by the exporter (excluded from the hash it reports).
    pub tree_hash: String,
}

/// Duplicate card IDs and duplicate example IDs (hard gate).
pub fn duplicate_ids(cards: &[LanguageCard], examples: &[Example]) -> Vec<String> {
    let mut conflicts = Vec::new();
    let mut seen = BTreeSet::new();
    for c in cards {
        if !seen.insert(c.id.clone()) {
            conflicts.push(format!("duplicate card id: {}", c.id));
        }
    }
    let mut seen_ex = BTreeSet::new();
    for e in examples {
        if !seen_ex.insert(e.id.clone()) {
            conflicts.push(format!("duplicate example id: {}", e.id));
        }
    }
    conflicts
}

/// Citation gate: every card claiming normative
/// authority (a `kerml.`/`sysml.` id — not a `tooling.` implementation card)
/// must carry at least one structured normative locator. Accepted locators are
/// a grammar rule ref (`normative_rules`), a spec clause (`normative_clauses`),
/// or a metamodel element (`semantic_types` — the abstract-syntax type(s) the
/// concept/constraint governs). A normative card with none is a hard failure:
/// it would assert normative authority with nothing to trace it to.
pub fn cards_without_locator(cards: &[LanguageCard]) -> Vec<String> {
    let mut conflicts = Vec::new();
    for c in cards {
        let normative = c.id.starts_with("kerml.") || c.id.starts_with("sysml.");
        if !normative {
            continue; // tooling.* implementation cards make no normative claim
        }
        let has_locator = !c.normative_rules.is_empty()
            || !c.normative_clauses.is_empty()
            || !c.semantic_types.is_empty();
        if !has_locator {
            conflicts.push(format!(
                "{}: normative card carries no normative locator (grammar rule, spec clause, or \
                 metamodel element)",
                c.id
            ));
        }
    }
    conflicts
}

/// Escape hatch for the topical-plausibility gate: a
/// `(card_id, rationale)` allowlist for the rare genuinely-odd
/// citation whose concept keyword legitimately does not surface in the cited
/// clause's opening prose (e.g. a constraint stated only in a derived
/// attribute's OCL, or grounded in a parent clause). Each entry is a reviewed
/// exception and must stay rare — the default expectation is that a citation
/// earns its keep by discussing the concept. Empty: after the citation
/// repair every semantic-rule card's cited clause names its own concept.
const REVIEWED_CITATIONS: &[(&str, &str)] = &[];

/// Generic metamodel/prose tokens that occur in almost every abstract-syntax
/// clause; matching on them would make the topical gate vacuous, so they are
/// excluded from a card's candidate keyword set.
const TOPICAL_STOPWORDS: &[&str] = &[
    "usage", "definition", "kind", "node", "membership", "type", "types",
    "element", "feature", "features", "owned", "model", "abstract", "syntax",
    "validation", "constraint", "constraints", "value", "values", "must",
    "with", "from", "that", "this", "have",
];

/// Topical-plausibility citation gate.
/// Beyond mere existence (`cards_without_locator`), a semantic-rule card's cited
/// clause must actually *discuss* the concept it governs: at least one of the
/// card's element-kind keywords (its `semantic_types`, whole or CamelCase-split)
/// or rule subject-nouns (its `keywords`) must appear in the cited clause's
/// heading or opening prose (`clause_contexts`, ~40 lines). This catches a
/// citation that resolves to a real-but-wrong clause — e.g. the former blanket
/// `SysML 8.3.4` (Annotations) default on typing/state/requirement rules.
///
/// Scoped to the validation-facet cards (`*.validation.*`) minted from
/// `semantic_rules.toml` — the set this repair covers and whose citations are
/// modeler-authored. Matching is case-insensitive with singular/plural
/// tolerance. `contexts` maps clause-number -> lowercased clause context, keyed
/// by document ("SysML"/"KerML").
pub fn cards_missing_topical_citation(
    cards: &[LanguageCard],
    contexts: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
) -> Vec<String> {
    let mut conflicts = Vec::new();
    for c in cards {
        // Scope: the semantic-rule cards minted from `semantic_rules.toml` — the
        // set this repair covers. They are identified by carrying at least one
        // `S###` rule id in `validation_rules`; this excludes the obligation- and
        // stdlib-derived validation-facet cards, whose citations have separate
        // provenance and are not modeler-authored spec_refs.
        if !c.validation_rules.iter().any(|r| is_semantic_rule_id(r)) {
            continue;
        }
        if REVIEWED_CITATIONS.iter().any(|(id, _)| *id == c.id) {
            continue;
        }
        let candidates = candidate_keywords(&c.semantic_types, &c.keywords);

        for cl in &c.normative_clauses {
            let Some(doc_ctx) = contexts.get(&cl.document) else { continue };
            let Some(ctx) = doc_ctx.get(&cl.clause) else { continue };
            let hit = candidates.iter().any(|k| keyword_in(ctx, k));
            if !hit {
                conflicts.push(format!(
                    "{}: cited clause {} {} discusses none of the card's concept keywords \
                     [{}] (topical-plausibility gate)",
                    c.id,
                    cl.document,
                    cl.clause,
                    candidates.iter().cloned().collect::<Vec<_>>().join(", "),
                ));
            }
        }
    }
    conflicts
}

/// A card's concept keyword set for the topical gate: its element kinds (whole
/// + CamelCase tokens) and rule subject-nouns (`keywords`), lowercased, minus
/// generic metamodel stopwords and tokens shorter than 4 chars.
fn candidate_keywords(semantic_types: &[String], keywords: &[String]) -> BTreeSet<String> {
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for t in semantic_types {
        candidates.insert(t.to_ascii_lowercase());
        for tok in split_camel(t) {
            candidates.insert(tok);
        }
    }
    for k in keywords {
        candidates.insert(k.to_ascii_lowercase());
    }
    candidates.retain(|k| k.len() >= 4 && !TOPICAL_STOPWORDS.contains(&k.as_str()));
    candidates
}

/// A `semantic_rules.toml` rule id: `S` followed by exactly three digits
/// (`S001`..`S146`). Distinguishes the semantic-rule cards from obligation /
/// stdlib validation-facet cards (whose `validation_rules` hold slug ids).
fn is_semantic_rule_id(r: &str) -> bool {
    let Some(rest) = r.strip_prefix('S') else { return false };
    rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit())
}

/// Case-insensitive substring match with singular/plural tolerance. `ctx` is
/// already lowercased; `k` is lowercased. Substring already catches a singular
/// keyword inside a plural context word; the extra de-pluralized probe catches
/// the reverse (a plural keyword against a singular context word).
fn keyword_in(ctx: &str, k: &str) -> bool {
    if ctx.contains(k) {
        return true;
    }
    let stripped = k.strip_suffix('s').unwrap_or(k);
    stripped.len() >= 4 && ctx.contains(stripped)
}

/// Split a CamelCase metamodel type name into lowercased word tokens
/// (`RequirementDefinition` -> `["requirement", "definition"]`).
fn split_camel(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_ascii_lowercase());
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur.to_ascii_lowercase());
    }
    out
}

/// Every `related_cards` link and `examples.*` reference must resolve (hard
/// gate).
pub fn dangling_references(cards: &[LanguageCard], examples: &[Example]) -> Vec<String> {
    let card_ids: BTreeSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
    let example_ids: BTreeSet<&str> = examples.iter().map(|e| e.id.as_str()).collect();
    let mut conflicts = Vec::new();
    for c in cards {
        for r in &c.related_cards {
            if !card_ids.contains(r.as_str()) {
                conflicts.push(format!("{}: dangling related_card {r}", c.id));
            }
        }
        for e in c
            .examples
            .positive
            .iter()
            .chain(&c.examples.negative)
            .chain(&c.examples.composed)
        {
            if !example_ids.contains(e.as_str()) {
                conflicts.push(format!("{}: dangling example ref {e}", c.id));
            }
        }
    }
    conflicts
}

#[cfg(test)]
mod topical_gate_tests {
    use super::*;

    #[test]
    fn semantic_rule_id_discriminates() {
        assert!(is_semantic_rule_id("S001"));
        assert!(is_semantic_rule_id("S146"));
        assert!(!is_semantic_rule_id("at-most-one-subject"));
        assert!(!is_semantic_rule_id("S1")); // obligation-ish, wrong shape
        assert!(!is_semantic_rule_id("S0001"));
    }

    #[test]
    fn camel_split_tokens() {
        assert_eq!(split_camel("RequirementDefinition"), vec!["requirement", "definition"]);
        assert_eq!(split_camel("MergeNode"), vec!["merge", "node"]);
    }

    #[test]
    fn keyword_plural_tolerance() {
        // singular keyword inside a plural context word (substring)
        assert!(keyword_in("a requirementdefinition has subjects", "subject"));
        // plural keyword against a singular context word (de-pluralized probe)
        assert!(keyword_in("the subject parameter", "subjects"));
        assert!(!keyword_in("annotations abstract syntax", "requirement"));
    }

    // The gate's core decision: a requirement/subject rule cited to the
    // Annotations clause is flagged; cited to the RequirementDefinition clause
    // it passes. This is exactly the former blanket `SysML 8.3.4` failure mode.
    #[test]
    fn topical_decision_has_teeth() {
        let semantic_types = vec!["RequirementDefinition".to_owned()];
        let keywords = vec!["subject".to_owned(), "requirement".to_owned(), "validation".to_owned()];
        let cands = candidate_keywords(&semantic_types, &keywords);

        let annotations_ctx = "annotations abstract syntax\nan annotation relates an \
                               annotatingelement to the element it annotates via comments \
                               and documentation.";
        let reqdef_ctx = "requirementdefinition\na requirementdefinition is a \
                          constraintdefinition that defines a requirement relative to a \
                          specified subject.";

        assert!(
            !cands.iter().any(|k| keyword_in(annotations_ctx, k)),
            "requirement rule must NOT be plausible against the Annotations clause"
        );
        assert!(
            cands.iter().any(|k| keyword_in(reqdef_ctx, k)),
            "requirement rule must be plausible against the RequirementDefinition clause"
        );
    }

    #[test]
    fn stopwords_are_excluded() {
        // A card whose only tokens are generic must yield no candidates, so it
        // cannot be spuriously validated (or spuriously flagged) on them.
        let cands = candidate_keywords(&["Usage".to_owned()], &["definition".to_owned(), "type".to_owned()]);
        assert!(cands.is_empty(), "generic-only card should produce no candidate keywords: {cands:?}");
    }
}
