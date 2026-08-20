use std::collections::BTreeSet;

use sysml_core::{ElementKind, ModelGraph, RelationshipKind};

use crate::visual_kind::is_state_kind;

/// Export a ModelGraph to PlantUML format.
pub fn to_plantuml(graph: &ModelGraph) -> String {
    let mut output = String::new();
    output.push_str("@startuml\n");
    output.push_str("skinparam linetype ortho\n");
    output.push('\n');

    // Export packages first
    for element in graph.elements_by_kind(&ElementKind::Package) {
        let name = element.name.as_deref().unwrap_or("unnamed");
        output.push_str(&format!("package \"{}\" {{\n", name));

        // Add children
        for child in graph.children_of(&element.id) {
            let child_name = child.name.as_deref().unwrap_or("unnamed");
            let stereotype = plantuml_stereotype(&child.kind);
            output.push_str(&format!(
                "  {} \"{}\" as {}\n",
                stereotype, child_name, child.id
            ));
        }

        output.push_str("}\n\n");
    }

    // Export top-level elements not in packages
    for element in graph.elements.values() {
        if element.owner.is_none() && !matches!(element.kind, ElementKind::Package) {
            let name = element.name.as_deref().unwrap_or("unnamed");
            let stereotype = plantuml_stereotype(&element.kind);
            output.push_str(&format!("{} \"{}\" as {}\n", stereotype, name, element.id));
        }
    }

    output.push('\n');

    // Export relationships
    for rel in graph.relationships.values() {
        let arrow = plantuml_arrow(&rel.kind);
        let label = rel.kind.as_str();
        output.push_str(&format!(
            "{} {} {} : {}\n",
            rel.source, arrow, rel.target, label
        ));
    }

    output.push_str("@enduml\n");
    output
}

/// Export a ModelGraph to a PlantUML state diagram.
pub fn to_plantuml_state_view(graph: &ModelGraph) -> String {
    let mut output = String::new();
    output.push_str("@startuml\n");
    output.push_str("hide empty description\n");
    output.push('\n');

    let mut state_ids = BTreeSet::new();
    for element in graph.elements.values() {
        if is_state_kind(&element.kind) {
            state_ids.insert(element.id.clone());
        }
    }

    for id in &state_ids {
        if let Some(element) = graph.get_element(id) {
            let name = element.name.as_deref().unwrap_or("unnamed");
            output.push_str(&format!("state \"{}\" as {}\n", name, id));
        }
    }

    output.push('\n');

    for rel in graph.relationships.values() {
        if rel.kind != RelationshipKind::Transition {
            continue;
        }
        if state_ids.contains(&rel.source) && state_ids.contains(&rel.target) {
            output.push_str(&format!("{} --> {}\n", rel.source, rel.target));
        }
    }

    output.push_str("@enduml\n");
    output
}

fn plantuml_stereotype(kind: &ElementKind) -> String {
    use crate::visual_kind::VisualKind;
    let gk = VisualKind::from_element_kind(kind);
    match gk {
        VisualKind::Package => "package".to_owned(),
        VisualKind::State => "state".to_owned(),
        VisualKind::Comment => "note".to_owned(),
        _ => {
            let css = gk.css_class();
            if css == "generic" {
                "class".to_owned()
            } else {
                format!("class <<{}>>", css)
            }
        }
    }
}

fn plantuml_arrow(kind: &RelationshipKind) -> &'static str {
    kind.plantuml_arrow()
}
