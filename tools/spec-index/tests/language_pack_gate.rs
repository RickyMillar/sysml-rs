//! Gates for the generated language pack under
//! `references/sysmlv2/derived/language-pack/`:
//!
//! 1. **Regen-diff** — regenerate the pack in-process and byte-compare every
//!    file of the pack on disk. Any drift (hand edit, stale artifact after a
//!    spec drop, generator change without regeneration) is a hard failure.
//! 2. **Schema** — every exported card/example/denominator record validates
//!    against its committed JSON Schema.
//! 3. **Duplicate-ID / dangling-ref** — no two cards or examples share an ID;
//!    every cross-reference resolves.
//! 4. **Determinism** — two clean runs produce identical tree hashes.
//!
//! The pack directory is not tracked in git (all of `references/sysmlv2/` is
//! generated/fetched); every test here skips with a message when the fetched
//! sources or the generated pack are absent, and runs fully when present.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use spec_index::language_pack::{self, export, report, schema};

fn repo_root() -> PathBuf {
    language_pack::repo_root()
}

fn committed_dir(root: &Path) -> PathBuf {
    language_pack::default_output_dir(root)
}

fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// Regen-diff: the pack on disk byte-equals a fresh in-process regeneration.
#[test]
fn language_pack_matches_regeneration() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let committed = committed_dir(&root);
    if !committed.exists() {
        eprintln!(
            "SKIP: no language pack at {} (run cargo run -p spec-index -- language-pack)",
            committed.display()
        );
        return;
    }

    let tmp = std::env::temp_dir().join(format!("lp-regen-{}/language-pack", std::process::id()));
    std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
    language_pack::run(&root, &tmp).expect("regeneration");

    let a = read_tree(&committed);
    let b = read_tree(&tmp);
    let a_keys: Vec<&String> = a.keys().collect();
    let b_keys: Vec<&String> = b.keys().collect();
    assert_eq!(
        a_keys, b_keys,
        "pack file set on disk differs from a fresh regeneration"
    );
    for (rel, bytes) in &a {
        assert_eq!(
            bytes,
            b.get(rel).unwrap(),
            "derived/language-pack/{rel} differs from a fresh regeneration — it was \
             hand-edited, or the sources/generator changed without re-running \
             cargo run -p spec-index -- language-pack"
        );
    }
    let _ = std::fs::remove_dir_all(tmp.parent().unwrap());
}

/// Schema + duplicate-ID + dangling-ref gates over the generated pack.
#[test]
fn language_pack_gates_pass() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let pack = language_pack::generate(&root).expect("generate");

    // Duplicate-ID + dangling-ref gates.
    let mut conflicts = report::duplicate_ids(&pack.cards, &pack.examples);
    conflicts.extend(report::dangling_references(&pack.cards, &pack.examples));
    assert!(conflicts.is_empty(), "pack conflicts: {conflicts:?}");

    // Schema gate.
    let schemas = schema::SchemaSet::load(&root).expect("load schemas");
    for card in &pack.cards {
        let value = export::to_value(card).unwrap();
        schemas
            .validate("language-card.schema.json", &value)
            .unwrap_or_else(|e| panic!("card {} invalid: {e:?}", card.id));
    }
    for ex in &pack.examples {
        let value = export::to_value(ex).unwrap();
        schemas
            .validate("example.schema.json", &value)
            .unwrap_or_else(|e| panic!("example {} invalid: {e:?}", ex.id));
    }
    for rec in &pack.denominator {
        let value = export::to_value(rec).unwrap();
        schemas
            .validate("denominator-record.schema.json", &value)
            .unwrap_or_else(|e| panic!("denominator {} invalid: {e:?}", rec.source_id));
    }
}

/// Denominator-closure gate: the generated denominator closure
/// must not diverge from the 650-name grammar inventory. Every unique grammar
/// rule name has at least one xtext denominator record, and every xtext record
/// names a real grammar rule — so no concept can be silently dropped.
#[test]
fn denominator_closure_covers_full_650_inventory() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let pack = language_pack::generate(&root).expect("generate");
    let grammars = language_pack::load_grammars(&root).expect("grammars");
    let universe = grammars.rule_name_universe();

    let recorded: std::collections::BTreeSet<String> = pack
        .denominator
        .iter()
        .filter(|r| r.source_kind == "xtext")
        .map(|r| r.raw_name.clone())
        .collect();

    // Every inventory name is recorded.
    let missing: Vec<&String> = universe.iter().filter(|n| !recorded.contains(*n)).collect();
    assert!(missing.is_empty(), "grammar names with no denominator record: {missing:?}");
    // Every xtext record names a real grammar rule.
    let extra: Vec<&String> = recorded.iter().filter(|n| !universe.contains(*n)).collect();
    assert!(extra.is_empty(), "xtext denominator records not in the grammar universe: {extra:?}");
    // The inventory is the design-grounded 650 unique names.
    assert_eq!(recorded.len(), universe.len(), "closure size must equal the inventory");
    assert_eq!(universe.len(), 650, "grammar inventory must be the 650 unique names");

    // Every record carries a mapping; card mappings name a concept, non-card do not.
    for rec in &pack.denominator {
        assert!(!rec.mapping.is_empty(), "{}: empty mapping", rec.source_id);
        if rec.mapping == "card" {
            assert!(rec.normalized_concept_id.is_some(), "{}: card w/o concept id", rec.source_id);
        } else {
            assert!(
                rec.normalized_concept_id.is_none(),
                "{}: non-card mapping must have null concept id",
                rec.source_id
            );
        }
    }
}

