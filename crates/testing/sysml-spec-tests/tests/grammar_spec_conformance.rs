//! Tree-sitter grammar validation against SysML v2 specification.
//!
//! These tests verify that the tree-sitter grammar correctly represents
//! SysML v2 element types and enum values by cross-referencing against
//! the authoritative spec files via `sysml-codegen` parsers.
//!
//! Run with:
//! ```bash
//! SYSML_CORPUS_PATH=references/sysmlv2 \
//!   cargo test -p sysml-spec-tests treesitter -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use sysml_spec_tests::treesitter_validation::{format_report, validate_treesitter};

/// Enum names the tree-sitter grammar is permitted to diverge from the xtext
/// enum keywords on, each with a rationale (SUP-02). An entry allows the enum
/// to be reported `spec-only` (absent from the grammar's enum assets) OR to
/// carry mismatched values, without failing [`treesitter_enum_validation`].
/// Anything NOT listed here is a hard failure. Seeded from current reality —
/// keep it minimal and re-justify every addition against the live grammar.
const ENUM_EXCEPTIONS: &[(&str, &str)] = &[(
    "TransitionFeatureKind",
    "generated/enums.js records the metamodel literal names (trigger/guard/effect) \
     for this enum, whereas the xtext concrete syntax uses the keywords \
     accept/if/do (trigger='accept', guard='if', effect='do'). The concrete \
     keywords are matched directly by rules/states.js (trigger_accept, \
     guard_expression, effect_do), so transition features parse correctly; the \
     enum-asset entry is a naming artifact that only feeds the generic \
     enum_value token, not a coverage gap.",
)];

