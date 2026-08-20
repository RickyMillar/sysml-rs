// Minimal profiling script - time the key operations
use std::time::Instant;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{ParseResult, Parser, SysmlFile};

// Helper: parse + run structure/relationship validation (replaces the
// Pest-only `PestParser::parse_with_validation` after the TS-3.6 swap).
fn parse_with_validation(parser: &TreeSitterParser, files: &[SysmlFile]) -> ParseResult {
    let mut result = parser.parse(files);
    result.validate_structure();
    result.validate_relationships();
    result
}

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

    let parser = TreeSitterParser::new();

    // === Single File Test ===
    println!("=== Single File Profile ===");
    let test_file = format!("{}/SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.domain/Quantities and Units/ISQSpaceTime.sysml", corpus_path);
    let content = std::fs::read_to_string(&test_file).expect("Failed to read file");
    println!("File size: {} bytes", content.len());

    let file = SysmlFile {
        path: test_file.clone(),
        text: content.clone(),
    };

    let start = Instant::now();
    let result = parse_with_validation(&parser, &[file]);
    println!("Parse time: {:?}", start.elapsed());

    let mut graph = result.graph;
    println!("Elements: {}", graph.elements.len());

    let start = Instant::now();
    let res = sysml_core::resolution::resolve_references(&mut graph);
    println!("Resolution time: {:?}", start.elapsed());
    println!(
        "Resolved: {}, Unresolved: {}\n",
        res.resolved_count, res.unresolved_count
    );

    // === Multi-File Test (5 files) ===
    println!("=== Multi-File Profile (5 files) ===");
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

    let total_size: usize = files.iter().map(|f| f.text.len()).sum();
    println!("Total size: {} bytes ({} files)", total_size, files.len());

    let start = Instant::now();
    let result = parse_with_validation(&parser, &files);
    println!("Parse time: {:?}", start.elapsed());

    let mut graph = result.graph;
    println!("Elements: {}", graph.elements.len());

    let start = Instant::now();
    let res = sysml_core::resolution::resolve_references(&mut graph);
    println!("Resolution time: {:?}", start.elapsed());
    println!(
        "Resolved: {}, Unresolved: {}",
        res.resolved_count, res.unresolved_count
    );

    // === Library-size test (10 files for quick check) ===
    println!("\n=== Library-Size Profile (10 files) ===");
    let test_files_10 = [
        "library.domain/Quantities and Units/ISQSpaceTime.sysml",
        "library.domain/Quantities and Units/ISQMechanics.sysml",
        "library.domain/Quantities and Units/ISQThermodynamics.sysml",
        "library.domain/Quantities and Units/ISQElectromagnetism.sysml",
        "library.domain/Quantities and Units/ISQLight.sysml",
        "library.domain/Quantities and Units/ISQAtomicNuclear.sysml",
        "library.domain/Quantities and Units/ISQChemistryMolecular.sysml",
        "library.domain/Quantities and Units/Quantities.sysml",
        "library.domain/Quantities and Units/MeasurementReferences.sysml",
        "library.domain/Quantities and Units/UnitsAndScales.sysml",
    ];

    let lib_files: Vec<SysmlFile> = test_files_10
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

    let total_size: usize = lib_files.iter().map(|f| f.text.len()).sum();
    println!(
        "Total size: {} bytes ({} files)",
        total_size,
        lib_files.len()
    );

    let start = Instant::now();
    let result = parse_with_validation(&parser, &lib_files);
    println!("Parse time: {:?}", start.elapsed());

    let mut graph = result.graph;
    println!("Elements: {}", graph.elements.len());

    let start = Instant::now();
    let res = sysml_core::resolution::resolve_references(&mut graph);
    println!("Resolution time: {:?}", start.elapsed());
    println!(
        "Resolved: {}, Unresolved: {}",
        res.resolved_count, res.unresolved_count
    );
}
