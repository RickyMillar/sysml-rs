//! Deterministic retrieval-evaluation gate.
//!
//! The other language-pack gates prove the *reference answers* are correct;
//! none exercised retrieval itself. This gate runs the deterministic BM25
//! retriever (`spec_index::language_pack::retriever`) over the shipped index
//! against the held-out `evals/retrieval.jsonl` query set and measures:
//!
//! - **top-k expected-card recall** (micro, at k = 1 / 3 / 5 / 10)
//! - **all-expected-cards recall** (queries whose every expected card is in top-k)
//! - **MRR** and rank stats (first-hit reciprocal rank)
//! - **dependency-expansion contribution** — recall with vs. without one-hop
//!   expansion through `dependencies.json`, measured from the single top hit.
//!
//! Thresholds here are a **deterministic regression floor**: pinned *below* the
//! observed values with a comment, they catch a retrieval regression (index
//! drift, tokenizer change, ranking bug) without pretending to be the
//! bar for judging a candidate model (a separate, human-gated question).
//! Because the retriever is deterministic (fixed tokenization, score then id
//! tiebreak), these numbers are stable across platforms.
//!
//! The pack directory is not tracked in git; the test skips with a message
//! when it is absent and runs fully when present.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeMap;

use serde_json::Value;
use spec_index::language_pack::retriever::{evaluate, set_recall, Retriever};
use spec_index::language_pack::{default_output_dir, repo_root};

/// Primary retrieval cutoff for the pinned recall floor.
const K: usize = 10;
/// Tight cutoff at which the one-hop dependency expansion actually contributes
/// (at k=10 BM25 already saturates recall, so expansion adds nothing there).
const K_EXP: usize = 1;

// --- Pinned deterministic floors -------------------------------------------
// Observed on the generated pack (67 queries, 70 expected cards; see the
// printed report from this test):
//   micro recall@1=0.757, @3=0.957, @5=0.986, @10=1.000; all-expected@10=1.000;
//   MRR=0.874; dependency-expansion contribution from the single top hit =
//   +7 expected cards (base recall@1 53/70 -> expanded 60/70), i.e. one-hop
//   expansion recovers 7 of the 17 answers the top-1 ranker missed. Floors are
//   pinned conservatively below the observed values; a real regression (index
//   drift, tokenizer or ranking change) trips them, normal noise does not.
//   These are the deterministic regression floor, NOT a bar for judging a
//   candidate model (a separate, human-gated question).
const FLOOR_MICRO_RECALL_AT_K: f64 = 0.95;
const FLOOR_RECALL_AT_3: f64 = 0.85;
const FLOOR_ALL_EXPECTED_RECALL: f64 = 0.92;
const FLOOR_MRR: f64 = 0.82;

