//! Obligation cards: one validation-facet card per *reviewed-valid*
//! semantic-conformance obligation drawn from `spec-obligations/*.md`.
//!
//! Re-validation discipline (never copy a stale tracker verdict):
//! an obligation is carded ONLY when it carries a `// OBL:` marker on a
//! currently-green (non-`#[ignore]`d) gate test in `sysml-spec-tests`. That
//! marker set is kept in lockstep with the tracker rows by the
//! `obligation_matrix_consistency` meta-gate, and its tests are proven green by
//! a live run — so a carded obligation is one the engine demonstrably conforms
//! to at this commit. Obligations that are deferred, ungated, or gated only by
//! an `#[ignore]`d (open-gap) test stay in the denominator as `excluded`, with
//! the tracker's own status cell as rationale — never carded, never faked.

use std::collections::BTreeSet;
use std::path::Path;

use super::LpError;

/// Where the obligation gate tests (and their `// OBL:` markers) live.
const TESTS_DIR: &str = "crates/testing/sysml-spec-tests/tests";

/// One parsed obligation-matrix row (the first three columns are uniform across
/// every area file; the last cell — Gate/Coverage/Verdict/Build-status — is
/// kept verbatim as the exclusion rationale for un-carded rows).
#[derive(Debug, Clone)]
pub struct Obligation {
    pub id: String,
    /// Area file stem, e.g. `requirements`.
    pub area: String,
    /// The obligation sentence (already an original paraphrase in the tracker,
    /// safe to reuse as a card summary — never reproduced normative prose).
    pub text: String,
    /// The raw citation cell (clause + tier).
    pub citation: String,
    /// `SysML` or `KerML`, inferred from the citation.
    pub document: String,
    /// `sysml` or `kerml`, inferred from the citation.
    pub authority: String,
    /// The first spec clause number in the citation (e.g. `7.21.1`), if any.
    pub clause: Option<String>,
    /// The last table cell, verbatim (Gate/Coverage/Verdict/Build-status).
    pub status: String,
}

/// Allowlisted obligation md file for an area stem.
pub fn area_file(area: &str) -> &'static str {
    use super::manifest::{
        OBLIGATION_ACTIONS, OBLIGATION_CALCULATIONS, OBLIGATION_CONSTRAINTS,
        OBLIGATION_FLOWS_PORTS, OBLIGATION_OCCURRENCES, OBLIGATION_ODE_PHYSICS,
        OBLIGATION_REQUIREMENTS, OBLIGATION_STATE_MACHINES, OBLIGATION_STRUCTURAL,
        OBLIGATION_VERIFICATION,
    };
    match area {
        "actions" => OBLIGATION_ACTIONS,
        "calculations" => OBLIGATION_CALCULATIONS,
        "constraints-expressions" => OBLIGATION_CONSTRAINTS,
        "flows-ports" => OBLIGATION_FLOWS_PORTS,
        "occurrences-clocks" => OBLIGATION_OCCURRENCES,
        "ode-physics" => OBLIGATION_ODE_PHYSICS,
        "requirements" => OBLIGATION_REQUIREMENTS,
        "state-machines" => OBLIGATION_STATE_MACHINES,
        "structural" => OBLIGATION_STRUCTURAL,
        "verification-analysis-cases" => OBLIGATION_VERIFICATION,
        _ => OBLIGATION_ACTIONS,
    }
}

/// Extract the first `§<clause>` (a run of digits and dots) after `marker`, or
/// the first bare `§<clause>` when `marker` is empty. Trailing `.` trimmed.
fn extract_clause(citation: &str, kerml: bool) -> Option<String> {
    let hay = citation;
    // For a KerML citation, take the clause that follows "KerML §"; otherwise
    // the first bare "§" clause.
    let start = if kerml {
        let idx = hay.find("KerML")?;
        hay[idx..].find('§').map(|o| idx + o + '§'.len_utf8())?
    } else {
        hay.find('§').map(|o| o + '§'.len_utf8())?
    };
    let rest = &hay[start..];
    let clause: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let clause = clause.trim_end_matches('.').to_owned();
    if clause.is_empty() {
        None
    } else {
        Some(clause)
    }
}

