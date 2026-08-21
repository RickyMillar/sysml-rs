//! Executable validation gate for the language-pack examples.
//!
//! Reads the exported example records under
//! `references/sysmlv2/derived/language-pack/examples/`, runs each through the
//! real tree-sitter parser + resolver + semantic validators, and asserts:
//!
//! - every positive/composed example parses with zero syntax errors and lowers
//!   to the ModelGraph element kinds it declares;
//! - every negative example fails at its declared phase for its declared reason
//!   (a parse error, an unresolved reference, or a specific `S0xx` validator),
//!   never "failed somehow".
//!
//! It then emits machine-readable evidence records (the `evidence-record`
//! schema) keyed to a stable spec-drop evidence epoch, which
//! `tools/spec-index` ingests to derive each card's support axes. The pack's
//! `evidence.jsonl` is regenerated here and byte-compared. Set
//! `SYSML_LP_UPDATE_EVIDENCE=1` to rewrite the tracked evidence seed
//! (`tools/spec-index/data/evidence.jsonl`) after an intended change, then
//! regenerate the pack so it ships the new support axes.
//!
//! The pack directory is not tracked in git; this gate skips with a message
//! when it is absent and runs fully when present.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use spec_index::language_pack::{
    default_output_dir, evidence_epoch, evidence_seed_path, manifest, repo_root,
    support::{self, EvidenceRecord},
};
use sysml_core::elaborate::elaborate;
use sysml_core::ElementKind;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_span::Severity;

const GATE: &str = "sysml-spec-tests::language_card_examples";

struct Analysis {
    parse_errors: usize,
    kinds: BTreeSet<String>,
    resolved: usize,
    unresolved: usize,
    sem_codes: BTreeSet<String>,
}

