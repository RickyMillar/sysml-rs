//! Deterministic, atomic pack export.
//!
//! Rules: UTF-8, LF, stable key/record order, no wall-clock or git commit in
//! content files, canonical repo-relative locators. Writes to a temp dir, then
//! renames over the target only after all files land. Two clean runs at the
//! same commit produce an identical tree hash.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::denominator::DenominatorReport;
use super::report::Report;
use super::support::EvidenceRecord;
use super::{cards::LanguageCard, concepts::DenominatorRecord, examples::Example, manifest::Manifest};
use super::LpError;

/// Everything a generated pack contains, in memory.
pub struct Pack {
    pub manifest: Manifest,
    pub cards: Vec<LanguageCard>,
    pub examples: Vec<Example>,
    pub denominator: Vec<DenominatorRecord>,
    /// Published 725->650 collapse + §6.2 merge/split report.
    pub denominator_report: DenominatorReport,
    pub aliases: std::collections::BTreeMap<String, String>,
    /// Grammar rule-ref graph (rule name -> the rule names it directly
    /// references), used to resolve helper `rule_dependencies` transitively to
    /// carded concepts in the dependency-expansion map.
    pub rule_refs: std::collections::BTreeMap<String, Vec<String>>,
    /// Machine-readable support evidence, carried through export unchanged
    /// (the evidence gate in sysml-spec-tests is its source of truth).
    pub evidence: Vec<EvidenceRecord>,
    /// Known-gap registry (drives the `tooling.implementation.*` cards).
    pub known_gaps: Vec<super::known_gaps::KnownGap>,
    pub report: Report,
}

/// Canonical JSON for one record: pretty, LF, trailing newline. Struct field
/// order is stable (serde preserves declaration order), so output is
/// reproducible.
fn canonical_json<T: Serialize>(value: &T) -> Result<String, LpError> {
    let mut s = serde_json::to_string_pretty(value)
        .map_err(|e| LpError::Other(format!("serialize: {e}")))?;
    s.push('\n');
    Ok(s)
}

/// Compact single-line JSON (for JSONL rows).
fn compact_json<T: Serialize>(value: &T) -> Result<String, LpError> {
    serde_json::to_string(value).map_err(|e| LpError::Other(format!("serialize: {e}")))
}

fn write_file(path: &Path, contents: &str) -> Result<(), LpError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| LpError::Io(format!("mkdir {}: {e}", parent.display())))?;
    }
    std::fs::write(path, contents).map_err(|e| LpError::Io(format!("write {}: {e}", path.display())))
}

/// Aggregate tree hash over `dir`: for every file (sorted by forward-slash
/// relative path), feed `<relpath>\0<filehash>\n` into one SHA-256. Same
/// convention as `spec-drop.toml`'s directory hashes.
pub fn tree_hash(dir: &Path, exclude: &[&str]) -> Result<String, LpError> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    collect_files(dir, dir, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut acc: Vec<u8> = Vec::new();
    for (rel, abs) in entries {
        if exclude.contains(&rel.as_str()) {
            continue;
        }
        let bytes = std::fs::read(&abs).map_err(|e| LpError::Io(format!("read {}: {e}", abs.display())))?;
        acc.extend_from_slice(rel.as_bytes());
        acc.push(0);
        acc.extend_from_slice(crate::sha256_hex(&bytes).as_bytes());
        acc.push(b'\n');
    }
    Ok(crate::sha256_hex(&acc))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), LpError> {
    for entry in std::fs::read_dir(dir).map_err(|e| LpError::Io(format!("readdir {}: {e}", dir.display())))? {
        let entry = entry.map_err(|e| LpError::Io(format!("entry: {e}")))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| LpError::Other(format!("strip_prefix: {e}")))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}

