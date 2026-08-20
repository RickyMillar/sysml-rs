//! L2 meta-gate: the spec-obligation matrix and the `// OBL:` markers in the
//! gating tests can never drift apart (testing-architecture-redesign §3C).
//!
//! Two directions:
//!
//! 1. **Marker → row**: every `// OBL: <id>` line in `tests/*.rs` must
//!    resolve to an obligation ID defined in some
//!    `spec-obligations/*.md` table row. A typo'd or orphaned marker is a
//!    hard failure.
//! 2. **Gated row → marker**: every obligation row whose Gate column names
//!    a test (rather than `—` / prose deferral) should have a matching
//!    `// OBL: <id>` marker in `tests/`. Historical rows predating the
//!    marker convention are pinned in `LEGACY_UNMARKED` — a shrink-only
//!    ratchet; new gated rows MUST carry markers.
//!
//! Obligation IDs are the backticked kebab-case first cells of the
//! `## Obligation table` rows (see `spec-obligations/README.md` "How an
//! obligation is recorded").

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parse `| `id` | ... |` obligation rows from one area file. Only
/// kebab-case ids count (path-like backticked cells in prose tables are
/// not obligation rows).
fn obligation_ids(markdown: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `") else {
            continue;
        };
        let Some(end) = rest.find('`') else { continue };
        let id = &rest[..end];
        let is_kebab = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if is_kebab {
            out.push((id.to_owned(), line.trim().to_owned()));
        }
    }
    out
}

fn area_files() -> Vec<PathBuf> {
    let dir = crate_dir().join("spec-obligations");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "md")
                && p.file_name().is_some_and(|n| n != "README.md" && n != "TEMPLATE.md")
        })
        .collect();
    files.sort();
    // 9 area files today (structural well-formedness is cross-area and has
    // no file of its own). This bound only guards against the directory
    // moving/being emptied; new families push it up naturally.
    assert!(
        files.len() >= 9,
        "expected at least the 9 obligation area files, found {}",
        files.len()
    );
    files
}

/// All `// OBL: <id>` markers in tests/*.rs → the files carrying them.
fn obl_markers() -> BTreeMap<String, BTreeSet<String>> {
    let tests_dir = crate_dir().join("tests");
    let mut markers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in std::fs::read_dir(&tests_dir).expect("read tests dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let file = path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("// OBL: ") else {
                continue;
            };
            // The id is the first token; some markers carry a trailing
            // annotation, e.g. `// OBL: at-most-one-objective (file / S064)`.
            let Some(id) = rest.split_whitespace().next() else {
                continue;
            };
            markers
                .entry(id.to_owned())
                .or_default()
                .insert(file.clone());
        }
    }
    markers
}

fn all_obligation_ids() -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for file in area_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        let fname = file
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        for (id, _row) in obligation_ids(&text) {
            ids.insert(id, fname.clone());
        }
    }
    ids
}

/// Direction 1 — every `// OBL:` marker resolves to a matrix row.
#[test]
fn every_obl_marker_resolves_to_a_matrix_row() {
    let ids = all_obligation_ids();
    assert!(
        ids.len() > 100,
        "obligation-id extraction looks broken: only {} ids parsed",
        ids.len()
    );

    let mut orphans = Vec::new();
    for (marker, files) in obl_markers() {
        if !ids.contains_key(&marker) {
            orphans.push(format!("  {marker}  (in {})", files.iter().cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    assert!(
        orphans.is_empty(),
        "// OBL: markers that resolve to NO row in spec-obligations/*.md \
         (typo, renamed row, or missing matrix entry):\n{}",
        orphans.join("\n")
    );
}

/// Direction 2 — gated GOSPEL/LIBRARY rows carry `// OBL:` markers.
///
/// A row counts as "gated in this crate" when its Gate cell names a
/// `*_spec_conformance` test. Rows created before the marker convention
/// are pinned below (shrink-only); anything NEW showing up here fails.
#[test]
fn every_gated_row_has_an_obl_marker() {
    // Shrink-only ratchet: obligation ids whose gates predate the
    // `// OBL:` convention. Remove entries as markers are backfilled;
    // NEVER add to this list — new gates must carry markers from birth.
    const LEGACY_UNMARKED: &[&str] = &[];

    let markers = obl_markers();
    let mut missing = Vec::new();

    for file in area_files() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        let fname = file
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        for (id, row) in obligation_ids(&text) {
            // Gate cell = 4th column of the canonical 5-column row.
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            let Some(gate_cell) = cells.get(4) else {
                continue;
            };
            let gated_here = gate_cell.contains("_spec_conformance")
                || (gate_cell.starts_with('`') && gate_cell.contains("::"));
            if gated_here
                && !markers.contains_key(&id)
                && !LEGACY_UNMARKED.contains(&id.as_str())
            {
                missing.push(format!("  {fname}: {id}  (gate cell: {gate_cell})"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "gated obligation rows with NO `// OBL:` marker in tests/ — add the \
         marker to the gating test (or, for a row being retired, fix the \
         matrix):\n{}",
        missing.join("\n")
    );
}
