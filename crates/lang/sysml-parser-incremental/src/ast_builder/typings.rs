//! Type-reference and multiplicity extraction from usage / definition nodes.
//!
//! These methods produce strings (qualified names) that the resolver will
//! later turn into ElementId refs. The `create_keyed_rels_from_type_refs`
//! helper threads ADR-009 canonical keys + per-role sibling counters into
//! each `*_with_key` relationship builder.

use super::{AstBuilder, ModelGraphResult, RelKind};
use sysml_core::{CanonicalKey, ElementId, ElementKind, ModelGraph};
use sysml_parser_trait::extraction::UsageExtraction;
use sysml_parser_trait::relationship_builder::{
    create_cross_subsetting_with_key, create_redefinition_with_key,
    create_standalone_subsetting_with_key, create_subsetting_with_key,
};
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    /// Helper: create relationship elements for each type_ref child of a parent
    /// node, threading the canonical key + per-role sibling counter through to
    /// each `*_with_key` builder. Returns the next sibling index for the role.
    pub(super) fn create_keyed_rels_from_type_refs(
        &self,
        parent: &Node<'a>,
        owner_id: &ElementId,
        owner_key: &CanonicalKey,
        role: &str,
        start_index: usize,
        graph: &mut ModelGraph,
        kind: RelKind,
    ) -> usize {
        let mut idx = start_index;
        let child_count = parent.child_count();
        for i in 0..child_count {
            if let Some(child) = parent.child(i) {
                // Accept both wrapped (`type_ref → feature_chain`) and direct
                // (`feature_chain` / `qualified_name`) children. The tree-sitter
                // grammar uses bare `feature_chain` under `redefinition`,
                // `crosses_clause`, and `references_keyword` nodes; only
                // `supertype_list` wraps in `type_ref`. Without this branch the
                // dispatch path for `:>>` and `redefines` never reaches
                // `create_redefinition_with_key` (TS-1.2 / gap #2).
                let qname_opt = match child.kind() {
                    "type_ref" => self.extract_type_ref_from_node(&child),
                    "feature_chain" | "qualified_name" => {
                        Some(self.node_text(&child).to_owned())
                    }
                    _ => None,
                };
                if let Some(qname) = qname_opt {
                    let span = Some(self.node_span(&child));
                    match kind {
                        RelKind::Redefinition => {
                            create_redefinition_with_key(
                                graph,
                                owner_id.clone(),
                                qname,
                                span,
                                owner_key,
                                role,
                                idx,
                            );
                        }
                        RelKind::Subsetting => {
                            create_subsetting_with_key(
                                graph,
                                owner_id.clone(),
                                qname,
                                span,
                                owner_key,
                                role,
                                idx,
                            );
                        }
                        RelKind::CrossSubsetting => {
                            create_cross_subsetting_with_key(
                                graph,
                                owner_id.clone(),
                                qname,
                                span,
                                owner_key,
                                role,
                                idx,
                            );
                        }
                    }
                    idx += 1;
                }
            }
        }
        idx
    }

    // ========== Extraction helpers ==========

    /// Extract package information from a node.
    ///
    /// For library packages, detects the `standard` keyword by checking
    /// the node's text prefix. Per SysML spec, `LibraryPackage` is the kind
    /// and `isStandard` is set by the `standard` keyword.

    /// Extract typing declarations from a usage node, separating conjugated (`~Type`) from regular.
    ///
    /// Returns `(regular_typings, conjugated_typings)` where conjugated typings
    /// have the `~` prefix stripped (just the port def name).
    pub(super) fn extract_typings_with_conjugation(&self, node: &Node<'a>) -> (Vec<String>, Vec<String>) {
        let mut typings = Vec::new();
        let mut conjugated = Vec::new();

        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                if child.kind() == "typing" {
                    // Check each type_ref within the typing node
                    let type_count = child.child_count();
                    for j in 0..type_count {
                        if let Some(type_child) = child.child(j) {
                            if type_child.kind() == "type_ref" {
                                let text = self.node_text(&type_child).trim();
                                if text.starts_with('~') {
                                    // Conjugated port typing: strip `~` and extract the name
                                    let name = text.trim_start_matches('~').trim();
                                    if !name.is_empty() {
                                        conjugated.push(name.to_owned());
                                    }
                                } else if let Some(qname) = self.extract_type_ref_from(&type_child)
                                {
                                    typings.push(qname);
                                }
                            }
                        }
                    }
                }
            }
        }

        (typings, conjugated)
    }

    /// Extract a type reference name from a type_ref node.
    pub(super) fn extract_type_ref_from(&self, type_ref: &Node<'a>) -> Option<String> {
        if let Some(qname) = self.find_child_node(type_ref, "qualified_name") {
            return Some(self.node_text(&qname).to_owned());
        }
        if let Some(id) = self.find_child_node(type_ref, "identifier") {
            return Some(self.node_text(&id).to_owned());
        }
        None
    }

    /// Process a standalone `subset X subsets Y;` declaration (G08e) into a
    /// `Subsetting` relationship element owned by the enclosing namespace.
    ///
    /// Per KerML.xtext:679-688 this is a `NonFeatureElement` (a namespace
    /// member), NOT a usage. Both endpoints are stored unresolved and wired up
    /// by name in pass-2 resolution (`unresolved_subsettingFeature` /
    /// `unresolved_subsettedFeature`).
    pub(super) fn process_subsetting_decl(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let subsetting = node.child_by_field_name("subsetting")?;
        let subsetted = node.child_by_field_name("subsetted")?;
        let subsetting_qname = self.node_text(&subsetting).to_owned();
        let subsetted_qname = self.node_text(&subsetted).to_owned();
        let parent_key_resolved = self.resolve_parent_key(parent_key);
        let sibling_index = self.next_sibling_index(parent_id, &ElementKind::Subsetting);
        let span = Some(self.node_span(node));
        let id = create_standalone_subsetting_with_key(
            &mut result.graph,
            parent_id.clone(),
            subsetting_qname,
            subsetted_qname,
            span,
            &parent_key_resolved,
            "subsetting",
            sibling_index,
        );
        let child_key =
            CanonicalKey::for_anonymous(&parent_key_resolved, "Subsetting:subsetting", sibling_index);
        Some((id, child_key))
    }

    /// Extract subsetting declarations from a usage node.
    pub(super) fn extract_subsettings(&self, node: &Node<'a>) -> Vec<String> {
        let mut subsettings = Vec::new();

        // Subsettings come from supertype_list with :> syntax
        if let Some(supertype_list) = self.find_child_node(node, "supertype_list") {
            // Look for subsetting indicators (:>)
            let text = self.node_text(&supertype_list);
            if text.contains(":>") && !text.contains(":>>") {
                // Extract qualified names after :>
                let mut cursor = supertype_list.walk();
                for child in supertype_list.children(&mut cursor) {
                    if child.kind() == "qualified_name" || child.kind() == "identifier" {
                        subsettings.push(self.node_text(&child).to_owned());
                    }
                }
            }
        }

        subsettings
    }

    /// Extract redefinition declarations from a usage node.
    pub(super) fn extract_redefinitions(&self, node: &Node<'a>) -> Vec<String> {
        let mut redefinitions = Vec::new();

        // Look for redefinition child nodes
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "redefinition" {
                if let Some(qname) = self.find_qualified_name(&child) {
                    redefinitions.push(qname);
                }
            }
        }

        // Also check supertype_list for :>> syntax
        if let Some(supertype_list) = self.find_child_node(node, "supertype_list") {
            let text = self.node_text(&supertype_list);
            if text.contains(":>>") {
                let mut cursor = supertype_list.walk();
                for child in supertype_list.children(&mut cursor) {
                    if child.kind() == "qualified_name" || child.kind() == "identifier" {
                        redefinitions.push(self.node_text(&child).to_owned());
                    }
                }
            }
        }

        redefinitions
    }

    /// Extract supertype list (for definitions - specialization).
    pub(super) fn extract_supertype_list(&self, node: &Node<'a>) -> Vec<String> {
        let mut supertypes = Vec::new();

        if let Some(supertype_list) = self.find_child_node(node, "supertype_list") {
            // supertype_list contains type_ref nodes
            let child_count = supertype_list.child_count();
            for i in 0..child_count {
                if let Some(child) = supertype_list.child(i) {
                    if child.kind() == "type_ref" {
                        if let Some(qname) = self.extract_type_ref_from_node(&child) {
                            supertypes.push(qname);
                        }
                    }
                }
            }
        }

        supertypes
    }

    /// Extract the type name from a type_ref node directly.
    pub(super) fn extract_type_ref_from_node(&self, type_ref: &Node<'a>) -> Option<String> {
        // type_ref may contain qualified_name, feature_chain, or identifier
        if let Some(qname) = self.find_child_node(type_ref, "qualified_name") {
            return Some(self.node_text(&qname).to_owned());
        }
        if let Some(chain) = self.find_child_node(type_ref, "feature_chain") {
            return Some(self.node_text(&chain).to_owned());
        }
        if let Some(id) = self.find_child_node(type_ref, "identifier") {
            return Some(self.node_text(&id).to_owned());
        }
        None
    }

    /// Extract multiplicity from a usage node.
    ///
    /// Uses `parse_multiplicity_full` to handle both numeric and symbolic bounds.
    /// Symbolic bounds (e.g., `[min..max]`) are stored in the extraction's
    /// `multiplicity_lower_text` and `multiplicity_upper_text` fields.
    pub(super) fn extract_multiplicity_full(&self, node: &Node<'a>, extraction: &mut UsageExtraction) {
        // Multiplicity can appear as:
        // 1. Direct child of usage node (standalone: `part p [0..*]`)
        // 2. Inside a typing node (combined: `part p : T [1]`)
        let mult_node = self.find_child_node(node, "multiplicity").or_else(|| {
            let typing_node = self.find_child_node(node, "typing")?;
            self.find_child_node(&typing_node, "multiplicity")
        });

        let Some(mult_node) = mult_node else { return };
        let text = self.node_text(&mult_node);

        if let Some(result) = sysml_parser_trait::extraction::parse_multiplicity_full(text) {
            extraction.multiplicity = result.numeric;
            extraction.multiplicity_lower_text = result.lower_text;
            extraction.multiplicity_upper_text = result.upper_text;
        }
    }

    /// Extract multiplicity modifiers (ordered/nonunique) from a usage node.
    pub(super) fn extract_multiplicity_modifiers(&self, node: &Node<'a>, extraction: &mut UsageExtraction) {
        if let Some(mods_node) = self.find_child_node(node, "multiplicity_modifiers") {
            let mut cursor = mods_node.walk();
            for child in mods_node.children(&mut cursor) {
                match self.node_text(&child) {
                    "ordered" => extraction.is_ordered = true,
                    "nonunique" => extraction.is_nonunique = true,
                    _ => {}
                }
            }
        }
    }
}