/// Write the pack into a fresh temp dir, compute its tree hash, then atomically
/// replace `out_dir`. Returns the tree hash (excludes `report.json`, which
/// carries the hash itself).
pub fn export_pack(pack: &Pack, out_dir: &Path) -> Result<String, LpError> {
    let parent = out_dir
        .parent()
        .ok_or_else(|| LpError::Other("output dir has no parent".to_owned()))?;
    let tmp = parent.join(".language-pack.tmp");
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|e| LpError::Io(format!("rm tmp: {e}")))?;
    }

    // Content files (stable).
    write_file(&tmp.join("manifest.json"), &canonical_json(&pack.manifest)?)?;

    let mut cards_sorted = pack.cards.clone();
    cards_sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for card in &cards_sorted {
        write_file(&tmp.join(format!("cards/{}.json", card.id)), &canonical_json(card)?)?;
    }
    // One JSONL chunk per card (retrieval index), sorted by ID.
    let mut jsonl = String::new();
    for card in &cards_sorted {
        jsonl.push_str(&compact_json(card)?);
        jsonl.push('\n');
    }
    write_file(&tmp.join("indexes/cards.jsonl"), &jsonl)?;

    let mut examples_sorted = pack.examples.clone();
    examples_sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for ex in &examples_sorted {
        write_file(&tmp.join(format!("examples/{}.json", ex.id)), &canonical_json(ex)?)?;
    }

    write_file(&tmp.join("indexes/aliases.json"), &canonical_json(&pack.aliases)?)?;

    // Denominator records (JSONL).
    let mut denom = pack.denominator.clone();
    denom.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    let mut denom_jsonl = String::new();
    for rec in &denom {
        denom_jsonl.push_str(&compact_json(rec)?);
        denom_jsonl.push('\n');
    }
    write_file(&tmp.join("indexes/denominator.jsonl"), &denom_jsonl)?;

    // Published denominator report (725->650 collapse + §6.2 merge/split).
    write_file(
        &tmp.join("indexes/denominator-report.json"),
        &canonical_json(&pack.denominator_report)?,
    )?;

    // Completeness report: numerator/denominator
    // metrics, family totals, covered/unknown/blocked/excluded lists. Emitted as
    // both machine-readable JSON and human-readable Markdown.
    let completeness = super::completeness::compute(pack, &pack.denominator_report);
    write_file(&tmp.join("completeness.json"), &canonical_json(&completeness)?)?;
    write_file(
        &tmp.join("completeness.md"),
        &super::completeness::to_markdown(&completeness),
    )?;

    // Support evidence (carried through unchanged; sorted deterministically).
    let mut evidence = pack.evidence.clone();
    evidence.sort_by(|a, b| {
        (&a.card_id, &a.axis, &a.case_id).cmp(&(&b.card_id, &b.axis, &b.case_id))
    });
    let mut ev_jsonl = String::new();
    for rec in &evidence {
        ev_jsonl.push_str(&compact_json(rec)?);
        ev_jsonl.push('\n');
    }
    write_file(&tmp.join("evidence.jsonl"), &ev_jsonl)?;

    // Known-gap registry (sorted by id for determinism).
    let mut gaps = pack.known_gaps.clone();
    gaps.sort_by(|a, b| a.id.cmp(&b.id));
    write_file(&tmp.join("known-gaps.json"), &canonical_json(&gaps)?)?;

    // Retrieval layer: one-card-per-chunk export, a
    // BM25-ready keyword index, a dependency-aware expansion map, and a
    // chunk-budget report. All are pure functions of the sorted card corpus +
    // alias table, so they regenerate deterministically.
    let chunks = super::retrieval::build_chunks(&cards_sorted, &examples_sorted);
    let mut chunk_jsonl = String::new();
    for chunk in &chunks {
        chunk_jsonl.push_str(&compact_json(chunk)?);
        chunk_jsonl.push('\n');
    }
    write_file(&tmp.join("retrieval/chunks.jsonl"), &chunk_jsonl)?;

    let keyword_index = super::retrieval::build_keyword_index(&cards_sorted, &chunks);
    write_file(&tmp.join("indexes/keywords.json"), &canonical_json(&keyword_index)?)?;

    let dep_map =
        super::retrieval::build_dependency_map(&cards_sorted, &pack.aliases, &pack.rule_refs);
    write_file(&tmp.join("indexes/dependencies.json"), &canonical_json(&dep_map)?)?;

    let card_json_tokens: Vec<usize> = cards_sorted
        .iter()
        .map(|c| canonical_json(c).map(|s| s.chars().count().div_ceil(4)))
        .collect::<Result<Vec<_>, _>>()?;
    let retrieval_report = super::retrieval::build_report(
        &chunks,
        &card_json_tokens,
        keyword_index.terms.len(),
        keyword_index.average_chunk_tokens,
    );
    write_file(
        &tmp.join("retrieval/retrieval-report.json"),
        &canonical_json(&retrieval_report)?,
    )?;

    // Held-out executable evaluations. Authored tables, emitted as
    // JSONL; the sysml-spec-tests `language_pack_evals` gate proves every
    // reference answer passes its own check.
    for (name, jsonl) in super::evals::export_evals()? {
        write_file(&tmp.join(format!("evals/{name}.jsonl")), &jsonl)?;
    }

    // Held-out retrieval query set. Authored queries with
    // expected-card answer keys; the sysml-spec-tests `retrieval_eval` gate runs
    // the deterministic BM25 retriever over the shipped index against these and
    // asserts a recall/MRR floor.
    write_file(
        &tmp.join("evals/retrieval.jsonl"),
        &super::retriever::export_retrieval_queries()?,
    )?;

    // Tree hash over everything written so far (report.json not yet present).
    let hash = tree_hash(&tmp, &[])?;

    // Report carries the tree hash; excluded from the hash it reports.
    let mut report = pack.report.clone();
    report.tree_hash = hash.clone();
    write_file(&tmp.join("report.json"), &canonical_json(&report)?)?;

    // Atomic swap.
    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir).map_err(|e| LpError::Io(format!("rm out: {e}")))?;
    }
    std::fs::rename(&tmp, out_dir).map_err(|e| LpError::Io(format!("rename tmp->out: {e}")))?;
    Ok(hash)
}

/// Serialize a card to a `serde_json::Value` for schema validation.
pub fn to_value<T: Serialize>(value: &T) -> Result<Value, LpError> {
    serde_json::to_value(value).map_err(|e| LpError::Other(format!("to_value: {e}")))
}

/// Read an evidence JSONL file (the tracked seed, or a pack's exported copy).
/// A missing file yields an empty vector (the bootstrap case, before any
/// evidence is emitted).
pub fn read_evidence_file(path: &Path) -> Result<Vec<EvidenceRecord>, LpError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(LpError::Io(format!("read {}: {e}", path.display()))),
    };
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        out.push(
            serde_json::from_str(line)
                .map_err(|e| LpError::Other(format!("parse evidence record: {e}")))?,
        );
    }
    Ok(out)
}