/// Metrics-shape gate: coverage must be measured against the full
/// reviewed denominator, so it can be BELOW 100% — the old circular metric was
/// tautologically 100%. Assert the honest shape, not a pinned number.
#[test]
fn completeness_metrics_are_not_circular() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let pack = language_pack::generate(&root).expect("generate");
    let c = language_pack::completeness::compute(&pack, &pack.denominator_report);

    // Uncarded, in-scope card-bearing concepts exist and lower coverage.
    assert!(
        *c.mapping_counts.get("uncarded").unwrap_or(&0) > 0,
        "there must be uncarded in-scope concepts (the honest low-coverage state)"
    );
    assert!(
        c.card_coverage.numerator < c.card_coverage.denominator,
        "card_coverage must be below 100% against the full denominator (was {}/{})",
        c.card_coverage.numerator,
        c.card_coverage.denominator
    );
    assert!(
        c.user_facing_syntax_coverage.numerator < c.user_facing_syntax_coverage.denominator,
        "user-facing syntax coverage must be a real gap, not 100%"
    );
    // The metamodel/SHACL surface is no longer an opaque blocked slot —
    // every metamodel class (182) and distinct constrained shape (175) carries a
    // reviewed disposition. Assert the complete accounting, and that no metamodel/
    // SHACL concept remains `block`-mapped (the old opaque state).
    let mm_total: usize = c.metamodel_disposition.values().sum();
    let shacl_total: usize = c.shacl_disposition.values().sum();
    assert_eq!(mm_total, 182, "all 182 metamodel classes must be dispositioned");
    assert_eq!(shacl_total, 175, "all 175 distinct constrained shapes must be dispositioned");
    // The common case is fold-and-enrich (a concept card carries the metaclass
    // facet), never a duplicate card.
    assert!(
        *c.metamodel_disposition.get("fold-enriched").unwrap_or(&0) > 100,
        "most metamodel classes fold onto an existing concept card and enrich it"
    );
    for rec in &pack.denominator {
        if matches!(rec.source_kind.as_str(), "metamodel" | "shacl") {
            assert_ne!(
                rec.mapping, "block",
                "{}: metamodel/SHACL concept must be reviewed, never left blocked",
                rec.source_id
            );
        }
    }
}

/// Finding 6: every normative card carries a structured normative locator. (The
/// generator already fails hard on a missing locator; this pins the invariant.)
#[test]
fn every_normative_card_has_a_locator() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let pack = language_pack::generate(&root).expect("generate");
    let missing = report::cards_without_locator(&pack.cards);
    assert!(missing.is_empty(), "normative cards without a locator: {missing:?}");
}

/// Finding 7: a nested concept's grammar dependencies resolve transitively
/// through non-carded helper rules to its real parent-family cards.
#[test]
fn dependency_expansion_resolves_through_helpers() {
    let root = repo_root();
    if !language_pack::generation_sources_present(&root) {
        eprintln!("SKIP: sources absent (fetch references, then cargo run -p spec-index)");
        return;
    }
    let pack = language_pack::generate(&root).expect("generate");
    let dep_map = language_pack::retrieval::build_dependency_map(
        &pack.cards,
        &pack.aliases,
        &pack.rule_refs,
    );
    let tu = dep_map
        .cards
        .get("sysml.behavior.transition-usage")
        .expect("transition-usage in dep map");
    assert!(
        !tu.grammar_dependencies.is_empty(),
        "transition-usage must expand to non-empty grammar dependencies"
    );
    // Its transition guards/effects are expressions and actions/states — those
    // parent-family cards must appear via helper-fold resolution.
    let has = |p: &str| tu.grammar_dependencies.iter().any(|d| d.starts_with(p));
    assert!(has("kerml.expression."), "must reach an expression parent card");
    assert!(has("sysml.behavior."), "must reach a behavior (action/state) parent card");
}

/// The schema validator itself catches a deliberately malformed card (guards
/// against a validator that rubber-stamps everything).
#[test]
fn schema_validator_rejects_bad_records() {
    let root = repo_root();
    let schemas = schema::SchemaSet::load(&root).unwrap();
    // Missing required fields + illegal id pattern.
    let bad = serde_json::json!({
        "schema_version": "1",
        "id": "NOT.a.valid.id.pattern",
        "title": "x"
    });
    assert!(
        schemas.validate("language-card.schema.json", &bad).is_err(),
        "validator must reject a malformed card"
    );

    // A support object with an out-of-enum axis value.
    let bad_support = serde_json::json!({
        "parse": "definitely-not-a-support-value",
        "lower": "unknown", "resolve": "unknown", "elaborate": "unknown",
        "validate": "unknown", "execute": "unknown", "format": "unknown", "lsp": "unknown"
    });
    assert!(schemas
        .validate("support-status.schema.json", &serde_json::json!({
            "axes": bad_support, "derived_at_commit": "abc1234"
        }))
        .is_err());
}