/// The held-out query rows, or `None` when the (untracked) pack is absent —
/// the caller skips with a message rather than failing.
fn load_queries() -> Option<Vec<Value>> {
    let path = default_output_dir(&repo_root()).join("evals/retrieval.jsonl");
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

#[test]
fn retrieval_eval_meets_deterministic_floor() {
    let pack_dir = default_output_dir(&repo_root());
    let Some(queries) = load_queries() else { return };
    let retriever = Retriever::load(&pack_dir).expect("load retriever from shipped pack");
    assert!(
        queries.len() >= 40,
        "expected a held-out query set of >=40 (got {})",
        queries.len()
    );

    // Aggregates. Recall is measured at several cutoffs; the tight ones (@1,@3)
    // are ranking-sensitive and catch regressions that @10 (already saturated)
    // would hide.
    let cutoffs = [1usize, 3, 5, K];
    let mut found_at: BTreeMap<usize, usize> = cutoffs.iter().map(|&k| (k, 0)).collect();
    let mut total_expected = 0usize;
    let mut sum_rr = 0.0f64;
    let mut all_expected_hits = 0usize; // all expected within top-K
    // Dependency-expansion contribution, measured from the tight K_EXP cutoff
    // (where BM25 alone still misses some answers, so expansion can help).
    let mut base_found_exp = 0usize;
    let mut expanded_found_exp = 0usize;
    let mut fam_found: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    for query_row in &queries {
        let id = str_field(query_row, "id");
        let query = str_field(query_row, "query");
        let expected = strs(query_row, "expected_card_ids");
        let family = str_field(query_row, "family").to_owned();
        assert!(!query.is_empty(), "{id}: empty query");
        assert!(!expected.is_empty(), "{id}: needs >=1 expected_card_ids");
        for card in &expected {
            assert!(
                retriever.contains_card(card),
                "{id}: expected card `{card}` is not in the shipped pack index"
            );
        }

        let ranked = retriever.retrieve(query, K);
        total_expected += expected.len();
        for &k in &cutoffs {
            *found_at.get_mut(&k).unwrap() += evaluate(&ranked, &expected, k).found_at_k;
        }
        let m = evaluate(&ranked, &expected, K);
        sum_rr += m.reciprocal_rank();
        if m.all_found {
            all_expected_hits += 1;
        }
        let fam = fam_found.entry(family).or_insert((0, 0));
        fam.0 += m.found_at_k;
        fam.1 += m.expected;

        // Dependency-expansion contribution at the tight cutoff: recall over the
        // base top-K_EXP vs. that set plus its one-hop neighbourhood.
        let base = retriever.retrieve(query, K_EXP);
        let (base_found, _) = set_recall(&base, &expected);
        let expanded = retriever.retrieve_expanded(query, K_EXP);
        let (expanded_found, _) = set_recall(&expanded, &expected);
        base_found_exp += base_found;
        expanded_found_exp += expanded_found;

        if !m.all_found {
            println!(
                "  MISS {id}: recall@{K} {:.2} first_rank {:?} top3={:?}",
                m.recall_at_k,
                m.first_rank,
                ranked.iter().take(3).collect::<Vec<_>>()
            );
        }
    }

    let n = queries.len() as f64;
    let recall = |k: usize| found_at[&k] as f64 / total_expected as f64;
    let micro_recall = recall(K);
    let recall_at_3 = recall(3);
    let all_expected_recall = all_expected_hits as f64 / n;
    let mrr = sum_rr / n;
    let expansion_contribution = expanded_found_exp.saturating_sub(base_found_exp);

    println!("=== retrieval eval (n={}, {} expected cards) ===", queries.len(), total_expected);
    for &k in &cutoffs {
        println!("micro recall@{k:<2}       = {:.3} ({}/{total_expected})", recall(k), found_at[&k]);
    }
    println!("all-expected recall@{K} = {all_expected_recall:.3} ({all_expected_hits}/{})", queries.len());
    println!("MRR                   = {mrr:.3}");
    println!(
        "expansion contrib@{K_EXP}   = +{expansion_contribution} cards (base recall@{K_EXP} {base_found_exp}/{total_expected} -> expanded {expanded_found_exp}/{total_expected})"
    );
    println!("per-family recall@{K}:");
    for (fam, (found, exp)) in &fam_found {
        println!("  {fam:20} {:.3} ({found}/{exp})", *found as f64 / *exp as f64);
    }

    // Expansion must never REDUCE recall (it only adds candidates).
    assert!(
        expanded_found_exp >= base_found_exp,
        "one-hop expansion reduced recall ({base_found_exp} -> {expanded_found_exp}); the expansion map is malformed"
    );

    // Pinned deterministic floors.
    assert!(
        micro_recall >= FLOOR_MICRO_RECALL_AT_K,
        "micro recall@{K} {micro_recall:.3} fell below the pinned floor {FLOOR_MICRO_RECALL_AT_K} — retrieval regressed"
    );
    assert!(
        recall_at_3 >= FLOOR_RECALL_AT_3,
        "micro recall@3 {recall_at_3:.3} fell below the pinned floor {FLOOR_RECALL_AT_3} — ranking regressed"
    );
    assert!(
        all_expected_recall >= FLOOR_ALL_EXPECTED_RECALL,
        "all-expected recall {all_expected_recall:.3} fell below the pinned floor {FLOOR_ALL_EXPECTED_RECALL} — retrieval regressed"
    );
    assert!(
        mrr >= FLOOR_MRR,
        "MRR {mrr:.3} fell below the pinned floor {FLOOR_MRR} — retrieval regressed"
    );
}
