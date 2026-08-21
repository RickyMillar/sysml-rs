//! Executable validation gate for the held-out evaluation datasets
//! under `references/sysmlv2/derived/language-pack/evals/`.
//!
//! The datasets are held-out (authored independently of the pack examples), but
//! their *reference answers* must still be correct. This gate proves it, so the
//! evals can never silently rot:
//!
//! - **explanation** — every cited `expected_card_ids` resolves to a real card;
//!   key facts are present; and every `normative_locators` entry (`"<Document>
//!   <clause>"`) appears in a cited card's `normative_clauses`, so the answer key
//!   is anchored to the spec, not merely to non-empty prose.
//! - **generation** — every `reference_solution` parses with zero syntax errors
//!   through the real tree-sitter parser, and additionally resolves clean when
//!   the row declares `check_resolve` (the runner checks all declared phases,
//!   not just parse); cited cards exist.
//! - **repair** — every `broken_source` fails at its declared phase for its
//!   declared reason (parse error / unresolved ref / specific S0xx), and every
//!   `fixed_source` is clean through that phase; cited cards exist.
//! - **support-discrimination** — the referenced card exists and its live
//!   support axis equals `expected_support`, so the answer key cannot drift
//!   from the pack (same discipline as the evidence gate).
//!
//! The analysis pipeline is the shared `sysml_spec_tests::eval_pipeline::analyze`
//! (one home, also used by the candidate-report harness).
//!
//! The pack directory is not tracked in git; every test here skips with a
//! message when it is absent and runs fully when present.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeSet;

use serde_json::Value;
use spec_index::language_pack::{default_output_dir, repo_root};
use sysml_spec_tests::eval_pipeline::analyze;

