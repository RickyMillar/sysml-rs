//! Transport-neutral helpers for ad-hoc diagram requests.

use std::collections::HashSet;

use sysml_core::ModelGraph;
use sysml_diagram::ViewType;

/// Convert a view type to its canonical standard-library definition name.
pub fn view_type_name(view_type: ViewType) -> &'static str {
    match view_type {
        ViewType::General => "GeneralView",
        ViewType::Interconnection => "InterconnectionView",
        ViewType::StateTransition => "StateTransitionView",
        ViewType::ActionFlow => "ActionFlowView",
        ViewType::Browser => "BrowserView",
        ViewType::Sequence => "SequenceView",
        ViewType::Grid => "GridView",
        ViewType::Geometry => "GeometryView",
    }
}

/// Parse a request view type, defaulting to `General` for an unknown value.
pub fn parse_view_type(value: &str) -> ViewType {
    ViewType::from_request_str(value).unwrap_or(ViewType::General)
}

/// Drop expanded ids that no longer resolve in the graph.
pub fn prune_expanded_ids(expanded: &mut HashSet<String>, graph: &ModelGraph) {
    expanded.retain(|id| {
        let element_id = sysml_id::ElementId::from_string(id);
        graph.get_element(&element_id).is_some()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_view_types() {
        assert_eq!(parse_view_type("general"), ViewType::General);
        assert_eq!(
            parse_view_type("StateTransitionView"),
            ViewType::StateTransition
        );
        assert_eq!(parse_view_type("unknown"), ViewType::General);
    }

    #[test]
    fn view_type_names_are_canonical() {
        assert_eq!(view_type_name(ViewType::General), "GeneralView");
        assert_eq!(
            view_type_name(ViewType::StateTransition),
            "StateTransitionView"
        );
    }
}
