//! Basic Ownership Example
//!
//! Demonstrates SysML v2 compliant ownership through Memberships.
//!
//! Run with: cargo run --example basic_ownership -p sysml-core

use sysml_core::{ElementFactory, ModelGraph, VisibilityKind};

fn main() {
    println!("=== SysML v2 Membership-Based Ownership ===\n");

    let mut graph = ModelGraph::new();

    // Create a root package (no owner needed for packages)
    let vehicle_pkg = ElementFactory::package("VehicleLibrary");
    let vehicle_pkg_id = graph.add_element(vehicle_pkg);
    println!("Created root package: VehicleLibrary");

    // Add owned elements using the new membership-based ownership
    let vehicle_def = ElementFactory::part_definition("Vehicle");
    let vehicle_def_id =
        graph.add_owned_element(vehicle_def, vehicle_pkg_id.clone(), VisibilityKind::Public);
    println!("Added Vehicle definition (public)");

    let engine_def = ElementFactory::part_definition("Engine");
    let _engine_def_id =
        graph.add_owned_element(engine_def, vehicle_pkg_id.clone(), VisibilityKind::Public);
    println!("Added Engine definition (public)");

    // Add a private helper definition
    let internal_helper = ElementFactory::part_definition("InternalHelper");
    let _helper_id = graph.add_owned_element(
        internal_helper,
        vehicle_pkg_id.clone(),
        VisibilityKind::Private,
    );
    println!("Added InternalHelper definition (private)");

    // Add nested parts inside Vehicle
    let engine_usage = ElementFactory::part_usage("engine");
    let engine_usage_id =
        graph.add_owned_element(engine_usage, vehicle_def_id.clone(), VisibilityKind::Public);
    println!("Added engine usage inside Vehicle");

    let wheels = ElementFactory::part_usage("wheels");
    let _wheels_id =
        graph.add_owned_element(wheels, vehicle_def_id.clone(), VisibilityKind::Public);
    println!("Added wheels usage inside Vehicle");

    println!("\n=== Ownership Queries ===\n");

    // Query ownership relationships
    let vehicle_owner = graph.owner_of(&vehicle_def_id).unwrap();
    println!(
        "Vehicle's owner: {:?}",
        vehicle_owner.name.as_ref().unwrap()
    );

    let engine_usage_owner = graph.owner_of(&engine_usage_id).unwrap();
    println!(
        "engine usage's owner: {:?}",
        engine_usage_owner.name.as_ref().unwrap()
    );

    // Get ancestors (ownership chain)
    let ancestors = graph.ancestors(&engine_usage_id);
    println!("\nAncestors of 'engine' usage:");
    for (i, ancestor) in ancestors.iter().enumerate() {
        println!(
            "  {}: {:?} ({:?})",
            i + 1,
            ancestor.name.as_ref().unwrap_or(&"<unnamed>".to_string()),
            ancestor.kind
        );
    }

    // Build qualified names
    println!("\n=== Qualified Names ===\n");
    if let Some(qname) = graph.build_qualified_name(&engine_usage_id) {
        println!("engine usage qualified name: {}", qname);
    }
    if let Some(qname) = graph.build_qualified_name(&vehicle_def_id) {
        println!("Vehicle definition qualified name: {}", qname);
    }

    // Check root and depth
    println!("\n=== Hierarchy Info ===\n");
    println!("VehicleLibrary is root: {}", graph.is_root(&vehicle_pkg_id));
    println!("Vehicle is root: {}", graph.is_root(&vehicle_def_id));
    println!(
        "Depth of VehicleLibrary: {}",
        graph.depth_of(&vehicle_pkg_id).unwrap()
    );
    println!(
        "Depth of Vehicle: {}",
        graph.depth_of(&vehicle_def_id).unwrap()
    );
    println!(
        "Depth of engine usage: {}",
        graph.depth_of(&engine_usage_id).unwrap()
    );

    // Get the owning membership element
    println!("\n=== Owning Membership Details ===\n");
    if let Some(membership) = graph.owning_membership_of(&vehicle_def_id) {
        println!("Vehicle's owning membership ID: {}", membership.id);
        println!("Membership kind: {:?}", membership.kind);

        // Use the membership view to access properties
        if let Some(view) = membership.as_owning_membership_view() {
            println!("Visibility: {:?}", view.visibility());
            if let Some(name) = view.member_name() {
                println!("Member name: {}", name);
            }
        }
    }

    println!("\n=== Summary ===\n");
    println!("Total elements: {}", graph.element_count());
    println!("Root elements: {}", graph.roots().count());
}