/// Clauses of a single card, formatted `"<Document> <clause>"` (the locator
/// form the explanation dataset uses).
fn card_clauses(card_id: &str) -> BTreeSet<String> {
    let path = default_output_dir(&repo_root()).join(format!("cards/{card_id}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read card {}: {e}", path.display()));
    let card: Value = serde_json::from_str(&text).unwrap();
    card.get("normative_clauses")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let doc = c.get("document").and_then(Value::as_str)?;
                    let clause = c.get("clause").and_then(Value::as_str)?;
                    Some(format!("{doc} {clause}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pack_card_ids() -> BTreeSet<String> {
    let dir = default_output_dir(&repo_root()).join("cards");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read cards dir {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            v.get("id").and_then(Value::as_str).unwrap().to_owned()
        })
        .collect()
}

/// The eval dataset rows, or `None` when the (untracked) pack is absent — the
/// caller skips with a message rather than failing.
fn read_jsonl(name: &str) -> Option<Vec<Value>> {
    let path = default_output_dir(&repo_root()).join(format!("evals/{name}.jsonl"));
    if !path.exists() {
        eprintln!(
            "SKIP: no language pack at {} (run cargo run -p spec-index, then \
             cargo run -p spec-index -- language-pack)",
            path.display()
        );
        return None;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    Some(
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
    )
}

fn strs(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn assert_cards_exist(ids: &[String], known: &BTreeSet<String>, ctx: &str) {
    for id in ids {
        assert!(known.contains(id), "{ctx}: cited card `{id}` not in pack");
    }
}

#[test]
fn explanation_dataset_is_wellformed() {
    let Some(rows) = read_jsonl("explanation") else { return };
    let known = pack_card_ids();
    assert!(rows.len() >= 30, "expected >=30 explanation items (got {})", rows.len());
    for r in &rows {
        let id = str_field(r, "id");
        let cards = strs(r, "expected_card_ids");
        assert!(!cards.is_empty(), "{id}: needs >=1 expected_card_ids");
        assert_cards_exist(&cards, &known, id);
        assert!(!str_field(r, "question").is_empty(), "{id}: empty question");
        assert!(!strs(r, "key_facts").is_empty(), "{id}: needs key_facts");

        // Locator grounding: every declared normative locator must appear in
        // one of the cited cards' normative_clauses, so the answer key is
        // anchored to the spec and cannot drift from the cards.
        let locators = strs(r, "normative_locators");
        assert!(!locators.is_empty(), "{id}: needs >=1 normative_locators");
        let mut available: BTreeSet<String> = BTreeSet::new();
        for card in &cards {
            available.extend(card_clauses(card));
        }
        for loc in &locators {
            assert!(
                available.contains(loc),
                "{id}: normative locator `{loc}` is not among the cited cards' clauses {available:?} — \
                 the answer key drifted from the pack; update it"
            );
        }
    }
}

#[test]
fn generation_reference_solutions_pass_declared_phases() {
    let Some(rows) = read_jsonl("generation") else { return };
    let known = pack_card_ids();
    assert!(rows.len() >= 24, "expected >=24 generation items (got {})", rows.len());
    for r in &rows {
        let id = str_field(r, "id");
        let sol = str_field(r, "reference_solution");
        assert!(!sol.is_empty(), "{id}: empty reference_solution");
        let a = analyze(sol);
        assert_eq!(
            a.parse_errors, 0,
            "{id}: reference solution must parse with 0 syntax errors:\n  {sol}"
        );
        // When declared self-contained, the reference must also resolve clean —
        // the runner checks every declared phase, not just parse.
        if r.get("check_resolve").and_then(Value::as_bool).unwrap_or(false) {
            assert_eq!(
                a.unresolved, 0,
                "{id}: reference solution declares check_resolve but left {} unresolved reference(s):\n  {sol}",
                a.unresolved
            );
        }
        assert_cards_exist(&strs(r, "expected_card_ids"), &known, id);
    }
}

#[test]
fn repair_items_fail_broken_and_pass_fixed() {
    let Some(rows) = read_jsonl("repair") else { return };
    let known = pack_card_ids();
    assert!(rows.len() >= 18, "expected >=18 repair items (got {})", rows.len());
    for r in &rows {
        let id = str_field(r, "id");
        let broken = str_field(r, "broken_source");
        let fixed = str_field(r, "fixed_source");
        let phase = str_field(r, "expected_phase");
        assert_cards_exist(&strs(r, "expected_card_ids"), &known, id);

        let b = analyze(broken);
        match phase {
            "parse" => assert!(
                b.parse_errors > 0,
                "{id}: broken source must fail at parse:\n  {broken}"
            ),
            "resolve" => {
                assert_eq!(b.parse_errors, 0, "{id}: resolve-phase broken must parse first");
                assert!(b.unresolved > 0, "{id}: broken source must leave an unresolved ref");
            }
            "validate" => {
                let code = str_field(r, "diagnostic_code");
                assert!(!code.is_empty(), "{id}: validate repair needs diagnostic_code");
                assert_eq!(b.parse_errors, 0, "{id}: validate-phase broken must parse cleanly");
                assert_eq!(b.unresolved, 0, "{id}: validate-phase broken must resolve cleanly");
                assert!(
                    b.sem_codes.contains(code),
                    "{id}: expected validator `{code}` to fire (got {:?})",
                    b.sem_codes
                );
            }
            other => panic!("{id}: unknown expected_phase `{other}`"),
        }

        // The fix must be clean through the phase it repairs.
        let f = analyze(fixed);
        assert_eq!(f.parse_errors, 0, "{id}: fixed source must parse green:\n  {fixed}");
        if phase == "resolve" {
            assert_eq!(f.unresolved, 0, "{id}: fixed source must resolve cleanly");
        }
        if phase == "validate" {
            let code = str_field(r, "diagnostic_code");
            assert!(
                !f.sem_codes.contains(code),
                "{id}: fixed source must NOT trip `{code}` (got {:?})",
                f.sem_codes
            );
        }
    }
}

#[test]
fn support_discrimination_matches_live_pack() {
    let dir = default_output_dir(&repo_root()).join("cards");
    let Some(rows) = read_jsonl("support-discrimination") else { return };
    assert!(rows.len() >= 18, "expected >=18 items (got {})", rows.len());
    let axes: BTreeSet<&str> = [
        "parse", "lower", "resolve", "elaborate", "validate", "execute", "format", "lsp",
    ]
    .into_iter()
    .collect();
    for r in &rows {
        let id = str_field(r, "id");
        let card_id = str_field(r, "concept_card_id");
        let axis = str_field(r, "axis");
        let expected = str_field(r, "expected_support");
        assert!(axes.contains(axis), "{id}: `{axis}` is not a support axis");
        let path = dir.join(format!("{card_id}.json"));
        assert!(path.exists(), "{id}: card `{card_id}` not in pack");
        let card: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let live = card
            .get("support")
            .and_then(|s| s.get(axis))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(
            live, expected,
            "{id}: card `{card_id}` axis `{axis}` is `{live}` in the pack but the eval \
             expects `{expected}` — the dataset drifted from the pack; update it."
        );
        assert!(!str_field(r, "answer_key").is_empty(), "{id}: empty answer_key");
    }
}
