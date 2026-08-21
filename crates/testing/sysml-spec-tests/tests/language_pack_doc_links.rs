//! Focused gate over the documentation files that point at the language pack.
//! Fails when:
//!
//! 1. a card id referenced in a doc file does not resolve to a card;
//! 2. a repo/pack path linked in a doc file does not exist;
//! 3. the documented pack is stale versus its manifest (freshness);
//! 4. a doc file's language-pack guidance cites a fixture path as a source
//!    (the pack indexes normative spec sources, never fixtures).
//!
//! It is anchored to the actual doc files, not a broad repo scan. The pack
//! directory is not tracked in git; both tests skip with a message when it is
//! absent and run fully when present.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use spec_index::language_pack::{default_output_dir, generation_sources_present, info, repo_root};

/// Documentation files whose language-pack references this gate validates.
const DOC_FILES: &[&str] = &["tools/spec-index/README.md"];

const AUTHORITIES: &[&str] = &["kerml", "sysml", "tooling"];
const FACETS: &[&str] = &[
    "structure",
    "behavior",
    "requirements",
    "cases",
    "views",
    "expression",
    "metadata",
    "validation",
    "library",
    "implementation",
];

/// Repo-relative path prefixes (resolve against repo root). `references/` paths
/// are checked only when the referenced sources are actually fetched.
const REPO_ROOTS: &[&str] = &["references/", "docs/", "crates/", "tools/"];
/// Pack-relative path prefixes (resolve against the pack dir).
const PACK_ROOTS: &[&str] = &["cards/", "indexes/", "retrieval/", "evals/"];

fn pack_card_ids(pack_dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(pack_dir.join("cards"))
        .expect("cards dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

fn looks_like_card_id(tok: &str) -> bool {
    let parts: Vec<&str> = tok.split('.').collect();
    parts.len() == 3
        && AUTHORITIES.contains(&parts[0])
        && FACETS.contains(&parts[1])
        && !parts[2].is_empty()
        && parts[2].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Inline-code spans (between backticks) of a markdown file.
fn code_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut in_span = false;
    let mut cur = String::new();
    for ch in text.chars() {
        if ch == '`' {
            if in_span {
                spans.push(std::mem::take(&mut cur));
            }
            in_span = !in_span;
        } else if in_span {
            cur.push(ch);
        }
    }
    spans
}

/// Heading depth (number of leading `#`), or 0 if the line is not a heading.
fn heading_level(line: &str) -> usize {
    let t = line.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    if n > 0 && t.chars().nth(n) == Some(' ') {
        n
    } else {
        0
    }
}

/// The language-pack guidance slice of an instruction file — anchored to the
/// block we added, so unrelated pre-existing links elsewhere in these large
/// files are not scanned.
///
/// If a heading mentions the language pack, the section runs from that heading
/// to the next heading of the same-or-higher level. Otherwise (the pack guidance
/// is a bullet under an existing heading, e.g. the parser crate's "Spec
/// References"), the section runs from the first "language pack" line to the next
/// heading, so it stays local to that bullet block.
fn pack_section(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let is_pack = |l: &str| {
        let low = l.to_ascii_lowercase();
        low.contains("language pack") || low.contains("language-pack")
    };
    // Prefer a heading anchor.
    if let Some(start) = lines.iter().position(|l| heading_level(l) > 0 && is_pack(l)) {
        let level = heading_level(lines[start]);
        let mut end = lines.len();
        for (i, l) in lines.iter().enumerate().skip(start + 1) {
            let lv = heading_level(l);
            if lv > 0 && lv <= level {
                end = i;
                break;
            }
        }
        return lines[start..end].join("\n");
    }
    // Fallback: first pack-mentioning line to the next heading.
    if let Some(start) = lines.iter().position(|l| is_pack(l)) {
        let mut end = lines.len();
        for (i, l) in lines.iter().enumerate().skip(start + 1) {
            if heading_level(l) > 0 {
                end = i;
                break;
            }
        }
        return lines[start..end].join("\n");
    }
    String::new()
}

#[test]
fn doc_links_and_card_ids_resolve() {
    let repo = repo_root();
    let pack_dir = default_output_dir(&repo);
    if !pack_dir.join("cards").is_dir() {
        eprintln!(
            "SKIP: no language pack at {} (run cargo run -p spec-index, then \
             cargo run -p spec-index -- language-pack)",
            pack_dir.display()
        );
        return;
    }
    let card_ids = pack_card_ids(&pack_dir);

    let mut problems: Vec<String> = Vec::new();

    for rel in DOC_FILES {
        let path = repo.join(rel);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        // Anchor every check to the language-pack guidance block; pre-existing
        // links elsewhere in these large files are out of scope.
        let section = pack_section(&text);
        assert!(
            !section.is_empty(),
            "{rel}: no language-pack guidance section found"
        );

        for span in code_spans(&section) {
            let span = span.trim();

            // (1) Card-id references must resolve.
            if looks_like_card_id(span) && !card_ids.contains(span) {
                problems.push(format!("{rel}: card id `{span}` not found in pack"));
            }

            // (2) Path links must exist. Skip templates and commands.
            if span.contains('<') || span.contains('*') || span.contains(' ') || span.contains("::")
            {
                continue;
            }
            // Strip a trailing anchor/punctuation.
            let clean = span.trim_end_matches([')', ',', '.', ';', ':']);
            let clean = clean.split('#').next().unwrap_or(clean);
            // (4) The pack indexes normative spec sources, never fixtures. Any
            // `examples/` path cited as a source in the pack guidance is wrong.
            if clean.starts_with("examples/") {
                problems.push(format!(
                    "{rel}: language-pack guidance cites a fixture path `{clean}` as a source"
                ));
            }
            let checked: Option<PathBuf> = if REPO_ROOTS.iter().any(|r| clean.starts_with(r)) {
                Some(repo.join(clean))
            } else if PACK_ROOTS.iter().any(|r| clean.starts_with(r)) {
                Some(pack_dir.join(clean))
            } else {
                None
            };
            if let Some(p) = checked {
                if !p.exists() {
                    problems.push(format!("{rel}: linked path `{clean}` does not exist"));
                }
            }
        }
    }

    assert!(problems.is_empty(), "doc link/card gate:\n  {}", problems.join("\n  "));
}

#[test]
fn documented_pack_is_present_and_fresh() {
    let repo = repo_root();
    let pack_dir = default_output_dir(&repo);
    if !pack_dir.join("manifest.json").exists() || !generation_sources_present(&repo) {
        eprintln!(
            "SKIP: no language pack at {} or sources absent (fetch references, run \
             cargo run -p spec-index, then cargo run -p spec-index -- language-pack)",
            pack_dir.display()
        );
        return;
    }
    let info = info::pack_info(&repo, &pack_dir).expect("pack info");
    assert!(info.present, "documented language pack is absent at {}", info.pack_path);
    assert_eq!(
        info.freshness, "clean",
        "documented language pack is `{}` (committed tree {} vs regenerated {}) — \
         regenerate with `cargo run -p spec-index -- language-pack`",
        info.freshness, info.committed_tree_hash, info.regenerated_tree_hash
    );
}
