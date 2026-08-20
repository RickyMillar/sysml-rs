//! Single-file resolution debugger.
//!
//! This tool provides detailed resolution debugging for a single SysML file.
//! Useful for understanding why specific references fail to resolve.
//!
//! Run with:
//! ```bash
//! SYSML_CORPUS_PATH=/path/to/references/sysmlv2 \
//!     cargo run -p sysml-spec-tests --example resolution_debug -- path/to/file.sysml
//! ```

use std::env;
use std::fs;
use std::path::Path;

use sysml_core::crossrefs::{find_crossref_spec, ScopeStrategy};
use sysml_core::ElementKind;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::library::{load_standard_library, LibraryConfig};
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_spec_tests::CoverageConfig;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <file.sysml>", args[0]);
        eprintln!();
        eprintln!("Debug resolution for a single SysML file.");
        eprintln!();
        eprintln!("Environment variables:");
        eprintln!("  SYSML_CORPUS_PATH - Path to references/sysmlv2 (required for library)");
        std::process::exit(1);
    }

    let file_path = &args[1];

    // Check if file exists
    if !Path::new(file_path).exists() {
        eprintln!("Error: File not found: {}", file_path);
        std::process::exit(1);
    }

    // Read the file
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file {}: {}", file_path, e);
            std::process::exit(1);
        }
    };

    let parser = TreeSitterParser::new();

    // Try to load standard library if SYSML_CORPUS_PATH is set
    let library = if let Some(config) = CoverageConfig::from_env() {
        println!("Loading standard library...");
        let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
        match load_standard_library(&parser, &lib_config) {
            Ok(lib) => {
                println!("Library loaded: {} elements\n", lib.element_count());
                Some(lib)
            }
            Err(e) => {
                eprintln!("Warning: Could not load library: {}", e);
                eprintln!("Continuing without library...\n");
                None
            }
        }
    } else {
        println!("Warning: SYSML_CORPUS_PATH not set, running without library\n");
        None
    };

    // Parse the file
    println!("=== Parsing: {} ===\n", file_path);
    let sysml_file = SysmlFile::new(file_path, &content);
    let mut result = parser.parse(&[sysml_file]);

    // Report parse errors
    if result.has_errors() {
        println!("Parse errors ({}):", result.error_count());
        for diag in result.diagnostics.iter().filter(|d| d.is_error()) {
            println!("  {}", diag);
        }
        println!();
    }

    // Report elements parsed
    let element_count = result.graph.element_count();
    println!("Elements parsed: {}", element_count);

    // Count by element kind
    let mut by_kind: std::collections::HashMap<ElementKind, usize> =
        std::collections::HashMap::new();
    for (_, elem) in result.graph.elements.iter() {
        *by_kind.entry(elem.kind.clone()).or_default() += 1;
    }
    let mut kinds: Vec<_> = by_kind.into_iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nElement kinds:");
    for (kind, count) in kinds.iter().take(15) {
        println!("  {:?}: {}", kind, count);
    }
    if kinds.len() > 15 {
        println!("  ... and {} more kinds", kinds.len() - 15);
    }

    // Resolve
    println!("\n=== Running Resolution ===\n");
    let res = if let Some(lib) = library {
        result.resolve_with_library(lib)
    } else {
        result.resolve()
    };

    println!("Resolved: {}", res.resolved_count);
    println!("Unresolved: {}", res.unresolved_count);

    let total = res.resolved_count + res.unresolved_count;
    if total > 0 {
        let rate = 100.0 * res.resolved_count as f64 / total as f64;
        println!("Resolution rate: {:.1}%", rate);
    }

    // Detailed unresolved report
    if res.unresolved_count > 0 {
        println!("\n=== Unresolved References ===\n");

        // Group by property
        let mut by_prop: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
            let msg = diag.to_string();

            // Extract the name and property
            let mut name = String::new();
            let mut prop = String::new();

            if let Some(start) = msg.find('\'') {
                if let Some(end) = msg[start + 1..].find('\'') {
                    name = msg[start + 1..start + 1 + end].to_string();
                }
            }

            if let Some(prop_start) = msg.rfind("property '") {
                let prop_offset = prop_start + 10;
                if let Some(prop_end) = msg[prop_offset..].find('\'') {
                    prop = msg[prop_offset..prop_offset + prop_end].to_string();
                }
            }

            by_prop.entry(prop).or_default().push(name);
        }

        // Print grouped
        let mut props: Vec<_> = by_prop.into_iter().collect();
        props.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        for (prop, names) in props {
            // Get scoping strategy
            let strategy = find_crossref_spec(&prop)
                .map(|s| format!("{:?}", s.scope))
                .unwrap_or_else(|| "Unknown".to_string());

            println!("Property '{}' (strategy: {}):", prop, strategy);
            println!("  {} unresolved names:", names.len());

            // Dedupe and count
            let mut name_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for name in names {
                *name_counts.entry(name).or_default() += 1;
            }
            let mut sorted: Vec<_> = name_counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));

            for (name, count) in sorted.iter().take(10) {
                if *count > 1 {
                    println!("    '{}' ({}x)", name, count);
                } else {
                    println!("    '{}'", name);
                }
            }
            if sorted.len() > 10 {
                println!("    ... and {} more", sorted.len() - 10);
            }
            println!();
        }

        // Print recommendations
        println!("=== Analysis ===\n");

        // Check for common library type issues
        let common_lib_types = [
            "Real",
            "Integer",
            "String",
            "Boolean",
            "Anything",
            "DataValue",
        ];
        let mut missing_lib_types: Vec<&str> = Vec::new();

        for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
            let msg = diag.to_string();
            for lib_type in &common_lib_types {
                if msg.contains(&format!("'{}'", lib_type)) {
                    if !missing_lib_types.contains(lib_type) {
                        missing_lib_types.push(lib_type);
                    }
                }
            }
        }

        if !missing_lib_types.is_empty() {
            println!("Missing standard library types: {:?}", missing_lib_types);
            println!("Ensure SYSML_CORPUS_PATH is set correctly.\n");
        }

        // Check for potential scoping issues
        let non_owning_strategies = [
            ScopeStrategy::NonExpressionNamespace,
            ScopeStrategy::RelativeNamespace,
            ScopeStrategy::TransitionSpecific,
        ];

        let mut potential_scoping_issues = false;
        for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
            let msg = diag.to_string();
            if let Some(prop_start) = msg.rfind("property '") {
                let prop_offset = prop_start + 10;
                if let Some(prop_end) = msg[prop_offset..].find('\'') {
                    let prop = &msg[prop_offset..prop_offset + prop_end];
                    if let Some(spec) = find_crossref_spec(prop) {
                        if non_owning_strategies.contains(&spec.scope) {
                            potential_scoping_issues = true;
                            break;
                        }
                    }
                }
            }
        }

        if potential_scoping_issues {
            println!("Some unresolved references use non-standard scoping strategies.");
            println!("These may require fixes to the scoping implementation.\n");
        }
    } else {
        println!("\nAll references resolved successfully!");
    }
}