/// Is `id` a clean card-id slug (`[a-z0-9]+(-[a-z0-9]+)*`)?
fn is_clean_slug(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// True if a citation cell looks like a real obligation citation (a `§` clause
/// or a tier keyword) rather than a reproduction-table grep command.
fn is_citation_cell(cell: &str) -> bool {
    cell.contains('§')
        || ["GOSPEL", "LIBRARY", "STRUCTURAL", "SPEC-SILENT"]
            .iter()
            .any(|k| cell.contains(k))
}

/// Parse an area file's obligation rows. The matrices are not uniformly
/// sectioned (some have `## Obligation table`, ode-physics splits them across
/// `## What the spec normatively defines` / `## ... leaves to the tool`, and
/// actions records four in a 3-column "Missed" sub-table), so rows are
/// identified *structurally*: a backticked clean-kebab id in the first cell and
/// a `§`/tier citation in the third cell. That predicate captures every
/// obligation-bearing row in all three layouts while excluding the reproduction
/// tables (whose third cell is a grep command) and completeness-audit tables
/// (whose first cell is prose, not a backticked id) — mirroring the id set the
/// `obligation_matrix_consistency` meta-gate resolves markers against.
fn parse_area(area: &str, md: &str) -> Vec<Obligation> {
    let mut out = Vec::new();
    for line in md.lines() {
        let t = line.trim_start();
        if !t.starts_with("| `") {
            continue;
        }
        // Cells: split on '|', drop the leading/trailing empties.
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // cells[0] is "" (before first pipe); id in cells[1], text cells[2],
        // citation cells[3]; the status is the last non-empty cell.
        let (Some(id_cell), Some(text_cell), Some(cite_cell)) =
            (cells.get(1), cells.get(2), cells.get(3))
        else {
            continue;
        };
        let id = id_cell.trim_matches('`').trim();
        if !is_clean_slug(id) {
            continue;
        }
        if !is_citation_cell(cite_cell) {
            continue; // reproduction row (grep command in the citation slot)
        }
        let text = (*text_cell).to_owned();
        let citation = (*cite_cell).to_owned();
        let status = cells
            .iter()
            .rev()
            .map(|c| c.trim())
            .find(|c| !c.is_empty())
            .unwrap_or("")
            .to_owned();
        // KerML when the clause reference is a KerML one — including cells that
        // lead with a library-file ref before `KerML §…` (e.g. clock-timeflow),
        // not only cells that start with the word "KerML".
        let kerml =
            citation.contains("KerML §") || citation.trim_start().starts_with("KerML");
        let (document, authority) = if kerml {
            ("KerML".to_owned(), "kerml".to_owned())
        } else {
            ("SysML".to_owned(), "sysml".to_owned())
        };
        let clause = extract_clause(&citation, kerml);
        out.push(Obligation {
            id: id.to_owned(),
            area: area.to_owned(),
            text,
            citation,
            document,
            authority,
            clause,
            status,
        });
    }
    out
}

/// Parse every allowlisted obligation area file (deterministic order).
pub fn parse_obligations(read_allowlisted: impl Fn(&str) -> Result<String, LpError>) -> Result<Vec<Obligation>, LpError> {
    let mut out = Vec::new();
    let areas = [
        "actions",
        "calculations",
        "constraints-expressions",
        "flows-ports",
        "occurrences-clocks",
        "ode-physics",
        "requirements",
        "state-machines",
        "structural",
        "verification-analysis-cases",
    ];
    for area in areas {
        let md = read_allowlisted(area_file(area))?;
        out.extend(parse_area(area, &md));
    }
    // Deterministic: sort by id, drop any duplicate ids (keep first).
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// Obligation ids carrying a `// OBL:` marker on a **non-ignored** test in
/// `tests/*.rs` — the executable re-validation signal (a currently-green gate).
/// A marker whose only home is an `#[ignore]`d test (an open gap) is excluded.
pub fn gated_green_ids(repo_root: &Path) -> Result<BTreeSet<String>, LpError> {
    let dir = repo_root.join(TESTS_DIR);
    let mut green: BTreeSet<String> = BTreeSet::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| LpError::Io(format!("readdir {}: {e}", dir.display())))?;
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| LpError::Io(format!("read {}: {e}", path.display())))?;
        // Track whether the current enclosing fn is `#[ignore]`d. Attributes
        // precede `fn`; a pending `#[ignore]` applies to the next fn.
        let mut pending_ignore = false;
        let mut current_ignored = false;
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with("#[ignore") {
                pending_ignore = true;
                continue;
            }
            if t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("async fn ") {
                current_ignored = pending_ignore;
                pending_ignore = false;
                continue;
            }
            if let Some(rest) = t.strip_prefix("// OBL: ") {
                if let Some(id) = rest.split_whitespace().next() {
                    if !current_ignored {
                        green.insert(id.to_owned());
                    }
                }
            }
        }
    }
    Ok(green)
}

/// A concise exclusion rationale from an un-carded obligation's status cell:
/// markdown emphasis/backticks stripped, collapsed whitespace, length-capped.
pub fn exclusion_rationale(status: &str) -> String {
    let cleaned: String = status
        .replace("**", "")
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let capped: String = cleaned.chars().take(240).collect();
    if capped.is_empty() {
        "not carded: no currently-green gate marker (deferred/ungated)".to_owned()
    } else {
        format!("not carded (no currently-green gate marker): {capped}")
    }
}

/// A short area-derived retrieval category tag (in addition to `validation`).
pub fn area_category(area: &str) -> Option<&'static str> {
    match area {
        "requirements" => Some("requirements"),
        "state-machines" => Some("state-machine"),
        "constraints-expressions" => Some("expression"),
        "verification-analysis-cases" => Some("cases"),
        "flows-ports" => Some("connection"),
        "actions" | "calculations" | "occurrences-clocks" | "ode-physics" => Some("behavior"),
        _ => None,
    }
}
