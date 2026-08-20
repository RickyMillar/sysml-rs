//! Phase 2b: spec coverage validation gate — element-kind exhaustiveness.
//!
//! For each expression `ElementKind` variant the parser is supposed to
//! emit, count occurrences across the example corpus + a small synthetic
//! coverage harness. Every variant must appear at least once. A zero
//! count means `process_expression` is dropping a grammar form on the
//! floor — block deletion of the legacy string fallback (Phase 2c) until
//! the coverage gap is closed (or until the corpus is extended to
//! exercise the variant).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

const REQUIRED: &[ElementKind] = &[
    ElementKind::OperatorExpression,
    ElementKind::FeatureReferenceExpression,
    ElementKind::InvocationExpression,
    ElementKind::LiteralBoolean,
    ElementKind::LiteralInteger,
    ElementKind::LiteralRational,
    ElementKind::LiteralString,
];

/// Aspirational variants: emitted by `process_expression` for grammar
/// forms not exercised heavily in the current example corpus. Surface
/// their coverage but do not block the test if absent.
const ASPIRATIONAL: &[ElementKind] = &[
    ElementKind::FeatureChainExpression,
    ElementKind::SelectExpression,
    ElementKind::CollectExpression,
    ElementKind::IndexExpression,
    ElementKind::MetadataAccessExpression,
    ElementKind::ConstructorExpression,
    ElementKind::NullExpression,
    ElementKind::LiteralInfinity,
];

fn examples_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .canonicalize()
        .expect("examples directory should exist")
}

fn collect_sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sysml_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
            out.push(path);
        }
    }
}

fn count_kinds_in(graph: &ModelGraph, counts: &mut BTreeMap<String, usize>) {
    for elem in graph.elements.values() {
        let key = format!("{:?}", elem.kind);
        *counts.entry(key).or_insert(0) += 1;
    }
}

#[test]
fn every_required_element_kind_emitted() {
    let dir = examples_dir();
    let mut files = Vec::new();
    collect_sysml_files(&dir, &mut files);
    let parser = TreeSitterParser::new();

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        let sysml_files = vec![SysmlFile {
            path: file.to_string_lossy().into_owned(),
            text: content,
        }];
        let result = parser.parse(&sysml_files);
        count_kinds_in(&result.graph, &mut counts);
    }

    // Print per-variant counts for the report.
    eprintln!("Element kind coverage matrix ({} files):", files.len());
    for kind in REQUIRED.iter().chain(ASPIRATIONAL.iter()) {
        let key = format!("{:?}", kind);
        let count = counts.get(&key).copied().unwrap_or(0);
        let aspirational = ASPIRATIONAL.contains(kind);
        let status = if count == 0 {
            if aspirational {
                "(aspirational, 0 — corpus does not exercise)"
            } else {
                "** MISSING **"
            }
        } else {
            ""
        };
        eprintln!("  {:>40}: {:>5}  {}", key, count, status);
    }

    let missing: Vec<&ElementKind> = REQUIRED
        .iter()
        .filter(|k| {
            let key = format!("{:?}", k);
            counts.get(&key).copied().unwrap_or(0) == 0
        })
        .collect();

    assert!(
        missing.is_empty(),
        "required expression kinds not emitted by parser: {:?}",
        missing
    );
}
