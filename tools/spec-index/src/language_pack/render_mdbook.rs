//! Render a generated language pack as mdBook-ready markdown.
//!
//! Reads a pack from disk (the exported JSON is the interface — rendering must
//! work against any pack a consumer has, not only one just generated in
//! memory) and writes one page per primary card category plus an index page.
//! Output is deterministic: cards sorted by id, categories sorted by slug, no
//! wall-clock — the same pack renders to a byte-identical tree.
//!
//! The pages inherit the pack's licensing position (citation-only; grammar IR
//! derived from the LGPL-3.0-or-later pilot Xtext grammars) and say so on the
//! index page: a book that embeds them carries that notice alongside its own
//! prose license.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use super::LpError;

/// Example ids rendered as untagged code blocks instead of ```sysml.
///
/// The book's checker (`tools/check-code-blocks.py` in the book repo)
/// parse-gates every ```sysml fence with `sysml inspect --no-stdlib` and
/// flags `<angle>` text as a mis-fenced template. These examples are valid
/// per the pack's own `language_card_examples` gate but do not stand alone
/// under that checker's rules, so they ship untagged (rendered, not gated):
/// - `decide`: the lone decision node draws "no outgoing branches";
/// - `perform`: the minted `pa` target draws "references unknown action";
/// - `named-rep`: the `<b>hi</b>` comment body matches the checker's
///   angle-bracket template heuristic.
const UNTAGGED_EXAMPLES: &[&str] = &[
    "kerml.metadata.textual-representation.positive.named-rep",
    "sysml.behavior.decision-node.positive.decide",
    "sysml.behavior.perform-action.positive.perform",
];

