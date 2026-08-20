//! End-to-end demo: parse SysML text with pest, resolve names, and validate.
//!
//! Run with:
//! ```bash
//! cargo run -p sysml-core --example end_to_end
//! ```

use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

const DEMO_MODEL: &str = r#"
package Demo {
    part def Engine;
    part def Wheel;

    part def Vehicle {
        part powerplant : Engine;
        part spare : Wheel;
        part ghostAttachment : MissingPart;
    }

    part sportsCoupe : Vehicle;
    part phantomPrototype : ImaginaryVehicle;
}
"#;

fn main() {
    println!("=== SysML Text ===\n{}\n", DEMO_MODEL.trim());

    let parser = TreeSitterParser::new();
    let files = [SysmlFile::new("demo.sysml", DEMO_MODEL)];

    let mut result = parser.parse(&files);
    println!(
        "Parsed {} elements and {} relationships using {} {}",
        result.graph.element_count(),
        result.graph.relationship_count(),
        parser.name(),
        parser.version()
    );

    if result.diagnostics.is_empty() {
        println!("Parser reported no diagnostics.\n");
    } else {
        println!("Parser diagnostics:");
        for diag in &result.diagnostics {
            println!("  - {}", diag);
        }
        println!();
    }

    let resolution_stats = result.resolve();
    println!(
        "Resolution: {} references resolved, {} unresolved.\n",
        resolution_stats.resolved_count, resolution_stats.unresolved_count
    );

    result.validate_structure();
    result.validate_relationships();

    let sysml_parser_trait::ParseResult { graph, diagnostics } = result;
    print_diagnostics(&diagnostics);
    highlight_elements(&graph);
}

fn print_diagnostics(diagnostics: &[sysml_span::Diagnostic]) {
    if diagnostics.is_empty() {
        println!("No diagnostics after resolution + validation.\n");
        return;
    }

    println!("Diagnostics after resolution + validation:");
    for diag in diagnostics {
        println!("  - {}", diag);
    }
    println!();
}

fn highlight_elements(graph: &ModelGraph) {
    println!("=== Resolved Elements ===");

    if let Some(vehicle) = graph.resolve_qname("Demo::Vehicle") {
        println!(
            "Demo::Vehicle resolved to {:?} with id {}",
            vehicle.kind, vehicle.id
        );
    }

    if let Some(engine_feature) = graph.resolve_qname("Demo::Vehicle::powerplant") {
        println!(
            "Demo::Vehicle::powerplant resolved to {:?} with id {}",
            engine_feature.kind, engine_feature.id
        );
    }

    if let Some(missing) = graph.resolve_qname("Demo::Vehicle::ghostAttachment") {
        println!(
            "Demo::Vehicle::ghostAttachment still exists as {:?} (will warn because its type is unresolved)",
            missing.kind
        );
    }

    println!(
        "\nModel stats: {} elements, {} relationships",
        graph.element_count(),
        graph.relationship_count()
    );
}
