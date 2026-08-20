//! Shared helpers for SModel visualization pipeline tests.
#![allow(dead_code)]

use std::collections::HashSet;

use sysml_diagram::smodel::{self, SEdge, SGraph, SModelElement, SNode, ViewType};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

pub fn parse_sysml(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("test.sysml", source)];
    let result = parser.parse(&files);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "Parse errors: {:?}", errors);
    result.graph
}

pub fn generate(source: &str, view: ViewType, expand_all: bool) -> SGraph {
    let mut graph = parse_sysml(source);
    // Elaborate the graph to synthesize implicit relationships (satisfy, verify,
    // transitions, connectors, flows) — same as the service layer's diagram() method.
    sysml_core::elaborate::elaborate(&mut graph);
    let expanded: HashSet<String> = if expand_all {
        graph.elements.keys().map(|id| id.to_string()).collect()
    } else {
        HashSet::new()
    };
    let request = sysml_diagram::ViewRequest::new(view).with_expanded(expanded);
    smodel::to_smodel_with(&graph, &request)
}

pub fn count_by_type(children: &[SModelElement], type_prefix: &str) -> usize {
    let mut count = 0;
    for child in children {
        match child {
            SModelElement::Node(n) => {
                if n.type_.starts_with(type_prefix) {
                    count += 1;
                }
                count += count_by_type(&n.children, type_prefix);
            }
            SModelElement::Edge(e) => {
                if e.type_.starts_with(type_prefix) {
                    count += 1;
                }
            }
            SModelElement::Compartment(c) => {
                count += count_by_type(&c.children, type_prefix);
            }
            _ => {}
        }
    }
    count
}

pub fn count_edges(children: &[SModelElement]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SModelElement::Edge(_)))
        .count()
}

pub fn count_nodes(children: &[SModelElement]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SModelElement::Node(_)))
        .count()
}

pub fn find_node_by_type<'a>(children: &'a [SModelElement], type_: &str) -> Vec<&'a SNode> {
    let mut result = Vec::new();
    for child in children {
        match child {
            SModelElement::Node(n) => {
                if n.type_ == type_ {
                    result.push(n);
                }
                for inner in find_node_by_type(&n.children, type_) {
                    result.push(inner);
                }
            }
            SModelElement::Compartment(c) => {
                for inner in find_node_by_type(&c.children, type_) {
                    result.push(inner);
                }
            }
            _ => {}
        }
    }
    result
}

pub fn has_edge_type(children: &[SModelElement], type_: &str) -> bool {
    children
        .iter()
        .any(|c| matches!(c, SModelElement::Edge(e) if e.type_ == type_))
}

pub fn has_css_class_on_node(children: &[SModelElement], node_type: &str, class: &str) -> bool {
    for child in children {
        if let SModelElement::Node(n) = child {
            if n.type_ == node_type && n.css_classes.iter().any(|c| c == class) {
                return true;
            }
        }
    }
    false
}

pub fn collect_all_types(children: &[SModelElement]) -> HashSet<String> {
    let mut types = HashSet::new();
    for child in children {
        match child {
            SModelElement::Graph(_) => {
                types.insert("graph".to_string());
            }
            SModelElement::Node(n) => {
                types.insert(n.type_.clone());
            }
            SModelElement::Edge(e) => {
                types.insert(e.type_.clone());
            }
            SModelElement::Compartment(c) => {
                types.insert(c.type_.clone());
            }
            SModelElement::Port(p) => {
                types.insert(p.type_.clone());
            }
            SModelElement::Label(l) => {
                types.insert(l.type_.clone());
            }
            SModelElement::Button(b) => {
                types.insert(b.type_.clone());
            }
        }
    }
    types
}

pub fn edges(children: &[SModelElement]) -> Vec<&SEdge> {
    children
        .iter()
        .filter_map(|c| {
            if let SModelElement::Edge(e) = c {
                Some(e)
            } else {
                None
            }
        })
        .collect()
}

pub fn ports_in(children: &[SModelElement]) -> usize {
    let mut count = 0;
    for child in children {
        match child {
            SModelElement::Port(_) => count += 1,
            SModelElement::Node(n) => count += ports_in(&n.children),
            SModelElement::Compartment(c) => count += ports_in(&c.children),
            _ => {}
        }
    }
    count
}
