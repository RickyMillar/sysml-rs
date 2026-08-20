//! Candidate regression-report scoring (LPFIX-7 finding 8, retrieval-contract §5).
//!
//! This is the *candidate-model* side of the eval story, kept deliberately
//! separate from the deterministic gates: it takes a model's answers to any of
//! the held-out datasets and emits the regression-report JSON shaped by
//! `retrieval-contract.md` §5 — **without asserting** the PROPOSED
//! D-RAG-THRESHOLDS bar. Deterministic gates fail CI; candidate scoring only
//! reports, because the thresholds are human-gated (decision register
//! D-RAG-THRESHOLDS, status PROPOSED).
//!
//! Scoring reuses the shared pipeline ([`crate::eval_pipeline::analyze`]) so a
//! generation/repair verdict means exactly what the reference-answer gate means.
//! The functions are IO-free (they take already-parsed rows + answer maps) so
//! they unit-test without touching disk.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::eval_pipeline::analyze;

/// PROPOSED candidate-model thresholds (decision register D-RAG-THRESHOLDS).
/// Reported as `thresholds_met` booleans, never asserted here.
pub mod thresholds {
    pub const GENERATION_PARSE_RATE: f64 = 0.95;
    pub const EXPLANATION_PRIMARY_HIT: f64 = 0.80;
    pub const EXPLANATION_FULL_HIT: f64 = 0.60;
    pub const REPAIR_FIX_RATE: f64 = 0.80;
    pub const SUPPORT_ACCURACY: f64 = 0.90;
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

/// Score generation answers: `id -> candidate SysML source`. A row counts as
/// green only when the candidate supplied an answer that parses with 0 syntax
/// errors (a missing answer scores 0, keeping the denominator honest).
pub fn score_generation(rows: &[Value], answers: &BTreeMap<String, String>) -> Value {
    let n = rows.len();
    let mut parse_green = 0usize;
    for r in rows {
        if let Some(src) = answers.get(str_field(r, "id")) {
            if analyze(src).parses() {
                parse_green += 1;
            }
        }
    }
    serde_json::json!({ "n": n, "parse_green": parse_green, "parse_rate": rate(parse_green, n) })
}

/// Score explanation answers: `id -> cited card ids`. `primary_hit` = the answer
/// cited at least one expected card; `full_hit` = it cited all of them.
pub fn score_explanation(rows: &[Value], answers: &BTreeMap<String, Vec<String>>) -> Value {
    let n = rows.len();
    let (mut primary, mut full) = (0usize, 0usize);
    for r in rows {
        let expected = strs(r, "expected_card_ids");
        let cited = answers.get(str_field(r, "id")).cloned().unwrap_or_default();
        let hits = expected.iter().filter(|e| cited.contains(e)).count();
        if hits >= 1 {
            primary += 1;
        }
        if !expected.is_empty() && hits == expected.len() {
            full += 1;
        }
    }
    serde_json::json!({ "n": n, "primary_hit": primary, "full_hit": full })
}

/// Score repair answers: `id -> candidate fixed source`. A row counts as fixed
/// when the candidate's fix is clean *through the phase it repairs* — parse for
/// parse-phase, parse+resolve for resolve-phase, and parse+resolve without the
/// declared validator firing for validate-phase.
pub fn score_repair(rows: &[Value], answers: &BTreeMap<String, String>) -> Value {
    let n = rows.len();
    let mut fixed = 0usize;
    for r in rows {
        let Some(src) = answers.get(str_field(r, "id")) else {
            continue;
        };
        let a = analyze(src);
        let ok = match str_field(r, "expected_phase") {
            "parse" => a.parses(),
            "resolve" => a.resolves(),
            "validate" => {
                let code = str_field(r, "diagnostic_code");
                a.resolves() && !a.sem_codes.contains(code)
            }
            _ => false,
        };
        if ok {
            fixed += 1;
        }
    }
    serde_json::json!({ "n": n, "fixed": fixed })
}

/// Score support-discrimination answers: `id -> reported support value`. Correct
/// when the reported value matches the row's `expected_support` (which the
/// reference gate already pins to the card's live axis).
pub fn score_support(rows: &[Value], answers: &BTreeMap<String, String>) -> Value {
    let n = rows.len();
    let mut correct = 0usize;
    for r in rows {
        if let Some(reported) = answers.get(str_field(r, "id")) {
            if reported == str_field(r, "expected_support") {
                correct += 1;
            }
        }
    }
    serde_json::json!({ "n": n, "correct": correct })
}

fn rate(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

fn num(v: &Value, key: &str) -> usize {
    v.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

/// The primary rate for a scored dataset (the figure a threshold / regression
/// check keys on). Returns `None` for an unknown dataset name.
pub fn primary_rate(dataset: &str, score: &Value) -> Option<f64> {
    let n = num(score, "n");
    match dataset {
        "generation" => Some(rate(num(score, "parse_green"), n)),
        "explanation" => Some(rate(num(score, "primary_hit"), n)),
        "repair" => Some(rate(num(score, "fixed"), n)),
        "support-discrimination" => Some(rate(num(score, "correct"), n)),
        _ => None,
    }
}

/// Whether a scored dataset meets its PROPOSED threshold (informational only).
pub fn meets_threshold(dataset: &str, score: &Value) -> Option<bool> {
    let n = num(score, "n");
    match dataset {
        "generation" => Some(rate(num(score, "parse_green"), n) >= thresholds::GENERATION_PARSE_RATE),
        "explanation" => Some(
            rate(num(score, "primary_hit"), n) >= thresholds::EXPLANATION_PRIMARY_HIT
                && rate(num(score, "full_hit"), n) >= thresholds::EXPLANATION_FULL_HIT,
        ),
        "repair" => Some(rate(num(score, "fixed"), n) >= thresholds::REPAIR_FIX_RATE),
        "support-discrimination" => {
            Some(rate(num(score, "correct"), n) >= thresholds::SUPPORT_ACCURACY)
        }
        _ => None,
    }
}

/// The §5 regression report. `datasets` keys are dataset names; `thresholds_met`
/// mirrors them with the PROPOSED-bar booleans; `regressions_vs_previous` lists
/// datasets whose primary rate dropped against a supplied previous report.
#[derive(Debug, Clone, Serialize)]
pub struct RegressionReport {
    pub pack_tree_hash: String,
    pub spec_drop: String,
    pub model: String,
    pub datasets: BTreeMap<String, Value>,
    pub thresholds_met: BTreeMap<String, bool>,
    pub regressions_vs_previous: Vec<String>,
    /// Explicit provenance note: this is candidate scoring, thresholds are not
    /// enforced (they are PROPOSED / human-gated).
    pub note: String,
}

impl RegressionReport {
    /// Assemble a report from scored datasets and pack identity. `previous` is an
    /// optional prior report JSON to diff primary rates against.
    pub fn assemble(
        pack_tree_hash: String,
        spec_drop: String,
        model: String,
        datasets: BTreeMap<String, Value>,
        previous: Option<&Value>,
    ) -> Self {
        let mut thresholds_met = BTreeMap::new();
        for (name, score) in &datasets {
            if let Some(met) = meets_threshold(name, score) {
                thresholds_met.insert(name.clone(), met);
            }
        }

        let mut regressions_vs_previous = Vec::new();
        if let Some(prev) = previous {
            let prev_sets = prev.get("datasets");
            for (name, score) in &datasets {
                let cur = primary_rate(name, score);
                let old = prev_sets
                    .and_then(|d| d.get(name))
                    .and_then(|s| primary_rate(name, s));
                if let (Some(cur), Some(old)) = (cur, old) {
                    if cur + 1e-9 < old {
                        regressions_vs_previous.push(format!("{name}: {old:.3} -> {cur:.3}"));
                    }
                }
            }
        }

        Self {
            pack_tree_hash,
            spec_drop,
            model,
            datasets,
            thresholds_met,
            regressions_vs_previous,
            note: "Candidate-model scoring. thresholds_met reflects the PROPOSED \
                   D-RAG-THRESHOLDS bar and is NOT enforced (human-gated); only the \
                   deterministic retrieval/reference gates fail CI."
                .to_owned(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn row(json: Value) -> Value {
        json
    }

    #[test]
    fn generation_scores_parse_green() {
        let rows = vec![
            row(serde_json::json!({"id": "a"})),
            row(serde_json::json!({"id": "b"})),
        ];
        let mut ans = BTreeMap::new();
        ans.insert("a".to_owned(), "package P { part def X; }".to_owned());
        ans.insert("b".to_owned(), "package P { part def ".to_owned()); // broken
        let s = score_generation(&rows, &ans);
        assert_eq!(num(&s, "n"), 2);
        assert_eq!(num(&s, "parse_green"), 1);
        assert!((primary_rate("generation", &s).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn explanation_primary_and_full() {
        let rows = vec![row(serde_json::json!({
            "id": "x", "expected_card_ids": ["c1", "c2"]
        }))];
        let mut ans = BTreeMap::new();
        ans.insert("x".to_owned(), vec!["c1".to_owned()]); // primary only
        let s = score_explanation(&rows, &ans);
        assert_eq!(num(&s, "primary_hit"), 1);
        assert_eq!(num(&s, "full_hit"), 0);
    }

    #[test]
    fn support_matches_expected() {
        let rows = vec![
            row(serde_json::json!({"id": "s1", "expected_support": "validated"})),
            row(serde_json::json!({"id": "s2", "expected_support": "unknown"})),
        ];
        let mut ans = BTreeMap::new();
        ans.insert("s1".to_owned(), "validated".to_owned());
        ans.insert("s2".to_owned(), "validated".to_owned()); // wrong
        let s = score_support(&rows, &ans);
        assert_eq!(num(&s, "correct"), 1);
    }

    #[test]
    fn regression_detected_against_previous() {
        let datasets: BTreeMap<String, Value> =
            [("repair".to_owned(), serde_json::json!({"n": 4, "fixed": 2}))]
                .into_iter()
                .collect();
        let previous = serde_json::json!({"datasets": {"repair": {"n": 4, "fixed": 4}}});
        let report = RegressionReport::assemble(
            "hash".to_owned(),
            "2025-04".to_owned(),
            "m".to_owned(),
            datasets,
            Some(&previous),
        );
        assert_eq!(report.regressions_vs_previous.len(), 1);
        assert!(report.regressions_vs_previous[0].contains("repair"));
        // thresholds_met is present but never asserted as a gate.
        assert_eq!(report.thresholds_met.get("repair"), Some(&false));
    }
}
