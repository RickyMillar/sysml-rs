//! Connector endpoint extraction (binding, connection, succession, allocation,
//! interface, flow). Most extractors populate the `source` / `target`
//! properties on an already-created element from the various endpoint
//! sub-forms in the CST.

use super::{AstBuilder, ModelGraphResult};
use sysml_core::{Element, ElementId, ModelGraph};
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    /// Extract assignment action properties: targetFeature and valueExpression.
    ///
    /// Grammar: `seq("assign", $._expression, choice("=", ":="), $._expression, ";")`
    /// The two unnamed `_expression` children are: first = target, second = value.
    pub(super) fn extract_assignment_props(&self, node: &Node<'a>, id: &ElementId, graph: &mut ModelGraph) {
        let mut expressions = Vec::new();
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                if child.is_named() {
                    let text = self.node_text(&child).trim().to_owned();
                    if !text.is_empty() {
                        expressions.push(text);
                    }
                }
            }
        }

        if let Some(elem) = graph.get_element_mut(id) {
            if let Some(target) = expressions.first() {
                elem.set_prop("targetFeature", target.clone());
            }
            if let Some(value) = expressions.get(1) {
                elem.set_prop("valueExpression", value.clone());
            }
        }
    }

    /// Extract source/target endpoint properties from a child node (flow_ends or connection_ends).
    ///
    /// Binary form uses named fields (`source`, `target`).
    /// N-ary form `(a, b, ...)` has unnamed feature_chain children — we treat the
    /// first two as source and target for 2-end connectors.
    #[allow(clippy::indexing_slicing)]
    pub(super) fn extract_endpoint_props(&self, node: &Node<'a>, element: &mut Element, child_kind: &str) {
        if let Some(ends_node) = self.find_child_node(node, child_kind) {
            // Try named fields first (binary form)
            let has_source = if let Some(source) = self.find_field_text(&ends_node, "source") {
                let source = source.trim();
                if !source.is_empty() {
                    element.set_prop("source", source.to_owned());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if let Some(target) = self.find_field_text(&ends_node, "target") {
                let target = target.trim();
                if !target.is_empty() {
                    element.set_prop("target", target.to_owned());
                }
            }
            // Fallback for n-ary form: collect unnamed feature_chain children
            if !has_source {
                let mut chains: Vec<String> = Vec::new();
                let mut cursor = ends_node.walk();
                for child in ends_node.children(&mut cursor) {
                    if child.kind() == "feature_chain" {
                        let text = self.node_text(&child).trim().to_owned();
                        if !text.is_empty() {
                            chains.push(text);
                        }
                    }
                }
                if chains.len() >= 2 {
                    element.set_prop("source", chains[0].clone());
                    element.set_prop("target", chains[1].clone());
                } else if chains.len() == 1 {
                    element.set_prop("source", chains[0].clone());
                }
            }
        }
    }

    /// Try to extract endpoint props from a child node. Returns true if the child was found.
    pub(super) fn try_extract_endpoint_props(
        &self,
        node: &Node<'a>,
        element: &mut Element,
        child_kind: &str,
    ) -> bool {
        if self.find_child_node(node, child_kind).is_some() {
            self.extract_endpoint_props(node, element, child_kind);
            true
        } else {
            false
        }
    }

    /// Extract source/target from direct field names on the node itself.
    pub(super) fn extract_direct_field_props(
        &self,
        node: &Node<'a>,
        element: &mut Element,
        source_field: &str,
        target_field: &str,
    ) {
        if let Some(source) = self.find_field_text(node, source_field) {
            let source = source.trim();
            if !source.is_empty() {
                element.set_prop("source", source.to_owned());
            }
        }
        if let Some(target) = self.find_field_text(node, target_field) {
            let target = target.trim();
            if !target.is_empty() {
                element.set_prop("target", target.to_owned());
            }
        }
    }

    /// Extract allocation endpoints: grammar uses `from`/`to` field names, mapped to source/target.
    pub(super) fn extract_allocation_endpoint_props(&self, node: &Node<'a>, element: &mut Element) {
        if let Some(ends_node) = self.find_child_node(node, "allocation_ends") {
            if let Some(from) = self.find_field_text(&ends_node, "from") {
                let from = from.trim();
                if !from.is_empty() {
                    element.set_prop("source", from.to_owned());
                }
            }
            if let Some(to) = self.find_field_text(&ends_node, "to") {
                let to = to.trim();
                if !to.is_empty() {
                    element.set_prop("target", to.to_owned());
                }
            }
        }
    }

    /// Extract succession endpoints: `source`/`target` from the node directly,
    /// with fallback from `successor` field to target.
    pub(super) fn extract_succession_endpoint_props(&self, node: &Node<'a>, element: &mut Element) {
        // succession_decl uses direct source/target fields
        if let Some(source) = self.find_field_text(node, "source") {
            let source = source.trim();
            if !source.is_empty() {
                element.set_prop("source", source.to_owned());
            }
        }
        if let Some(target) = self.find_field_text(node, "target") {
            let target = target.trim();
            if !target.is_empty() {
                element.set_prop("target", target.to_owned());
            }
        }
        // succession_usage uses `successor` field mapped to target
        if element.get_prop("target").is_none() {
            if let Some(successor) = self.find_field_text(node, "successor") {
                let successor = successor.trim();
                if !successor.is_empty() {
                    element.set_prop("target", successor.to_owned());
                }
            }
        }
    }

    /// Infer the source element name for a `then X;` (succession_usage) that
    /// has no explicit source in the grammar.
    ///
    /// Walks back through preceding CST siblings to find the logical predecessor:
    /// - If preceded by a `control_flow_node`, uses that node as the source.
    ///   For fork/decide, consecutive `then` statements all fan-out from the same
    ///   control node (parallel paths).
    /// - If preceded by another `succession_usage` whose chain originates from a
    ///   non-fork node, chains sequentially (source = previous `then`'s target).
    /// - If preceded by a regular element (action_usage, etc.), uses its name.
    pub(super) fn infer_succession_source(
        &self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        result: &ModelGraphResult,
    ) -> Option<String> {
        let prev = node.prev_named_sibling()?;

        match prev.kind() {
            "control_flow_node" => {
                // Direct predecessor is a control node — use its name
                self.find_element_name_by_span(prev.start_byte(), parent_id, result)
            }
            "succession_usage" => {
                // Another `then Y;` before us. Check if the chain originates from
                // a fork or decide (fan-out) vs a regular element (sequential chain).
                let origin = self.find_succession_chain_origin(&prev);
                match origin {
                    Some(o) if o.kind() == "control_flow_node" => {
                        let keyword = o
                            .child_by_field_name("keyword")
                            .map(|k| k.kind().to_owned());
                        if matches!(keyword.as_deref(), Some("fork") | Some("decide")) {
                            // Fan-out: all consecutive `then`s share the same source
                            self.find_element_name_by_span(o.start_byte(), parent_id, result)
                        } else {
                            // join/merge: chain sequentially — source is prev `then`'s target
                            self.extract_succession_target_name(&prev)
                        }
                    }
                    _ => {
                        // Non-control-node origin: chain sequentially
                        self.extract_succession_target_name(&prev)
                    }
                }
            }
            // Skip feature_declaration nodes (grammar split artifacts)
            "feature_declaration" => {
                let deeper = prev.prev_named_sibling()?;
                self.find_element_name_by_span(deeper.start_byte(), parent_id, result)
            }
            _ => {
                // Regular element (action_usage, state_usage, etc.) — use its name
                self.find_element_name_by_span(prev.start_byte(), parent_id, result)
            }
        }
    }

    /// Walk back through consecutive succession_usage siblings to find the
    /// originating non-succession element (the element before the `then` chain).
    pub(super) fn find_succession_chain_origin<'b>(&self, node: &Node<'b>) -> Option<Node<'b>> {
        let mut current = node.prev_named_sibling()?;
        while current.kind() == "succession_usage" {
            current = current.prev_named_sibling()?;
        }
        // Skip feature_declaration artifacts
        if current.kind() == "feature_declaration" {
            current = current.prev_named_sibling()?;
        }
        Some(current)
    }

    /// Look up the name of an element whose span starts at the given byte offset
    /// and shares the specified owner.
    pub(super) fn find_element_name_by_span(
        &self,
        start_byte: usize,
        owner: &Option<ElementId>,
        result: &ModelGraphResult,
    ) -> Option<String> {
        result
            .graph
            .elements
            .values()
            .find(|e| {
                e.owner.as_ref() == owner.as_ref()
                    && e.spans.iter().any(|s| s.start == start_byte)
            })
            .and_then(|e| e.name.clone())
    }

    /// Extract the target name from a succession_usage CST node (the `successor`
    /// field, or feature_chain/identifier fallback).
    pub(super) fn extract_succession_target_name(&self, node: &Node<'a>) -> Option<String> {
        self.find_field_text(node, "successor")
            .or_else(|| self.find_child_text(node, "feature_chain"))
            .or_else(|| self.find_child_text(node, "identifier"))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }
}