/// Locate the tree-sitter directory relative to the workspace root.
fn find_treesitter_dir() -> PathBuf {
    let candidates = [
        "sysml-ts/tree-sitter",
        "../sysml-ts/tree-sitter",
        "../../sysml-ts/tree-sitter",
    ];
    for candidate in &candidates {
        let path = PathBuf::from(candidate);
        if path.join("grammar.js").exists() {
            return path;
        }
    }
    // Try from CARGO_MANIFEST_DIR
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let from_manifest = manifest
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("crates/lang/sysml-parser-incremental/tree-sitter");
    if from_manifest.join("grammar.js").exists() {
        return from_manifest;
    }
    panic!(
        "Could not find tree-sitter directory.\n\
         Searched: {:?}\n\
         Ensure sysml-ts/tree-sitter/grammar.js exists.",
        candidates
    );
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn treesitter_spec_validation() {
    let refs_dir = sysml_spec_tests::find_references_dir();
    let ts_dir = find_treesitter_dir();

    // Ensure node-types.json exists (requires `npm run build` in tree-sitter dir)
    let node_types_path = ts_dir.join("src/node-types.json");
    assert!(
        node_types_path.exists(),
        "node-types.json not found at {}.\n\
         Run `npm run build` in {} first.",
        node_types_path.display(),
        ts_dir.display(),
    );

    let report = validate_treesitter(&refs_dir, &ts_dir)
        .unwrap_or_else(|e| panic!("Validation failed: {e}"));

    let output = format_report(&report);
    println!("{output}");

    // --- Assertions ---
    let etc = &report.element_type_coverage;

    // We expect at least some explicit rules
    assert!(
        !etc.explicit_rules.is_empty(),
        "No explicit element type matches found — something is wrong with the mapping"
    );

    // Report coverage as a clear metric
    println!(
        "\nSUMMARY: {}/{} spec types covered ({:.1}%), {} explicit, {} generic, {} missing",
        etc.explicit_rules.len() + etc.generic_coverage.len(),
        etc.total_spec_types,
        etc.coverage_percent,
        etc.explicit_rules.len(),
        etc.generic_coverage.len(),
        etc.missing.len(),
    );

    // Check enum mismatches (these are actionable bugs)
    let xtext_mismatches = report
        .enum_coverage_xtext
        .as_ref()
        .map(|c| c.value_mismatches.len())
        .unwrap_or(0);

    if xtext_mismatches > 0 {
        println!(
            "\nWARNING: {} enum value mismatches detected (vs xtext grammar)",
            xtext_mismatches
        );
        for m in &report
            .enum_coverage_xtext
            .as_ref()
            .unwrap()
            .value_mismatches
        {
            println!(
                "  {}: grammar has {:?}, xtext has {:?}",
                m.name, m.grammar_values, m.spec_values
            );
        }
    }
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn treesitter_element_type_coverage() {
    let refs_dir = sysml_spec_tests::find_references_dir();
    let ts_dir = find_treesitter_dir();

    let node_types_path = ts_dir.join("src/node-types.json");
    if !node_types_path.exists() {
        eprintln!(
            "SKIPPED: node-types.json not found. Run `npm run build` in tree-sitter dir first."
        );
        return;
    }

    let kerml_vocab = std::fs::read_to_string(refs_dir.join("Kerml-Vocab.ttl")).unwrap();
    let sysml_vocab = std::fs::read_to_string(refs_dir.join("SysML-vocab.ttl")).unwrap();

    let kerml_types = sysml_codegen::parse_ttl_vocab(&kerml_vocab).unwrap();
    let sysml_types = sysml_codegen::parse_ttl_vocab(&sysml_vocab).unwrap();
    let node_types =
        sysml_spec_tests::treesitter_validation::load_node_types(&node_types_path).unwrap();

    let coverage = sysml_spec_tests::treesitter_validation::validate_element_types(
        &kerml_types,
        &sysml_types,
        &node_types,
    );

    println!("\n=== Element Type Coverage ===");
    println!(
        "Total spec Definition/Usage types: {}",
        coverage.total_spec_types
    );
    println!(
        "Explicit tree-sitter rules:        {}",
        coverage.explicit_rules.len()
    );
    println!(
        "Generic fallback coverage:         {}",
        coverage.generic_coverage.len()
    );
    println!(
        "Missing:                           {}",
        coverage.missing.len()
    );
    println!(
        "Coverage:                          {:.1}%",
        coverage.coverage_percent
    );

    if !coverage.missing.is_empty() {
        println!("\nMissing types (need tree-sitter rules):");
        for name in &coverage.missing {
            let expected = sysml_spec_tests::treesitter_validation::spec_to_treesitter_name(name);
            println!("  {} -> {}", name, expected);
        }
    }

    // Soft assertion: coverage should be > 0
    assert!(
        coverage.coverage_percent > 0.0,
        "Zero coverage — check spec parsing and node-types.json"
    );
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn treesitter_enum_validation() {
    let refs_dir = sysml_spec_tests::find_references_dir();
    let ts_dir = find_treesitter_dir();

    let grammar_js_path = ts_dir.join("grammar.js");
    assert!(
        grammar_js_path.exists(),
        "grammar.js not found at {}",
        grammar_js_path.display()
    );

    let grammar_enums =
        sysml_spec_tests::treesitter_validation::parse_grammar_enums(&grammar_js_path).unwrap();

    // Validate against xtext (grammar-level enums)
    let xtext_path = refs_dir.join(
        "SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext",
    );
    if xtext_path.exists() {
        let xtext_content = std::fs::read_to_string(&xtext_path).unwrap();
        let xtext_enums = sysml_codegen::parse_xtext_enums(&xtext_content);
        let coverage = sysml_spec_tests::treesitter_validation::validate_enums_xtext(
            &xtext_enums,
            &grammar_enums,
        );

        println!("\n=== Enum Coverage (vs xtext) ===");
        println!("Matching:         {}", coverage.matching.len());
        println!("Spec-only:        {}", coverage.spec_only.len());
        println!("Grammar-only:     {}", coverage.grammar_only.len());
        println!("Value mismatches: {}", coverage.value_mismatches.len());

        for m in &coverage.value_mismatches {
            println!("\n  MISMATCH: {}", m.name);
            println!("    xtext:   {:?}", m.spec_values);
            println!("    grammar: {:?}", m.grammar_values);
            if !m.missing_from_grammar.is_empty() {
                println!("    missing: {:?}", m.missing_from_grammar);
            }
            if !m.extra_in_grammar.is_empty() {
                println!("    extra:   {:?}", m.extra_in_grammar);
            }
        }

        for name in &coverage.spec_only {
            println!("\n  MISSING: {} (in xtext but not in grammar enum assets)", name);
        }

        // SUP-02: the enum gate is a HARD failure on any *unexplained* divergence
        // between the xtext enum keywords and the tree-sitter grammar's enum
        // assets. Historically only `value_mismatches` failed and `spec_only`
        // enums were merely printed ("the enum check reports spec-only enums
        // without failing", retired-internal plan §4.3); worse, once the enums moved
        // from an inline `const enums` block to the modular generated/enums.js
        // asset, extraction silently returned an empty map and the whole gate
        // passed vacuously. With extraction repaired (parse_grammar_enums reads
        // the modular asset), both surfaces now fail unless the enum carries an
        // explained ENUM_EXCEPTIONS entry.
        let exceptions: std::collections::HashSet<&str> =
            ENUM_EXCEPTIONS.iter().map(|(n, _)| *n).collect();

        let unexplained_spec_only: Vec<&String> = coverage
            .spec_only
            .iter()
            .filter(|n| !exceptions.contains(n.as_str()))
            .collect();
        let unexplained_mismatches: Vec<&String> = coverage
            .value_mismatches
            .iter()
            .map(|m| &m.name)
            .filter(|n| !exceptions.contains(n.as_str()))
            .collect();

        assert!(
            unexplained_spec_only.is_empty(),
            "SUP-02: {} enum(s) present in xtext but absent from the tree-sitter \
             grammar enum assets with no explained exception: {:?}. Add a grammar \
             enum or an ENUM_EXCEPTIONS entry with a rationale.",
            unexplained_spec_only.len(),
            unexplained_spec_only,
        );
        assert!(
            unexplained_mismatches.is_empty(),
            "SUP-02: {} enum value mismatch(es) vs xtext with no explained \
             exception: {:?}. The grammar enum assets are out of sync with xtext.",
            unexplained_mismatches.len(),
            unexplained_mismatches,
        );
    } else {
        eprintln!("SKIPPED: SysML.xtext not found, cannot validate enums against xtext grammar");
    }
}

/// Verify that all grammar keyword nodes are captured in highlights.scm.
///
/// This catches drift between grammar.js changes (adding/renaming keywords) and
/// the highlights.scm query file. Regression: missing keyword highlighting.
#[test]
fn highlights_scm_covers_all_grammar_keywords() {
    let ts_dir = find_treesitter_dir();

    // 1. Load node-types.json
    let node_types_path = ts_dir.join("src/node-types.json");
    if !node_types_path.exists() {
        eprintln!(
            "SKIPPED: node-types.json not found at {}. Run `npm run build` first.",
            node_types_path.display()
        );
        return;
    }

    let node_types_content =
        std::fs::read_to_string(&node_types_path).expect("Failed to read node-types.json");
    let node_types: Vec<serde_json::Value> =
        serde_json::from_str(&node_types_content).expect("Failed to parse node-types.json");

    // 2. Extract anonymous keyword nodes (named=false, type is all alphabetic/underscore)
    let grammar_keywords: std::collections::HashSet<String> = node_types
        .iter()
        .filter(|entry| entry.get("named").and_then(|v| v.as_bool()) == Some(false))
        .filter_map(|entry| entry.get("type").and_then(|v| v.as_str()).map(String::from))
        .filter(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic() || c == '_'))
        .collect();

    // 3. Load highlights.scm from tree-sitter queries dir
    let highlights_path = ts_dir.join("queries/highlights.scm");
    assert!(
        highlights_path.exists(),
        "highlights.scm not found at {}",
        highlights_path.display()
    );
    let highlights_content =
        std::fs::read_to_string(&highlights_path).expect("Failed to read highlights.scm");

    // 4. Extract keyword strings from highlights.scm
    // Keywords appear as:
    //   - Bare lines inside [...] @keyword blocks: "keyword"
    //   - Standalone patterns: "keyword" @capture
    //   - Bracket patterns: ["true" "false"] @constant.builtin
    let mut highlights_keywords = std::collections::HashSet::new();
    let mut in_content = &highlights_content[..];
    while let Some(start) = in_content.find('"') {
        in_content = &in_content[start + 1..];
        if let Some(end) = in_content.find('"') {
            let keyword = &in_content[..end];
            if !keyword.is_empty()
                && keyword
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                highlights_keywords.insert(keyword.to_string());
            }
            in_content = &in_content[end + 1..];
        } else {
            break;
        }
    }

    // 5. Compute missing keywords
    let mut missing: Vec<&str> = grammar_keywords
        .iter()
        .filter(|kw| !highlights_keywords.contains(*kw))
        .map(|s| s.as_str())
        .collect();
    missing.sort();

    // 6. Report
    println!("\n=== Keyword Highlight Coverage ===");
    println!("Grammar keywords:     {}", grammar_keywords.len());
    println!("highlights.scm keywords: {}", highlights_keywords.len());
    println!("Missing:              {}", missing.len());

    if !missing.is_empty() {
        println!("\nKeywords in grammar but NOT in highlights.scm:");
        for kw in &missing {
            println!("  \"{}\"", kw);
        }
    }

    assert!(
        missing.is_empty(),
        "{} grammar keywords missing from highlights.scm: {:?}\n\
         Add them to the [...] @keyword block in queries/highlights.scm",
        missing.len(),
        missing
    );
}
