//! Namespace Resolution Example
//!
//! Demonstrates how to resolve names and qualified names in SysML v2 models.
//!
//! Run with: cargo run --example namespace_resolution -p sysml-core

use sysml_core::{ElementFactory, ModelGraph, VisibilityKind};

fn main() {
    println!("=== SysML v2 Namespace Resolution ===\n");

    let mut graph = ModelGraph::new();

    // Build a hierarchy:
    // RootPackage
    // ├── Definitions (public)
    // │   ├── Vehicle (public)
    // │   └── Engine (public)
    // ├── Usages (public)
    // │   └── myVehicle (public)
    // └── Internal (private)
    //     └── Helper (public within Internal)

    let root = ElementFactory::package("RootPackage");
    let root_id = graph.add_element(root);

    let definitions = ElementFactory::package("Definitions");
    let definitions_id =
        graph.add_owned_element(definitions, root_id.clone(), VisibilityKind::Public);

    let vehicle = ElementFactory::part_definition("Vehicle");
    let vehicle_id =
        graph.add_owned_element(vehicle, definitions_id.clone(), VisibilityKind::Public);

    let engine = ElementFactory::part_definition("Engine");
    let _engine_id =
        graph.add_owned_element(engine, definitions_id.clone(), VisibilityKind::Public);

    let usages = ElementFactory::package("Usages");
    let usages_id = graph.add_owned_element(usages, root_id.clone(), VisibilityKind::Public);

    let my_vehicle = ElementFactory::part_usage("myVehicle");
    let _my_vehicle_id =
        graph.add_owned_element(my_vehicle, usages_id.clone(), VisibilityKind::Public);

    let internal = ElementFactory::package("Internal");
    let internal_id = graph.add_owned_element(internal, root_id.clone(), VisibilityKind::Private);

    let helper = ElementFactory::part_definition("Helper");
    let _helper_id = graph.add_owned_element(helper, internal_id.clone(), VisibilityKind::Public);

    println!("Created model hierarchy:\n");
    println!("RootPackage");
    println!("├── Definitions (public)");
    println!("│   ├── Vehicle (public)");
    println!("│   └── Engine (public)");
    println!("├── Usages (public)");
    println!("│   └── myVehicle (public)");
    println!("└── Internal (private)");
    println!("    └── Helper (public)");

    // Demonstrate resolve_name within a namespace
    println!("\n=== Resolving Names Within Namespaces ===\n");

    if let Some(elem) = graph.resolve_name(&root_id, "Definitions") {
        println!(
            "resolve_name(RootPackage, 'Definitions') -> Found: {:?}",
            elem.name
        );
    }

    if let Some(elem) = graph.resolve_name(&definitions_id, "Vehicle") {
        println!(
            "resolve_name(Definitions, 'Vehicle') -> Found: {:?}",
            elem.name
        );
    }

    if graph.resolve_name(&root_id, "NonExistent").is_none() {
        println!("resolve_name(RootPackage, 'NonExistent') -> Not found (as expected)");
    }

    // Demonstrate resolve_qname from root
    println!("\n=== Resolving Qualified Names ===\n");

    let qnames = [
        "RootPackage",
        "RootPackage::Definitions",
        "RootPackage::Definitions::Vehicle",
        "RootPackage::Definitions::Engine",
        "RootPackage::Usages::myVehicle",
        "RootPackage::Internal::Helper",
        "RootPackage::NonExistent",
    ];

    for qname in &qnames {
        match graph.resolve_qname(qname) {
            Some(elem) => println!(
                "resolve_qname('{}') -> Found: {} ({:?})",
                qname,
                elem.name.as_ref().unwrap(),
                elem.kind
            ),
            None => println!("resolve_qname('{}') -> Not found", qname),
        }
    }

    // Demonstrate resolve_path (relative to a namespace)
    println!("\n=== Resolving Relative Paths ===\n");

    if let Some(elem) = graph.resolve_path(&root_id, "Definitions::Vehicle") {
        println!(
            "resolve_path(RootPackage, 'Definitions::Vehicle') -> Found: {:?}",
            elem.name
        );
    }

    if let Some(elem) = graph.resolve_path(&definitions_id, "Vehicle") {
        println!(
            "resolve_path(Definitions, 'Vehicle') -> Found: {:?}",
            elem.name
        );
    }

    // Demonstrate owned_members vs visible_members
    println!("\n=== Visibility Filtering ===\n");

    println!("All owned members of RootPackage:");
    for member in graph.owned_members(&root_id) {
        println!("  - {:?}", member.name.as_ref().unwrap());
    }

    println!("\nVisible (public) members of RootPackage:");
    for member in graph.visible_members(&root_id) {
        println!("  - {:?}", member.name.as_ref().unwrap());
    }

    // Demonstrate descendants
    println!("\n=== All Descendants of RootPackage ===\n");

    let descendants = graph.descendants(&root_id);
    println!("Found {} descendants:", descendants.len());
    for desc in &descendants {
        let qname = graph.build_qualified_name(&desc.id);
        println!(
            "  - {} ({:?})",
            qname
                .map(|q| q.to_string())
                .unwrap_or_else(|| "<no qname>".to_string()),
            desc.kind
        );
    }

    // Demonstrate is_descendant_of
    println!("\n=== Descendant Checks ===\n");

    println!(
        "Is Vehicle a descendant of RootPackage? {}",
        graph.is_descendant_of(&vehicle_id, &root_id)
    );
    println!(
        "Is Vehicle a descendant of Definitions? {}",
        graph.is_descendant_of(&vehicle_id, &definitions_id)
    );
    println!(
        "Is Vehicle a descendant of Usages? {}",
        graph.is_descendant_of(&vehicle_id, &usages_id)
    );
}
