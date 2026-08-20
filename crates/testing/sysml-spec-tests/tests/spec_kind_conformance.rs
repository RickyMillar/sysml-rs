//! Layer 4 v0 — spec-direct element-kind conformance gate.
//!
//! For every fixture in our conformance corpus (Pilot examples +
//! KerML examples + local examples) the TS tree-sitter parser must
//! emit a `ModelGraph` whose `ElementKind` values all appear in the
//! OMG SysML v2 spec TTL vocabularies (Kerml-Vocab.ttl ∪
//! SysML-vocab.ttl).
//!
//! This is the most foundational conformance check: TS must never
//! emit a kind that isn't in the spec. Unlike
//! `pilot_impl_conformance`, this gate uses the OMG TTL files
//! **directly** as the oracle — no intermediary reference impl.
//!
//! ## Why it can still catch regressions
//!
//! `ElementKind` is codegen'd from the same TTL files at build time
//! (`target/.../out/element_kind.generated.rs`), so by construction
//! every variant should be in the spec. This test catches:
//!
//! 1. Hand-rolled additions to `ElementKind` that bypass codegen.
//! 2. Codegen bugs (e.g. PascalCase mismatch, name normalisation drift).
//! 3. Drift when the TTL is updated and codegen is not re-run.
//! 4. Future runtime paths that synthesise kinds via string (`kind.as_str()`-
//!    bypassing constructors).
//!
//! ## Coverage as a side product
//!
//! The test also reports which spec kinds ARE exercised by the
//! corpus, and which are NOT. The latter is informational — corpus
//! expansion can target uncovered kinds. Coverage is logged to
//! stderr; only the kind-conformance assertion is load-bearing.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use sysml_codegen::parse_ttl_vocab;
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn references_root() -> PathBuf {
    workspace_root().join("references").join("sysmlv2")
}

fn pilot_examples_root() -> PathBuf {
    references_root()
        .join("SysML-v2-Pilot-Implementation")
        .join("sysml")
        .join("src")
        .join("examples")
}

fn kerml_examples_root() -> PathBuf {
    references_root()
        .join("SysML-v2-Pilot-Implementation")
        .join("kerml")
        .join("src")
        .join("examples")
}

fn local_examples_root() -> PathBuf {
    workspace_root().join("examples")
}

// ---------------------------------------------------------------------------
// Spec kinds (TTL oracle)
// ---------------------------------------------------------------------------

fn load_spec_kinds() -> HashSet<String> {
    let refs = references_root();
    let kerml_path = refs.join("Kerml-Vocab.ttl");
    let sysml_path = refs.join("SysML-vocab.ttl");
    let kerml_content = std::fs::read_to_string(&kerml_path)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", kerml_path.display()));
    let sysml_content = std::fs::read_to_string(&sysml_path)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", sysml_path.display()));
    let kerml_types = parse_ttl_vocab(&kerml_content)
        .unwrap_or_else(|e| panic!("parse Kerml-Vocab.ttl failed: {e}"));
    let sysml_types = parse_ttl_vocab(&sysml_content)
        .unwrap_or_else(|e| panic!("parse SysML-vocab.ttl failed: {e}"));
    kerml_types
        .iter()
        .chain(sysml_types.iter())
        .map(|t| t.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Corpus discovery
// ---------------------------------------------------------------------------

/// Recursively collect .sysml + .kerml files under `root`. Returns the paths
/// sorted lexicographically so coverage stats are stable across runs.
fn collect_model_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_model_files_into(root, &mut out);
    out.sort();
    out
}

fn collect_model_files_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_model_files_into(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "sysml" || e == "kerml")
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// TS parsing
// ---------------------------------------------------------------------------

fn parse_ts_or_skip(path: &Path) -> Option<sysml_core::ModelGraph> {
    let text = std::fs::read_to_string(path).ok()?;
    let parser = TreeSitterParser::new();
    let tree = parser.parse_tree(&text)?;
    Some(build_model_graph(&tree, &text, &path.to_string_lossy()).graph)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn ts_kinds_are_spec_conformant_across_pilot_examples() {
    let spec_kinds = load_spec_kinds();
    let corpus = collect_model_files(&pilot_examples_root());
    assert!(
        !corpus.is_empty(),
        "no Pilot example files found at {}",
        pilot_examples_root().display()
    );
    run_kind_conformance("Pilot examples", &spec_kinds, &corpus);
}

#[test]
fn ts_kinds_are_spec_conformant_across_kerml_examples() {
    let spec_kinds = load_spec_kinds();
    let corpus = collect_model_files(&kerml_examples_root());
    assert!(
        !corpus.is_empty(),
        "no KerML example files found at {}",
        kerml_examples_root().display()
    );
    run_kind_conformance("KerML examples", &spec_kinds, &corpus);
}

#[test]
fn ts_kinds_are_spec_conformant_across_local_examples() {
    let spec_kinds = load_spec_kinds();
    let corpus = collect_model_files(&local_examples_root());
    assert!(
        !corpus.is_empty(),
        "no local example files found at {}",
        local_examples_root().display()
    );
    run_kind_conformance("local examples", &spec_kinds, &corpus);
}

fn run_kind_conformance(label: &str, spec_kinds: &HashSet<String>, corpus: &[PathBuf]) {
    // (kind, sample fixture path) for every TS kind that is NOT in spec_kinds.
    let mut violations: Vec<(String, PathBuf)> = Vec::new();
    // Count files where each kind appears.
    let mut seen_kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_elements = 0usize;
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for path in corpus {
        let Some(graph) = parse_ts_or_skip(path) else {
            skipped_files += 1;
            continue;
        };
        parsed_files += 1;
        let mut file_kinds: HashSet<String> = HashSet::new();
        for element in graph.elements.values() {
            let kind = element.kind.as_str().to_string();
            total_elements += 1;
            file_kinds.insert(kind.clone());
            if !spec_kinds.contains(&kind) {
                violations.push((kind.clone(), path.clone()));
            }
        }
        for k in file_kinds {
            *seen_kinds.entry(k).or_insert(0) += 1;
        }
    }

    let uncovered_count = spec_kinds
        .iter()
        .filter(|k| !seen_kinds.contains_key(*k))
        .count();
    eprintln!(
        "[{label}] parsed={parsed_files} skipped={skipped_files} elements={total_elements} \
         spec_kinds={} covered={} uncovered={}",
        spec_kinds.len(),
        seen_kinds.len(),
        uncovered_count
    );
    // Print a brief uncovered sample to guide future corpus targeting.
    if uncovered_count > 0 {
        let sample: Vec<&String> = spec_kinds
            .iter()
            .filter(|k| !seen_kinds.contains_key(*k))
            .take(15)
            .collect();
        eprintln!("[{label}] uncovered (first 15): {:?}", sample);
    }

    if !violations.is_empty() {
        // Group by kind for a tighter message.
        let mut by_kind: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for (k, p) in &violations {
            by_kind.entry(k.clone()).or_default().push(p.clone());
        }
        let mut msg = format!(
            "[{label}] TS emitted {} elements whose kind is NOT in spec TTL ({} distinct non-spec kinds):\n",
            violations.len(),
            by_kind.len()
        );
        for (k, paths) in by_kind.iter().take(10) {
            let sample = paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            msg.push_str(&format!(
                "  - {} ({} occurrences; sample fixture: {})\n",
                k,
                paths.len(),
                sample
            ));
        }
        panic!("{msg}");
    }
}