/// Parse a set of `(name, source)` fragments through the full pipeline.
fn analyze(files: &[(String, String)]) -> Analysis {
    let parser = TreeSitterParser::new();
    let sysml_files: Vec<SysmlFile> = files
        .iter()
        .map(|(n, s)| SysmlFile::new(n, s))
        .collect();
    let mut result = parser.parse(&sysml_files);
    let parse_errors = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let kinds: BTreeSet<String> = ElementKind::iter()
        .filter(|k| !result.graph.element_ids_by_kind(k).is_empty())
        .map(|k| k.as_str().to_owned())
        .collect();
    let res = result.resolve();
    let _ = elaborate(&mut result.graph);
    let sem_codes: BTreeSet<String> = sysml_core::validate_semantic(&result.graph)
        .into_iter()
        .map(|e| e.rule_id.to_string())
        .collect();
    Analysis {
        parse_errors,
        kinds,
        resolved: res.resolved_count,
        unresolved: res.unresolved_count,
        sem_codes,
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Read the exported example fragments for an example JSON value.
fn example_files(v: &Value) -> Vec<(String, String)> {
    if let Some(files) = v.get("files").and_then(Value::as_array) {
        files
            .iter()
            .map(|f| {
                (
                    str_field(f, "name").to_owned(),
                    str_field(f, "source").to_owned(),
                )
            })
            .collect()
    } else {
        vec![("example.sysml".to_owned(), str_field(v, "source").to_owned())]
    }
}

fn record(card_id: &str, axis: &str, epoch: &str, case_id: &str) -> EvidenceRecord {
    EvidenceRecord {
        card_id: card_id.to_owned(),
        axis: axis.to_owned(),
        commit: epoch.to_owned(),
        gate: GATE.to_owned(),
        case_id: case_id.to_owned(),
        result: "pass".to_owned(),
        observed_kinds: Vec::new(),
        known_gap_ref: None,
        // This evidence justifies a `validated` support value for the axis.
        // (Also required for schema conformance: the evidence-record schema's
        // `if justifies==unsupported` clause is vacuously true when `justifies`
        // is absent, so a present non-`unsupported` value is needed.)
        justifies: Some("validated".to_owned()),
    }
}

/// Serialize evidence records the same way `tools/spec-index` export does:
/// compact JSON, one per line, sorted by (card_id, axis, case_id).
fn evidence_jsonl(mut records: Vec<EvidenceRecord>) -> String {
    records.sort_by(|a, b| {
        (&a.card_id, &a.axis, &a.case_id).cmp(&(&b.card_id, &b.axis, &b.case_id))
    });
    let mut out = String::new();
    for r in &records {
        out.push_str(&serde_json::to_string(r).unwrap());
        out.push('\n');
    }
    out
}

#[test]
fn language_card_examples_validate_and_emit_evidence() {
    let repo = repo_root();
    let pack_dir = default_output_dir(&repo);
    let examples_dir = pack_dir.join("examples");
    if !examples_dir.is_dir() {
        eprintln!(
            "SKIP: no language pack at {} (run cargo run -p spec-index, then \
             cargo run -p spec-index -- language-pack)",
            pack_dir.display()
        );
        return;
    }

    let manifest = manifest::resolve_manifest(&repo).expect("manifest");
    let epoch = evidence_epoch(&manifest);

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&examples_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();

    let mut records: Vec<EvidenceRecord> = Vec::new();
    let mut positives = 0usize;
    let mut negatives = 0usize;
    let mut composed = 0usize;

    for path in &paths {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let id = str_field(&v, "id").to_owned();
        let card_id = str_field(&v, "card_id").to_owned();
        let kind = str_field(&v, "kind");

        match kind {
            "positive" | "composed" => {
                if kind == "composed" {
                    composed += 1;
                } else {
                    positives += 1;
                }
                let files = example_files(&v);
                let a = analyze(&files);
                assert_eq!(
                    a.parse_errors, 0,
                    "{id}: positive/composed example must parse with 0 syntax errors"
                );
                let expected = v
                    .get("expected")
                    .and_then(|e| e.get("element_kinds"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for k in &expected {
                    let k = k.as_str().unwrap();
                    assert!(
                        a.kinds.contains(k),
                        "{id}: expected element kind `{k}` not lowered (present: {:?})",
                        a.kinds
                    );
                }
                // parse + lower evidence.
                records.push(record(&card_id, "parse", &epoch, &id));
                records.push(record(&card_id, "lower", &epoch, &id));
                // resolve evidence only when the example actually resolves refs.
                if a.resolved > 0 && a.unresolved == 0 {
                    records.push(record(&card_id, "resolve", &epoch, &id));
                }
            }
            "negative" => {
                negatives += 1;
                let files = example_files(&v);
                let a = analyze(&files);
                let ef = v.get("expected_failure").expect("negative needs expected_failure");
                let phase = str_field(ef, "phase");
                match phase {
                    "parse" => {
                        assert!(
                            a.parse_errors > 0,
                            "{id}: negative must fail at parse (got 0 syntax errors)"
                        );
                        records.push(record(&card_id, "parse", &epoch, &id));
                    }
                    "resolve" => {
                        assert_eq!(
                            a.parse_errors, 0,
                            "{id}: resolve-phase negative must parse cleanly first"
                        );
                        assert!(
                            a.unresolved > 0,
                            "{id}: negative must leave an unresolved reference"
                        );
                        records.push(record(&card_id, "resolve", &epoch, &id));
                    }
                    "validate" => {
                        let code = str_field(ef, "diagnostic_code");
                        assert!(!code.is_empty(), "{id}: validate negative needs diagnostic_code");
                        assert_eq!(a.parse_errors, 0, "{id}: validate negative must parse cleanly");
                        assert_eq!(a.unresolved, 0, "{id}: validate negative must resolve cleanly");
                        assert!(
                            a.sem_codes.contains(code),
                            "{id}: expected validator `{code}` to fire (got {:?})",
                            a.sem_codes
                        );
                        records.push(record(&card_id, "validate", &epoch, &id));
                    }
                    other => panic!("{id}: unknown negative phase `{other}`"),
                }
            }
            other => panic!("{id}: unknown example kind `{other}`"),
        }
    }

    // Every card is exercised: one positive + one negative each, >=4 composed.
    assert!(positives >= 20, "expected >=20 positive examples (got {positives})");
    assert_eq!(positives, negatives, "each card needs one positive and one negative");
    assert!(composed >= 4, "at least four composed examples required (got {composed})");

    let update_mode = std::env::var("SYSML_LP_UPDATE_EVIDENCE").is_ok();

    // Compare-back (the load-bearing property): the support axes SHIPPED in
    // every card must equal the axes re-derived from THIS live run's evidence.
    // A card claiming validated/partial that the current implementation no
    // longer reproduces fails here — strictly stronger than commit-pinning,
    // because "validated" is enforced by a gate green at every commit rather
    // than trusted from a stored record. Skipped only in evidence-refresh mode,
    // where the shipped support is intentionally stale until the pack is
    // regenerated from the new evidence.
    let cards_dir = pack_dir.join("cards");
    if !update_mode {
    let mut card_paths: Vec<PathBuf> = std::fs::read_dir(&cards_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    card_paths.sort();
    for path in &card_paths {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let id = str_field(&v, "id").to_owned();
        let shipped = v.get("support").expect("card has support");
        let derived = support::derive_axes(&id, &epoch, &records);
        for (axis, live) in [
            ("parse", &derived.parse),
            ("lower", &derived.lower),
            ("resolve", &derived.resolve),
            ("elaborate", &derived.elaborate),
            ("validate", &derived.validate),
            ("execute", &derived.execute),
            ("format", &derived.format),
            ("lsp", &derived.lsp),
        ] {
            let shipped_val = shipped.get(axis).and_then(Value::as_str).unwrap_or_default();
            assert_eq!(
                shipped_val,
                live.as_str(),
                "card {id} axis `{axis}`: pack ships `{shipped_val}` but a live re-run derives \
                 `{live}` — the pack claims support the current implementation no longer \
                 reproduces. Refresh evidence (SYSML_LP_UPDATE_EVIDENCE=1) and regenerate the pack."
                );
            }
        }
    }

    let fresh = evidence_jsonl(records);
    if update_mode {
        // Rewrite the tracked seed (the generator's input); the pack's copy is
        // refreshed by the next `cargo run -p spec-index -- language-pack`.
        let seed_path = evidence_seed_path(&repo);
        std::fs::write(&seed_path, &fresh).unwrap();
        println!("wrote {} ({} bytes)", seed_path.display(), fresh.len());
        println!("now regenerate the pack: cargo run -p spec-index -- language-pack");
    } else {
        let evidence_path = pack_dir.join("evidence.jsonl");
        let shipped = std::fs::read_to_string(&evidence_path).unwrap_or_default();
        assert_eq!(
            shipped, fresh,
            "evidence.jsonl is stale — re-run with SYSML_LP_UPDATE_EVIDENCE=1 (rewrites the \
             tracked seed tools/spec-index/data/evidence.jsonl), then regenerate the pack \
             with cargo run -p spec-index -- language-pack"
        );
    }
}
