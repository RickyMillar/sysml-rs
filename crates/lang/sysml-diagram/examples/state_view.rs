use sysml_core::{
    Element, ElementKind, ModelGraph, Relationship, RelationshipKind, VisibilityKind,
};
use sysml_diagram::to_plantuml_state_view;

fn main() {
    let mut graph = ModelGraph::new();

    let root =
        graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Controller"));

    let modes = graph.add_owned_element(
        Element::new_with_kind(ElementKind::Package).with_name("Modes"),
        root,
        VisibilityKind::Public,
    );

    let idle = graph.add_owned_element(
        Element::new_with_kind(ElementKind::StateDefinition).with_name("Idle"),
        modes.clone(),
        VisibilityKind::Public,
    );

    let starting = graph.add_owned_element(
        Element::new_with_kind(ElementKind::StateDefinition).with_name("Starting"),
        modes.clone(),
        VisibilityKind::Public,
    );

    let running = graph.add_owned_element(
        Element::new_with_kind(ElementKind::StateDefinition).with_name("Running"),
        modes.clone(),
        VisibilityKind::Public,
    );

    let error = graph.add_owned_element(
        Element::new_with_kind(ElementKind::StateDefinition).with_name("Error"),
        modes,
        VisibilityKind::Public,
    );

    graph.add_relationship(Relationship::new(
        RelationshipKind::Transition,
        idle.clone(),
        starting.clone(),
    ));
    graph.add_relationship(Relationship::new(
        RelationshipKind::Transition,
        starting,
        running.clone(),
    ));
    graph.add_relationship(Relationship::new(
        RelationshipKind::Transition,
        running.clone(),
        idle.clone(),
    ));
    graph.add_relationship(Relationship::new(
        RelationshipKind::Transition,
        running,
        error.clone(),
    ));
    graph.add_relationship(Relationship::new(RelationshipKind::Transition, error, idle));

    let puml = to_plantuml_state_view(&graph);
    println!("{}", puml);
}
