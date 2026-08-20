//! Structural Validation Example
//!
//! Demonstrates how to validate model structure and detect common errors.
//!
//! Run with: cargo run --example structural_validation -p sysml-core

use sysml_core::{
    Element, ElementFactory, ElementKind, ModelGraph, StructuralError, VisibilityKind,
};
use sysml_id::ElementId;

fn main() {
    println!("=== SysML v2 Structural Validation ===\n");

    // Example 1: Valid model
    println!("--- Example 1: Valid Model ---\n");
    validate_model(create_valid_model());

    // Example 2: Orphan element
    println!("\n--- Example 2: Orphan Element ---\n");
    validate_model(create_model_with_orphan());

    // Example 3: Ownership cycle
    println!("\n--- Example 3: Ownership Cycle ---\n");
    validate_model(create_model_with_cycle());

    // Example 4: Dangling owning membership
    println!("\n--- Example 4: Dangling Owning Membership ---\n");
    validate_model(create_model_with_dangling_membership());

    // Example 5: Invalid owning membership type
    println!("\n--- Example 5: Invalid Owning Membership Type ---\n");
    validate_model(create_model_with_invalid_membership_type());

    // Example 6: Multiple errors
    println!("\n--- Example 6: Multiple Errors ---\n");
    validate_model(create_model_with_multiple_errors());
}

fn validate_model(graph: ModelGraph) {
    let errors = graph.validate_structure();

    if errors.is_empty() {
        println!("Model is valid! No structural errors found.");
    } else {
        println!("Found {} structural error(s):", errors.len());
        for (i, error) in errors.iter().enumerate() {
            println!("  {}. {}", i + 1, error);
            print_error_details(error);
        }
    }
}

fn print_error_details(error: &StructuralError) {
    match error {
        StructuralError::OrphanElement { element_kind, .. } => {
            println!(
                "     Hint: {:?} elements need an owner (except Package types)",
                element_kind
            );
        }
        StructuralError::OwnershipCycle { element_ids } => {
            println!("     Cycle involves {} elements", element_ids.len());
        }
        StructuralError::DanglingOwningMembership { .. } => {
            println!("     Hint: Ensure the membership element exists in the graph");
        }
        StructuralError::InvalidOwningMembership {
            membership_kind, ..
        } => {
            println!(
                "     Hint: owning_membership must point to a Membership element, not {:?}",
                membership_kind
            );
        }
        _ => {}
    }
}

fn create_valid_model() -> ModelGraph {
    let mut graph = ModelGraph::new();

    let pkg = ElementFactory::package("ValidPackage");
    let pkg_id = graph.add_element(pkg);

    let part = ElementFactory::part_definition("ValidPart");
    graph.add_owned_element(part, pkg_id, VisibilityKind::Public);

    println!("Created: Package containing a PartDefinition");
    graph
}

fn create_model_with_orphan() -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Package is OK as root
    let pkg = ElementFactory::package("RootPackage");
    graph.add_element(pkg);

    // PartDefinition without owner is NOT OK
    let orphan = ElementFactory::part_definition("OrphanPart");
    graph.add_element(orphan);

    println!("Created: Package + orphan PartDefinition (no owner)");
    graph
}

fn create_model_with_cycle() -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Create two packages
    let pkg_a = Element::new_with_kind(ElementKind::Package).with_name("PackageA");
    let pkg_a_id = pkg_a.id.clone();
    graph.add_element(pkg_a);

    let pkg_b = Element::new_with_kind(ElementKind::Package)
        .with_name("PackageB")
        .with_owner(pkg_a_id.clone());
    let pkg_b_id = pkg_b.id.clone();
    graph.add_element(pkg_b);

    // Manually create a cycle by setting A's owner to B
    if let Some(a) = graph.get_element_mut(&pkg_a_id) {
        a.owner = Some(pkg_b_id);
    }

    println!("Created: PackageA -> PackageB -> PackageA (cycle)");
    graph
}

fn create_model_with_dangling_membership() -> ModelGraph {
    let mut graph = ModelGraph::new();

    let pkg = ElementFactory::package("MyPackage");
    graph.add_element(pkg);

    // Create a part with a fake owning_membership ID that doesn't exist
    let fake_membership_id = ElementId::new_v4();
    let part = Element::new_with_kind(ElementKind::PartDefinition)
        .with_name("DanglingPart")
        .with_owning_membership(fake_membership_id);
    graph.add_element(part);

    println!("Created: PartDefinition with non-existent owning_membership");
    graph
}

fn create_model_with_invalid_membership_type() -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Create a package
    let pkg = ElementFactory::package("MyPackage");
    let pkg_id = graph.add_element(pkg);

    // Create a part that incorrectly points to the Package as its owning_membership
    // (owning_membership should point to a Membership element, not a Package)
    let part = Element::new_with_kind(ElementKind::PartDefinition)
        .with_name("BadPart")
        .with_owning_membership(pkg_id); // Wrong! Should be a Membership ID
    graph.add_element(part);

    println!("Created: PartDefinition with owning_membership pointing to Package (wrong type)");
    graph
}

fn create_model_with_multiple_errors() -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Valid package
    let pkg = ElementFactory::package("ValidPackage");
    let pkg_id = graph.add_element(pkg);

    // Valid owned part
    let valid_part = ElementFactory::part_definition("ValidPart");
    graph.add_owned_element(valid_part, pkg_id.clone(), VisibilityKind::Public);

    // Error 1: Orphan
    let orphan = ElementFactory::action_definition("OrphanAction");
    graph.add_element(orphan);

    // Error 2: Another orphan
    let orphan2 = ElementFactory::requirement_definition("OrphanRequirement");
    graph.add_element(orphan2);

    // Error 3: Dangling membership
    let dangling = Element::new_with_kind(ElementKind::PartDefinition)
        .with_name("DanglingPart")
        .with_owning_membership(ElementId::new_v4());
    graph.add_element(dangling);

    println!("Created: Model with 3 errors (2 orphans + 1 dangling membership)");
    graph
}
