//! Renderer-neutral text derived from SysML model elements.

use sysml_core::{Element, ElementKind, ModelGraph};

/// Resolve a display name without exposing a renderer placeholder for unnamed
/// elements.
pub(crate) fn element_display_name(element: &Element, graph: &ModelGraph) -> String {
    if let Some(name) = &element.name {
        return name.clone();
    }
    match element.kind {
        ElementKind::TransitionUsage => {
            let source = element
                .get_prop("source")
                .map(|v| v.to_string().trim_matches('"').to_owned());
            let target = element
                .get_prop("target")
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let (Some(source), Some(target)) = (source, target) {
                if !source.is_empty() && !target.is_empty() {
                    return format!("{source} \u{2192} {target}");
                }
            }
        }
        ElementKind::Comment | ElementKind::Documentation => {
            let body = element
                .get_prop("body")
                .or_else(|| element.get_prop("documentation"))
                .map(|v| v.to_string().trim_matches('"').to_owned());
            if let Some(body) = body {
                if let Some(first) = crate::ir::generators::container::wrap_doc_text(&body)
                    .into_iter()
                    .next()
                {
                    return first;
                }
            }
        }
        _ => {}
    }
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::Redefinition {
            if let Some(name) = child
                .get_prop("unresolved_redefinedFeature")
                .and_then(|v| v.as_str())
            {
                return format!(":>> {name}");
            }
        }
    }
    "unnamed".to_owned()
}

/// Render an element kind as its declaration-text stereotype.
pub(crate) fn stereotype_text(kind: &ElementKind) -> String {
    format!(
        "\u{00ab}{}\u{00bb}",
        crate::visual_kind::element_keyword(kind)
    )
}

/// Generate a structured tooltip for an element.
pub(crate) fn tooltip_text(element: &Element, graph: &ModelGraph) -> Option<String> {
    let keyword = crate::visual_kind::element_keyword(&element.kind);
    let name = element_display_name(element, graph);
    let mut lines = vec![format!("«{keyword}» {name}")];

    if let Some(type_name) = find_element_type(element, graph) {
        lines.push(format!("type: {type_name}"));
    }
    let hierarchy = collect_hierarchy(element, graph, 3);
    if !hierarchy.is_empty() {
        lines.push(format!("hierarchy: {}", hierarchy.join(" > ")));
    }
    Some(lines.join("\n"))
}

fn find_element_type(element: &Element, graph: &ModelGraph) -> Option<String> {
    if let Some(typing) = element.props.get("typing").and_then(|v| v.as_str()) {
        return Some(typing.to_owned());
    }
    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(target_id) = child.props.get("type").and_then(|v| v.as_ref()) {
                if let Some(target) = graph.get_element(target_id) {
                    if let Some(name) = &target.name {
                        return Some(name.clone());
                    }
                }
            }
            if let Some(name) = child.props.get("unresolved_type").and_then(|v| v.as_str()) {
                return Some(name.to_owned());
            }
        }
    }
    None
}

fn collect_hierarchy(element: &Element, graph: &ModelGraph, max_depth: usize) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current_id = element.id.clone();

    for _ in 0..max_depth {
        let mut found_general = None;
        for child in graph.children_of(&current_id) {
            if child.kind == ElementKind::Specialization
                || child.kind.is_subtype_of(ElementKind::Specialization)
            {
                if let Some(general_id) = child.props.get("general").and_then(|v| v.as_ref()) {
                    if let Some(general) = graph.get_element(general_id) {
                        if let Some(name) = &general.name {
                            found_general = Some((name.clone(), general_id.clone()));
                            break;
                        }
                    }
                }
                if let Some(name) = child
                    .props
                    .get("unresolved_general")
                    .and_then(|v| v.as_str())
                {
                    chain.push(name.to_owned());
                    return chain;
                }
            }
        }
        if let Some((name, general_id)) = found_general {
            chain.push(name);
            current_id = general_id;
        } else {
            break;
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_uses_declaration_keyword_and_name() {
        let graph = ModelGraph::new();
        let element = Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        assert_eq!(
            tooltip_text(&element, &graph),
            Some("«part def» Vehicle".to_owned())
        );
    }
}
