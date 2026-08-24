//! PlantUML sequence + activity diagram export.

use sysml_runtime::actions::{ActionGraphIR, ActionNodeIR};

/// A flow event for sequence diagram generation.
#[derive(Debug, Clone)]
pub struct SequenceEvent {
    pub source: String,
    pub target: String,
    pub label: String,
}

/// Convert an `ActionGraphIR` to a PlantUML activity diagram.
///
/// Renders one labelled node per `ActionNodeIR` and an arrow per
/// `ActionEdgeIR`. Decision/merge/fork/join are stamped with their
/// PlantUML stereotype; guarded edges carry their guard label.
///
/// Used by `sysml.action.visualize` so the service-layer command can
/// return the design-target `{plantuml}` dual shape (Bucket B2).
pub fn to_plantuml_activity(ir: &ActionGraphIR) -> String {
    let mut out = String::new();
    out.push_str("@startuml\n");
    out.push_str(&format!("title {}\n", escape_plantuml(&ir.name)));
    out.push('\n');

    for node in &ir.nodes {
        let id = node.id();
        let label = node_label(node);
        let stereotype = node_stereotype(node);
        out.push_str(&format!(
            "state \"{}\" as {} {}\n",
            escape_plantuml(&label),
            sanitize_id(id),
            stereotype
        ));
    }
    out.push('\n');

    for edge in &ir.edges {
        let from = sanitize_id(&edge.from);
        let to = sanitize_id(&edge.to);
        match &edge.guard {
            Some(_) => out.push_str(&format!("{} --> {} : [guard]\n", from, to)),
            None => out.push_str(&format!("{} --> {}\n", from, to)),
        }
    }

    out.push_str("@enduml\n");
    out
}

fn node_label(node: &ActionNodeIR) -> String {
    match node {
        ActionNodeIR::Initial { .. } => "(start)".to_owned(),
        ActionNodeIR::Final { .. } => "(end)".to_owned(),
        ActionNodeIR::Perform { action_ref, .. } => action_ref.clone(),
        ActionNodeIR::Send { target, .. } => format!("send -> {}", target),
        ActionNodeIR::Accept { source, .. } => {
            source.clone().unwrap_or_else(|| "accept".to_owned())
        }
        ActionNodeIR::Assign { target, .. } => format!("assign {}", target),
        ActionNodeIR::If { .. } => "if".to_owned(),
        ActionNodeIR::WhileLoop { .. } => "while".to_owned(),
        ActionNodeIR::ForLoop { variable, .. } => format!("for {}", variable),
        ActionNodeIR::Terminate { .. } => "terminate".to_owned(),
        ActionNodeIR::Decision { .. } => "decision".to_owned(),
        ActionNodeIR::Merge { .. } => "merge".to_owned(),
        ActionNodeIR::Fork { .. } => "fork".to_owned(),
        ActionNodeIR::Join { .. } => "join".to_owned(),
        ActionNodeIR::StreamSource { target, .. } => format!("stream -> {}", target),
    }
}

fn node_stereotype(node: &ActionNodeIR) -> &'static str {
    match node {
        ActionNodeIR::Initial { .. } => "<<start>>",
        ActionNodeIR::Final { .. } => "<<end>>",
        ActionNodeIR::Decision { .. } => "<<choice>>",
        ActionNodeIR::Merge { .. } => "<<merge>>",
        ActionNodeIR::Fork { .. } => "<<fork>>",
        ActionNodeIR::Join { .. } => "<<join>>",
        _ => "",
    }
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Convert a sequence of flow events to a PlantUML sequence diagram.
pub fn to_plantuml_sequence(participants: &[String], events: &[SequenceEvent]) -> String {
    let mut out = String::new();
    out.push_str("@startuml\n");

    // Declare participants
    for p in participants {
        out.push_str(&format!("participant \"{}\"\n", escape_plantuml(p)));
    }
    out.push('\n');

    // Events as arrows
    for event in events {
        out.push_str(&format!(
            "\"{}\" -> \"{}\": {}\n",
            escape_plantuml(&event.source),
            escape_plantuml(&event.target),
            escape_plantuml(&event.label)
        ));
    }

    out.push_str("@enduml\n");
    out
}

/// Escape special characters for PlantUML format.
fn escape_plantuml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(':', "\\:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_sequence() {
        let participants = vec![];
        let events = vec![];
        let puml = to_plantuml_sequence(&participants, &events);

        assert!(puml.starts_with("@startuml"));
        assert!(puml.ends_with("@enduml\n"));
    }

    #[test]
    fn test_simple_sequence() {
        let participants = vec!["Alice".to_string(), "Bob".to_string()];
        let events = vec![SequenceEvent {
            source: "Alice".to_string(),
            target: "Bob".to_string(),
            label: "Hello".to_string(),
        }];
        let puml = to_plantuml_sequence(&participants, &events);

        assert!(puml.contains("participant \"Alice\""));
        assert!(puml.contains("participant \"Bob\""));
        assert!(puml.contains("\"Alice\" -> \"Bob\": Hello"));
    }

    #[test]
    fn test_multiple_events() {
        let participants = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let events = vec![
            SequenceEvent {
                source: "A".to_string(),
                target: "B".to_string(),
                label: "msg1".to_string(),
            },
            SequenceEvent {
                source: "B".to_string(),
                target: "C".to_string(),
                label: "msg2".to_string(),
            },
            SequenceEvent {
                source: "C".to_string(),
                target: "A".to_string(),
                label: "msg3".to_string(),
            },
        ];
        let puml = to_plantuml_sequence(&participants, &events);

        assert!(puml.contains("\"A\" -> \"B\": msg1"));
        assert!(puml.contains("\"B\" -> \"C\": msg2"));
        assert!(puml.contains("\"C\" -> \"A\": msg3"));
    }

    #[test]
    fn test_activity_initial_final_only() {
        let ir = ActionGraphIR::new("a1", "MyAction");
        let puml = to_plantuml_activity(&ir);
        assert!(puml.starts_with("@startuml"));
        assert!(puml.ends_with("@enduml\n"));
        assert!(puml.contains("title MyAction"));
        assert!(puml.contains("(start)"));
        assert!(puml.contains("(end)"));
        assert!(puml.contains("<<start>>"));
        assert!(puml.contains("<<end>>"));
    }

    #[test]
    fn test_activity_edges_render_as_arrows() {
        let mut ir = ActionGraphIR::new("a2", "EdgeAction");
        ir.add_edge("a2_initial", "a2_final");
        let puml = to_plantuml_activity(&ir);
        assert!(puml.contains("a2_initial --> a2_final"));
    }

    #[test]
    fn test_plantuml_escape() {
        assert_eq!(escape_plantuml("simple"), "simple");
        assert_eq!(escape_plantuml("with\"quote"), "with\\\"quote");
        assert_eq!(escape_plantuml("with\\backslash"), "with\\\\backslash");
        assert_eq!(escape_plantuml("with\newline"), "with\\newline");
        assert_eq!(escape_plantuml("with[bracket]"), "with\\[bracket\\]");
        assert_eq!(escape_plantuml("with:colon"), "with\\:colon");
    }
}
