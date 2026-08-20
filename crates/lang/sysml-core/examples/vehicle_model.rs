//! Vehicle Model Example
//!
//! A comprehensive example showing how to build a realistic SysML v2 model
//! of a vehicle system with definitions, usages, and requirements.
//!
//! Run with: cargo run --example vehicle_model -p sysml-core

use sysml_core::{ElementFactory, ModelGraph, Value, VisibilityKind};

fn main() {
    println!("=== SysML v2 Vehicle Model Example ===\n");

    let mut graph = ModelGraph::new();

    // ==========================================
    // Create the package structure
    // ==========================================
    println!("Building package structure...\n");

    // Root package
    let vehicle_project = ElementFactory::package("VehicleProject");
    let project_id = graph.add_element(vehicle_project);

    // Sub-packages
    let definitions_pkg = ElementFactory::package("Definitions");
    let definitions_id =
        graph.add_owned_element(definitions_pkg, project_id.clone(), VisibilityKind::Public);

    let requirements_pkg = ElementFactory::package("Requirements");
    let requirements_id =
        graph.add_owned_element(requirements_pkg, project_id.clone(), VisibilityKind::Public);

    let instances_pkg = ElementFactory::package("Instances");
    let instances_id =
        graph.add_owned_element(instances_pkg, project_id.clone(), VisibilityKind::Public);

    // ==========================================
    // Create part definitions
    // ==========================================
    println!("Creating part definitions...\n");

    // Vehicle definition (abstract)
    let vehicle_def = ElementFactory::abstract_part_definition("Vehicle");
    let vehicle_def_id =
        graph.add_owned_element(vehicle_def, definitions_id.clone(), VisibilityKind::Public);

    // Engine definition
    let mut engine_def = ElementFactory::part_definition("Engine");
    engine_def.set_prop("maxPower", Value::Float(150.0));
    engine_def.set_prop("displacement", Value::Float(2.0));
    let engine_def_id =
        graph.add_owned_element(engine_def, definitions_id.clone(), VisibilityKind::Public);

    // Wheel definition
    let mut wheel_def = ElementFactory::part_definition("Wheel");
    wheel_def.set_prop("diameter", Value::Float(17.0));
    let wheel_def_id =
        graph.add_owned_element(wheel_def, definitions_id.clone(), VisibilityKind::Public);

    // Brake definition
    let brake_def = ElementFactory::part_definition("Brake");
    let brake_def_id =
        graph.add_owned_element(brake_def, definitions_id.clone(), VisibilityKind::Public);

    // Concrete vehicle types
    let sedan_def = ElementFactory::part_definition("Sedan");
    let sedan_def_id =
        graph.add_owned_element(sedan_def, definitions_id.clone(), VisibilityKind::Public);

    let suv_def = ElementFactory::part_definition("SUV");
    let suv_def_id =
        graph.add_owned_element(suv_def, definitions_id.clone(), VisibilityKind::Public);

    // ==========================================
    // Add parts to Vehicle definition
    // ==========================================
    println!("Adding parts to Vehicle definition...\n");

    // Engine inside Vehicle
    let mut engine_usage = ElementFactory::part_usage("engine");
    engine_usage.set_prop("typeRef", Value::String("Engine".to_string()));
    let _engine_usage_id =
        graph.add_owned_element(engine_usage, vehicle_def_id.clone(), VisibilityKind::Public);

    // Wheels inside Vehicle (multiplicity would be [4])
    let mut front_left = ElementFactory::part_usage("frontLeftWheel");
    front_left.set_prop("typeRef", Value::String("Wheel".to_string()));
    graph.add_owned_element(front_left, vehicle_def_id.clone(), VisibilityKind::Public);

    let mut front_right = ElementFactory::part_usage("frontRightWheel");
    front_right.set_prop("typeRef", Value::String("Wheel".to_string()));
    graph.add_owned_element(front_right, vehicle_def_id.clone(), VisibilityKind::Public);

    let mut rear_left = ElementFactory::part_usage("rearLeftWheel");
    rear_left.set_prop("typeRef", Value::String("Wheel".to_string()));
    graph.add_owned_element(rear_left, vehicle_def_id.clone(), VisibilityKind::Public);

    let mut rear_right = ElementFactory::part_usage("rearRightWheel");
    rear_right.set_prop("typeRef", Value::String("Wheel".to_string()));
    graph.add_owned_element(rear_right, vehicle_def_id.clone(), VisibilityKind::Public);

    // ==========================================
    // Add brake to Wheel definition
    // ==========================================
    let mut brake_usage = ElementFactory::part_usage("brake");
    brake_usage.set_prop("typeRef", Value::String("Brake".to_string()));
    graph.add_owned_element(brake_usage, wheel_def_id.clone(), VisibilityKind::Public);

    // ==========================================
    // Create requirements
    // ==========================================
    println!("Creating requirements...\n");

    let mut safety_req = ElementFactory::requirement_definition("SafetyRequirement");
    safety_req.set_prop(
        "text",
        Value::String("The vehicle shall meet all safety standards".to_string()),
    );
    let safety_req_id =
        graph.add_owned_element(safety_req, requirements_id.clone(), VisibilityKind::Public);

    let mut brake_req = ElementFactory::requirement_usage("BrakePerformance");
    brake_req.set_prop(
        "text",
        Value::String("Brakes shall stop vehicle from 100km/h in < 40m".to_string()),
    );
    brake_req.set_prop("priority", Value::String("high".to_string()));
    graph.add_owned_element(brake_req, safety_req_id.clone(), VisibilityKind::Public);

    let mut emission_req = ElementFactory::requirement_definition("EmissionRequirement");
    emission_req.set_prop(
        "text",
        Value::String("Engine emissions shall comply with Euro 6 standards".to_string()),
    );
    graph.add_owned_element(
        emission_req,
        requirements_id.clone(),
        VisibilityKind::Public,
    );

    // ==========================================
    // Create vehicle instances
    // ==========================================
    println!("Creating vehicle instances...\n");

    let mut my_sedan = ElementFactory::part_usage("mySportSedan");
    my_sedan.set_prop("typeRef", Value::String("Sedan".to_string()));
    my_sedan.set_prop("color", Value::String("red".to_string()));
    my_sedan.set_prop("year", Value::Int(2024));
    let my_sedan_id =
        graph.add_owned_element(my_sedan, instances_id.clone(), VisibilityKind::Public);

    let mut my_suv = ElementFactory::part_usage("myFamilySUV");
    my_suv.set_prop("typeRef", Value::String("SUV".to_string()));
    my_suv.set_prop("color", Value::String("blue".to_string()));
    my_suv.set_prop("year", Value::Int(2024));
    my_suv.set_prop("seatingCapacity", Value::Int(7));
    graph.add_owned_element(my_suv, instances_id.clone(), VisibilityKind::Public);

    // ==========================================
    // Print the model summary
    // ==========================================
    println!("\n=== Model Summary ===\n");

    println!("Total elements: {}", graph.element_count());
    println!("Root packages: {}", graph.roots().count());

    println!("\n--- Package Structure ---\n");
    print_hierarchy(&graph, &project_id, 0);

    // ==========================================
    // Demonstrate queries
    // ==========================================
    println!("\n=== Queries ===\n");

    // Resolve qualified names
    let qnames = [
        "VehicleProject::Definitions::Vehicle",
        "VehicleProject::Definitions::Vehicle::engine",
        "VehicleProject::Requirements::SafetyRequirement::BrakePerformance",
        "VehicleProject::Instances::mySportSedan",
    ];

    println!("Resolving qualified names:");
    for qname in &qnames {
        match graph.resolve_qname(qname) {
            Some(elem) => {
                let kind = &elem.kind;
                println!("  {} -> {:?}", qname, kind);
            }
            None => println!("  {} -> NOT FOUND", qname),
        }
    }

    // Find all definitions
    println!("\nAll PartDefinition elements in Definitions package:");
    for member in graph.owned_members(&definitions_id) {
        if member.kind.is_definition() {
            if let Some(name) = &member.name {
                let is_abstract = member
                    .get_prop("isAbstract")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let abstract_str = if is_abstract { " (abstract)" } else { "" };
                println!("  - {}{}", name, abstract_str);
            }
        }
    }

    // Find all requirements
    println!("\nAll requirements:");
    for desc in graph.descendants(&requirements_id) {
        if let Some(name) = &desc.name {
            let text = desc
                .get_prop("text")
                .and_then(|v| v.as_str())
                .unwrap_or("<no text>");
            let truncated = if text.len() > 50 {
                format!("{}...", &text[..47])
            } else {
                text.to_string()
            };
            println!("  - {}: {}", name, truncated);
        }
    }

    // Check instance properties
    println!("\nInstance 'mySportSedan' properties:");
    if let Some(sedan) = graph.get_element(&my_sedan_id) {
        for (key, value) in &sedan.props {
            println!("  {}: {}", key, value);
        }
    }

    // Validate the model
    println!("\n=== Validation ===\n");
    let errors = graph.validate_structure();
    if errors.is_empty() {
        println!("Model is structurally valid!");
    } else {
        println!("Found {} structural error(s):", errors.len());
        for error in &errors {
            println!("  - {}", error);
        }
    }

    // Print unused definitions for demonstration
    let _ = (engine_def_id, brake_def_id, sedan_def_id, suv_def_id);
}

fn print_hierarchy(graph: &ModelGraph, element_id: &sysml_id::ElementId, depth: usize) {
    if let Some(element) = graph.get_element(element_id) {
        let indent = "  ".repeat(depth);
        let unnamed = "<unnamed>".to_string();
        let name = element.name.as_ref().unwrap_or(&unnamed);
        let kind = format!("{:?}", element.kind);

        // Get visibility if this element has an owning membership
        let visibility = if let Some(membership) = graph.owning_membership_of(element_id) {
            if let Some(view) = membership.as_owning_membership_view() {
                format!(" [{}]", view.visibility().as_str())
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        println!("{}{} ({}){}", indent, name, kind, visibility);

        // Print children
        for child in graph.owned_members(element_id) {
            print_hierarchy(graph, &child.id, depth + 1);
        }
    }
}
