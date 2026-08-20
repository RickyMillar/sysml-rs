//! Resolution failure categorizer.
//!
//! This tool categorizes unresolved references by scoping strategy to help
//! identify which resolution implementations need the most work.
//!
//! Run with:
//! ```bash
//! SYSML_CORPUS_PATH=/path/to/references/sysmlv2 \
//!     cargo run -p sysml-spec-tests --example categorize_failures
//! ```

use std::collections::HashMap;

use sysml_core::crossrefs::{find_crossref_spec, ScopeStrategy, ALL_CROSS_REFERENCES};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::library::{load_standard_library, LibraryConfig};
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_spec_tests::{corpus::discover_corpus_files, load_allow_list, CoverageConfig};

/// Maps unresolved property keys to their crossref property names.
fn unresolved_prop_to_crossref(prop_key: &str) -> Option<&str> {
    // The prop_key is like "unresolved_general", we need "general"
    prop_key.strip_prefix("unresolved_")
}

/// Get the scoping strategy for an unresolved property.
fn get_strategy_for_prop(prop_key: &str) -> Option<ScopeStrategy> {
    unresolved_prop_to_crossref(prop_key)
        .and_then(find_crossref_spec)
        .map(|spec| spec.scope)
}

fn main() {
    let config = match CoverageConfig::from_env() {
        Some(c) => c,
        None => {
            eprintln!("Error: Failed to load configuration");
            eprintln!("Set SYSML_CORPUS_PATH to references/sysmlv2");
            std::process::exit(1);
        }
    };

    let parser = TreeSitterParser::new();

    // Load the standard library
    println!("Loading standard library...");
    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let library = match load_standard_library(&parser, &lib_config) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("Failed to load standard library: {}", e);
            std::process::exit(1);
        }
    };
    println!("Library loaded: {} elements", library.element_count());

    // Load expected failures
    const EXPECTED_FAILURES: &str = include_str!("../data/expected_failures.txt");
    let allow_list = load_allow_list(EXPECTED_FAILURES);

    // Discover and filter corpus files
    let files = discover_corpus_files(&config);
    let mut model_files: Vec<SysmlFile> = Vec::new();

    for file in &files {
        // Skip files in allow list
        let in_allow_list = allow_list.contains(&file.relative_path)
            || allow_list.iter().any(|pattern| {
                if pattern.starts_with("**/") {
                    file.relative_path.ends_with(&pattern[3..])
                } else {
                    file.relative_path.contains(pattern)
                }
            });

        if in_allow_list {
            continue;
        }

        // Skip library files
        if file.relative_path.contains("library.kernel")
            || file.relative_path.contains("library.systems")
        {
            continue;
        }

        model_files.push(SysmlFile::new(&file.relative_path, &file.content));
    }

    println!("Parsing {} model files...", model_files.len());

    // Parse all files together
    let mut result = parser.parse(&model_files);
    let parse_errors = result.error_count();
    let element_count = result.graph.element_count();
    println!(
        "Parse complete: {} elements, {} parse errors",
        element_count, parse_errors
    );

    // Resolve with library
    println!("Resolving references...");
    let res = result.resolve_with_library(library);

    println!("\n=== Resolution Failure Analysis ===\n");
    println!("Total resolved: {}", res.resolved_count);
    println!("Total unresolved: {}", res.unresolved_count);

    let total = res.resolved_count + res.unresolved_count;
    if total > 0 {
        let rate = 100.0 * res.resolved_count as f64 / total as f64;
        println!("Resolution rate: {:.1}%", rate);
    }

    // Categorize unresolved by strategy
    let mut by_strategy: HashMap<ScopeStrategy, Vec<(String, String)>> = HashMap::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut unknown_props: HashMap<String, usize> = HashMap::new();

    for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
        let msg = diag.to_string();

        // Extract property name from "Unresolved reference 'X' for property 'Y'"
        let mut unresolved_name = String::new();
        let mut prop_name = String::new();

        if let Some(start) = msg.find('\'') {
            if let Some(end) = msg[start + 1..].find('\'') {
                unresolved_name = msg[start + 1..start + 1 + end].to_string();
            }
        }

        if let Some(prop_start) = msg.rfind("property '") {
            let prop_offset = prop_start + 10;
            if let Some(prop_end) = msg[prop_offset..].find('\'') {
                prop_name = msg[prop_offset..prop_offset + prop_end].to_string();
            }
        }

        // Count by name
        *by_name.entry(unresolved_name.clone()).or_default() += 1;

        // Categorize by strategy
        if let Some(strategy) = get_strategy_for_prop(&format!("unresolved_{}", prop_name)) {
            by_strategy
                .entry(strategy)
                .or_default()
                .push((unresolved_name, prop_name));
        } else if !prop_name.is_empty() {
            // Try direct lookup
            if let Some(spec) = find_crossref_spec(&prop_name) {
                by_strategy
                    .entry(spec.scope)
                    .or_default()
                    .push((unresolved_name, prop_name));
            } else {
                *unknown_props.entry(prop_name).or_default() += 1;
            }
        }
    }

    // Print breakdown by strategy
    println!("\nBy Scoping Strategy:");
    println!("{:-<60}", "");

    let strategy_order = [
        ScopeStrategy::OwningNamespace,
        ScopeStrategy::NonExpressionNamespace,
        ScopeStrategy::RelativeNamespace,
        ScopeStrategy::FeatureChaining,
        ScopeStrategy::TransitionSpecific,
        ScopeStrategy::Global,
    ];

    for strategy in &strategy_order {
        let count = by_strategy.get(strategy).map(|v| v.len()).unwrap_or(0);
        let pct = if res.unresolved_count > 0 {
            100.0 * count as f64 / res.unresolved_count as f64
        } else {
            0.0
        };
        println!(
            "  {:30} {:6} ({:5.1}%)",
            format!("{:?}:", strategy),
            count,
            pct
        );
    }

    if !unknown_props.is_empty() {
        let total_unknown: usize = unknown_props.values().sum();
        println!(
            "\n  Unknown properties: {} ({} occurrences)",
            unknown_props.len(),
            total_unknown
        );
        for (prop, count) in &unknown_props {
            if *count > 5 {
                println!("    {} ({}x)", prop, count);
            }
        }
    }

    // Print top unresolved names
    println!("\nTop Unresolved Names:");
    println!("{:-<60}", "");

    let mut sorted_names: Vec<_> = by_name.into_iter().collect();
    sorted_names.sort_by(|a, b| b.1.cmp(&a.1));

    for (name, count) in sorted_names.iter().take(20) {
        println!("  {} ({}x)", name, count);
    }
    if sorted_names.len() > 20 {
        println!("  ... and {} more unique names", sorted_names.len() - 20);
    }

    // Print expected crossref registry stats
    println!("\nCrossref Registry Stats:");
    println!("{:-<60}", "");
    for strategy in &strategy_order {
        let count = ALL_CROSS_REFERENCES
            .iter()
            .filter(|spec| spec.scope == *strategy)
            .count();
        println!("  {:30} {} properties", format!("{:?}:", strategy), count);
    }

    // Provide analysis recommendations
    println!("\n=== Recommendations ===\n");

    // Find highest-impact strategy to fix
    let mut strategy_counts: Vec<_> = by_strategy.iter().collect();
    strategy_counts.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    if let Some((top_strategy, failures)) = strategy_counts.first() {
        if !failures.is_empty() {
            println!(
                "Highest impact fix: {:?} ({} failures, {:.1}% of unresolved)",
                top_strategy,
                failures.len(),
                100.0 * failures.len() as f64 / res.unresolved_count.max(1) as f64
            );

            // Sample some failures for this strategy
            println!("\nSample failures for {:?}:", top_strategy);
            for (name, prop) in failures.iter().take(5) {
                println!("  '{}' in property '{}'", name, prop);
            }
        }
    }
}
