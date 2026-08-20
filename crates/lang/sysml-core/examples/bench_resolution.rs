//! Standardized resolution benchmark for performance tracking.
//!
//! # Usage
//!
//! ## Quick timing test:
//! ```bash
//! SYSML_CORPUS_PATH=... cargo run --release -p sysml-core --example bench_resolution
//! ```
//!
//! ## Flamegraph generation:
//! ```bash
//! SYSML_CORPUS_PATH=... cargo flamegraph -p sysml-core --example bench_resolution -o /tmp/resolution.svg
//! ```
//!
//! ## With specific iteration count:
//! ```bash
//! SYSML_CORPUS_PATH=... BENCH_ITERATIONS=10 cargo run --release -p sysml-core --example bench_resolution
//! ```

use std::time::Instant;

use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

fn main() {
    let corpus_path = std::env::var("SYSML_CORPUS_PATH")
        .or_else(|_| std::env::var("SYSML_REFS_DIR"))
        .or_else(|_| std::env::var("SYSMLV2_REFS_DIR"))
        .unwrap_or_else(|_| {
            let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let repo_root = manifest_dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .unwrap_or(&manifest_dir);
            repo_root
                .join("references")
                .join("sysmlv2")
                .to_string_lossy()
                .to_string()
        });

    let iterations: usize = std::env::var("BENCH_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let parser = TreeSitterParser::new();

    // Use 5 ISQ files - representative of real library complexity
    let test_files = [
        "library.domain/Quantities and Units/ISQSpaceTime.sysml",
        "library.domain/Quantities and Units/ISQMechanics.sysml",
        "library.domain/Quantities and Units/ISQThermodynamics.sysml",
        "library.domain/Quantities and Units/ISQElectromagnetism.sysml",
        "library.domain/Quantities and Units/ISQLight.sysml",
    ];

    let files: Vec<SysmlFile> = test_files
        .iter()
        .map(|f| {
            let path = format!(
                "{}/SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/{}",
                corpus_path, f
            );
            let text = std::fs::read_to_string(&path).expect(&format!("Failed to read {}", path));
            SysmlFile { path, text }
        })
        .collect();

    eprintln!("=== Resolution Benchmark ===");
    eprintln!("Files: {}", files.len());
    eprintln!("Iterations: {}", iterations);
    eprintln!();

    // Phase 1: Parsing
    let parse_start = Instant::now();
    // TS-3.6: inlined the structure/relationship validation that
    // `PestParser::parse_with_validation` performed.
    let mut result = parser.parse(&files);
    result.validate_structure();
    result.validate_relationships();
    let parse_time = parse_start.elapsed();
    let graph = result.graph;
    eprintln!(
        "Parsing:    {:>8.2?}  ({} elements)",
        parse_time,
        graph.elements.len()
    );

    // Phase 2: Resolution (multiple iterations for accurate timing)
    let mut resolution_times = Vec::with_capacity(iterations);
    let mut resolved_count = 0;
    let mut unresolved_count = 0;

    for i in 0..iterations {
        let mut g = graph.clone();
        let res_start = Instant::now();
        let res = sysml_core::resolution::resolve_references(&mut g);
        let res_time = res_start.elapsed();
        resolution_times.push(res_time);

        if i == 0 {
            resolved_count = res.resolved_count;
            unresolved_count = res.unresolved_count;
        }
    }

    // Calculate stats
    let total_res: std::time::Duration = resolution_times.iter().sum();
    let avg_res = total_res / iterations as u32;
    let min_res = resolution_times.iter().min().unwrap();
    let max_res = resolution_times.iter().max().unwrap();

    eprintln!(
        "Resolution: {:>8.2?}  (avg of {} runs)",
        avg_res, iterations
    );
    eprintln!("  Min: {:>8.2?}", min_res);
    eprintln!("  Max: {:>8.2?}", max_res);
    eprintln!();
    eprintln!("Results:");
    eprintln!("  Resolved:   {:>6}", resolved_count);
    eprintln!("  Unresolved: {:>6}", unresolved_count);
    eprintln!(
        "  Rate:       {:>5.1}%",
        100.0 * resolved_count as f64 / (resolved_count + unresolved_count) as f64
    );
    eprintln!();

    // Output for scripting
    println!("BENCHMARK_PARSE_MS={}", parse_time.as_millis());
    println!("BENCHMARK_RESOLVE_AVG_MS={}", avg_res.as_millis());
    println!("BENCHMARK_RESOLVE_MIN_MS={}", min_res.as_millis());
    println!("BENCHMARK_ELEMENTS={}", graph.elements.len());
    println!("BENCHMARK_RESOLVED={}", resolved_count);
    println!("BENCHMARK_UNRESOLVED={}", unresolved_count);
}