/// Display names for the primary category slugs. A slug outside this table
/// falls back to title-cased words, so a future category renders (with a
/// generic name) rather than failing.
fn category_display(slug: &str) -> String {
    match slug {
        "behavior" => "Behavior".to_owned(),
        "cases" => "Cases".to_owned(),
        "connection" => "Connections".to_owned(),
        "expression" => "Expressions".to_owned(),
        "implementation" => "Implementation Notes".to_owned(),
        "library" => "Standard Library".to_owned(),
        "metadata" => "Metadata".to_owned(),
        "requirements" => "Requirements".to_owned(),
        "state-machine" => "State Machines".to_owned(),
        "structure" => "Structure".to_owned(),
        "validation" => "Validation Rules".to_owned(),
        "views" => "Views".to_owned(),
        other => other
            .split('-')
            .map(|w| {
                let mut c = w.chars();
                c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn read_json(path: &Path) -> Result<Value, LpError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| LpError::Io(format!("read {}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| LpError::Other(format!("parse {}: {e}", path.display())))
}

fn str_field<'a>(card: &'a Value, key: &str) -> &'a str {
    card.get(key).and_then(Value::as_str).unwrap_or("")
}

fn str_list(card: &Value, key: &str) -> Vec<String> {
    card.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Anchor for a card id: dots to hyphens (`kerml.behavior.function` ->
/// `kerml-behavior-function`), stable across renders.
fn anchor(id: &str) -> String {
    id.replace('.', "-")
}

fn support_mark(value: &str) -> &'static str {
    match value {
        "validated" => "\u{2713}", // ✓
        "unsupported" => "\u{2717}", // ✗
        "partial" => "partial",
        _ => "unknown",
    }
}

/// Render one card as a markdown section.
fn render_card(
    card: &Value,
    page_of: &BTreeMap<String, String>,
    titles: &BTreeMap<String, String>,
    examples_dir: &Path,
    out: &mut String,
) -> Result<(), LpError> {
    let id = str_field(card, "id");
    let title = str_field(card, "title");
    let language = str_field(card, "language");
    let summary = str_field(card, "summary");

    let _ = writeln!(out, "## {title} {{#{}}}\n", anchor(id));
    let _ = writeln!(out, "<span class=\"lp-lang lp-lang-{}\">{language}</span> — `{id}`\n",
        language.to_lowercase());
    let _ = writeln!(out, "{summary}\n");

    // Positive examples (negative/composed records are gate fixtures, not
    // reader material).
    let positive: Vec<String> = card
        .get("examples")
        .and_then(|e| e.get("positive"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    for ex_id in &positive {
        let record = read_json(&examples_dir.join(format!("{ex_id}.json")))?;
        let source = str_field(&record, "source");
        if source.is_empty() {
            continue;
        }
        let fence = if UNTAGGED_EXAMPLES.contains(&ex_id.as_str()) { "```" } else { "```sysml" };
        let _ = writeln!(out, "{fence}\n{source}\n```\n");
    }

    // Normative clause citations: document + clause, plain text. `ancestor`
    // resolution (citation resolved to the nearest existing heading) stays
    // visible — precision loss must not be silently dropped.
    let clauses = card.get("normative_clauses").and_then(Value::as_array);
    if let Some(clauses) = clauses.filter(|c| !c.is_empty()) {
        let rendered: Vec<String> = clauses
            .iter()
            .map(|c| {
                let doc = str_field(c, "document");
                let clause = str_field(c, "clause");
                let suffix = if str_field(c, "resolution") == "ancestor" {
                    " (nearest heading)"
                } else {
                    ""
                };
                format!("{doc} \u{a7}{clause}{suffix}")
            })
            .collect();
        let _ = writeln!(out, "**Normative clauses:** {}\n", rendered.join(", "));
    }

    // Support axes, machine-derived from gate evidence; `unknown` means "no
    // evidence", never "no".
    if let Some(support) = card.get("support") {
        let line: Vec<String> = ["parse", "resolve", "elaborate", "execute"]
            .iter()
            .map(|axis| format!("{axis} {}", support_mark(str_field(support, axis))))
            .collect();
        let _ = writeln!(out, "**Support (sysml-rs):** {}\n", line.join(" \u{b7} "));
    }

    let related = str_list(card, "related_cards");
    if !related.is_empty() {
        let links: Vec<String> = related
            .iter()
            .map(|rid| {
                match (page_of.get(rid), titles.get(rid)) {
                    (Some(page), Some(rtitle)) => {
                        format!("[{rtitle}]({page}.md#{})", anchor(rid))
                    }
                    // A dangling related id would have failed the pack's own
                    // gates; render it inert rather than as a broken link.
                    _ => format!("`{rid}`"),
                }
            })
            .collect();
        let _ = writeln!(out, "**Related:** {}\n", links.join(", "));
    }
    Ok(())
}

fn index_page(manifest: &Value, report: &Value, categories: &[(String, usize)]) -> String {
    let spec_drop = str_field(manifest, "spec_drop");
    let tree_hash = str_field(report, "tree_hash");
    let card_count = report.get("card_count").and_then(Value::as_u64).unwrap_or(0);
    let mut out = String::new();
    let _ = writeln!(out, "# Language Reference\n");
    let _ = writeln!(
        out,
        "This section is generated from the sysml-rs **language pack**: a \
machine-readable index of every SysML v2 / KerML language concept, one \"card\" \
per concept. The pack is an *index over the normative sources, not an \
authority*: every card points at the governing specification clause and \
paraphrases it — where a card and the specification disagree, the \
specification wins. Use these pages to find which clause governs a question, \
then read and cite that clause.\n"
    );
    let _ = writeln!(
        out,
        "Implementation-support marks (parse / resolve / elaborate / execute) are \
machine-derived from test evidence in the sysml-rs repository, never \
hand-written: \u{2713} means a gate test passed for that axis at the current \
spec drop, \u{2717} means a reviewed known limitation, and *unknown* means no \
evidence either way — it never means \"no\".\n"
    );
    let _ = writeln!(out, "Generated from spec drop **{spec_drop}** ({card_count} cards, pack tree `{tree_hash}`).\n");

    let _ = writeln!(out, "## Categories\n");
    for (slug, count) in categories {
        let _ = writeln!(out, "- [{}]({slug}.md) ({count} cards)", category_display(slug));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Raw JSON for tools and agents\n");
    let _ = writeln!(
        out,
        "The pack itself ships with this book as static JSON under \
[`language-pack/`](../language-pack/manifest.json): `manifest.json` (spec-drop \
identity and pinned source hashes), `cards/<id>.json` (one card per concept), \
and `indexes/` (`keywords.json` term index, `aliases.json` alias \u{2192} card \
id, `dependencies.json` one-hop expansion map, `cards.jsonl` the whole corpus \
as JSONL). On the published site these resolve as \
`<book-url>/language-pack/manifest.json`, \
`<book-url>/language-pack/cards/<id>.json`, and so on. The intended lookup \
pattern: find candidate cards via `indexes/keywords.json` or \
`indexes/aliases.json`, read `cards/<id>.json`, expand one hop via \
`indexes/dependencies.json`, then cite the card's `normative_clauses`.\n"
    );

    let _ = writeln!(out, "## Regenerating\n");
    let _ = writeln!(
        out,
        "These pages and the raw JSON are generated artifacts — edit the \
generator, not the pages. From a sibling `sysml-rs` checkout:\n\n\
```text\n\
# regenerate the pack (see tools/spec-index/README.md for source fetching)\n\
cargo run -p spec-index -- language-pack\n\n\
# re-render this section + refresh the shipped JSON (from the book repo)\n\
./tools/render-language-pack.sh\n\
```\n"
    );

    let _ = writeln!(out, "## Licensing and attribution\n");
    let _ = writeln!(
        out,
        "The pack is **citation-only by design**: no OMG specification prose is \
reproduced in any card, example, or page — summaries are original paraphrases, \
and normative content is referenced by document + clause locator. The grammar \
information (rule names, structure, keyword literals) is derived from the \
Xtext grammars of the [SysML-v2 Pilot \
Implementation](https://github.com/Systems-Modeling/SysML-v2-Pilot-Implementation), \
which is licensed **LGPL-3.0-or-later**; this notice covers that derivation. \
Metamodel facets are derived from the OMG-published TTL vocabularies at pinned \
revisions. These generated pages and the shipped JSON carry the terms above, \
distinct from the CC-BY-4.0 license of this book's prose chapters.\n"
    );
    out
}

/// Render the pack at `pack_dir` into `out_dir` (wiped and recreated, so
/// removed categories cannot leave stale pages). One page per primary
/// category, plus `index.md`.
pub fn render(pack_dir: &Path, out_dir: &Path) -> Result<(), LpError> {
    let manifest = read_json(&pack_dir.join("manifest.json"))?;
    let report = read_json(&pack_dir.join("report.json"))?;
    let cards_dir = pack_dir.join("cards");
    let examples_dir = pack_dir.join("examples");

    // Load every card, sorted by id (BTreeMap keys the sort).
    let mut cards: BTreeMap<String, Value> = BTreeMap::new();
    for entry in std::fs::read_dir(&cards_dir)
        .map_err(|e| LpError::Io(format!("readdir {}: {e}", cards_dir.display())))?
    {
        let path = entry.map_err(|e| LpError::Io(format!("entry: {e}")))?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let card = read_json(&path)?;
        let id = str_field(&card, "id").to_owned();
        if id.is_empty() {
            return Err(LpError::Other(format!("card without id: {}", path.display())));
        }
        cards.insert(id, card);
    }
    if cards.is_empty() {
        return Err(LpError::Other(format!("no cards found in {}", cards_dir.display())));
    }

    // Group by primary category; build the id -> page / id -> title maps for
    // cross-links.
    let mut by_category: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut page_of: BTreeMap<String, String> = BTreeMap::new();
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    for (id, card) in &cards {
        let primary = str_list(card, "category")
            .first()
            .cloned()
            .ok_or_else(|| LpError::Other(format!("card {id} has no category")))?;
        page_of.insert(id.clone(), primary.clone());
        titles.insert(id.clone(), str_field(card, "title").to_owned());
        by_category.entry(primary).or_default().push(card);
    }

    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir)
            .map_err(|e| LpError::Io(format!("rm {}: {e}", out_dir.display())))?;
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|e| LpError::Io(format!("mkdir {}: {e}", out_dir.display())))?;

    let mut category_counts: Vec<(String, usize)> = Vec::new();
    for (slug, cards) in &by_category {
        category_counts.push((slug.clone(), cards.len()));
        let mut page = String::new();
        let _ = writeln!(page, "# {}\n", category_display(slug));
        let _ = writeln!(
            page,
            "Generated from the sysml-rs language pack — see the \
[Language Reference index](index.md) for provenance, licensing, and the raw \
JSON. {} cards.\n",
            cards.len()
        );
        for card in cards {
            render_card(card, &page_of, &titles, &examples_dir, &mut page)?;
        }
        let path = out_dir.join(format!("{slug}.md"));
        std::fs::write(&path, page)
            .map_err(|e| LpError::Io(format!("write {}: {e}", path.display())))?;
    }

    let index = index_page(&manifest, &report, &category_counts);
    let path = out_dir.join("index.md");
    std::fs::write(&path, index).map_err(|e| LpError::Io(format!("write {}: {e}", path.display())))?;

    println!(
        "rendered {} cards into {} category pages + index at {}",
        cards.len(),
        category_counts.len(),
        out_dir.display()
    );
    Ok(())
}
