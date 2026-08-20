//! Package processing (top-level + nested) and library-package recognition.

use super::{AstBuilder, ModelGraphResult};
use sysml_core::{CanonicalKey, ElementId, ElementKind};
use sysml_parser_trait::extraction::PackageExtraction;
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    /// Process a package declaration.
    pub(super) fn process_package(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        is_library: bool,
    ) -> Option<(ElementId, CanonicalKey)> {
        let mut extraction = self.extract_package(node, is_library);
        let span = self.node_span(node);

        let kind = if is_library {
            ElementKind::LibraryPackage
        } else {
            ElementKind::Package
        };

        let (resolved_parent_key, child_key, sibling_index) =
            self.prep_canonical_key(parent_key, parent_id, &kind, extraction.name.as_deref());
        let owner_key_for_membership = resolved_parent_key.clone();
        extraction.parent_key = Some(resolved_parent_key);
        extraction.sibling_index = sibling_index;

        let mut element = extraction.build_element(kind, Some(span));
        if let Some(name_node) = self.find_name_node(node) {
            element.name_span = Some(self.node_span(&name_node));
        }
        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            Some(&owner_key_for_membership),
            &child_key,
            &mut result.graph,
        );

        Some((id, child_key))
    }

    /// Process a definition (PartDefinition, ActionDefinition, etc.).
    #[allow(clippy::needless_pass_by_value)]

    pub(super) fn extract_package(&self, node: &Node<'a>, is_library: bool) -> PackageExtraction {
        let is_standard = is_library && self.node_text(node).starts_with("standard");
        PackageExtraction {
            name: self.find_name_field(node),
            is_standard,
            ..Default::default()
        }
    }

}
