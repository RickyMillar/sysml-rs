//! Import / expose / alias / comment processing.
//!
//! These declarations either contribute a directly-built element (with a
//! reparse-stable canonical key derived from the imported qname / target
//! name / explicit name) or, in the case of comments, contribute a
//! Comment / Documentation element whose body is the doc-string text.

use super::AstBuilder;
use sysml_core::{CanonicalKey, ElementId, ElementKind, Value};
use sysml_parser_trait::relationship_builder::create_annotation_with_key;
use tree_sitter::Node;

use super::ModelGraphResult;

impl<'a> AstBuilder<'a> {
    /// Process an import declaration.
    pub(super) fn process_import(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        // Extract the imported namespace/member qualified name. The grammar
        // exposes the full import path as `target` so single-member imports
        // like `import Lib::Engine;` don't collapse to just the first
        // identifier (`Lib`).
        let imported_qname = self
            .find_field_text(node, "target")
            .or_else(|| self.find_qualified_name(node))?;

        // Clamp span to the first line of the import statement so that
        // diagnostics point at the import line itself, not the enclosing block.
        let span = self.first_line_span(node);

        // Determine if this is a namespace import or member import
        let is_recursive = self.node_text(node).contains("::*");

        // The `visibility_indicator` child carries either a real visibility
        // keyword (`public`/`private`/`protected`) or — from the old grammar —
        // the `expose` marker. Read it once and split the two meanings.
        let visibility_text = self
            .find_child_node(node, "visibility_indicator")
            .map(|vi| self.node_text(&vi).trim().to_owned());

        // Backward compat: detect `expose import X` from old grammar
        // (new grammar uses expose_decl instead, but old parser.c still
        // produces import_decl with expose visibility_indicator)
        let is_expose = visibility_text.as_deref() == Some("expose");

        let kind = match (is_recursive, is_expose) {
            (true, true) => ElementKind::NamespaceExpose,
            (false, true) => ElementKind::MembershipExpose,
            (true, false) => ElementKind::NamespaceImport,
            (false, false) => ElementKind::MembershipImport,
        };

        // Imports are relationship elements, not named declarations. Keep them
        // anonymous in the canonical key so duplicate imports of the same target
        // in one scope remain distinct elements (and IM003 can diagnose them).
        // The imported qualified name is stored below as relationship data.
        let (child_key, mut element) = self.mint_direct_element(parent_key, parent_id, kind, None);
        element.set_prop("unresolved_importedNamespace", imported_qname.clone());
        // Keep import properties aligned with resolver expectations.
        element.set_prop("importedReference", imported_qname);
        element.set_prop("isNamespace", Value::Bool(is_recursive));
        element.set_prop("isRecursive", Value::Bool(is_recursive));
        // An Import's own visibility gates re-export of its imported members
        // (KerML: `Import.visibility`, default `private`). `public import`
        // re-exports; a plain/`private`/`protected` import does not.
        let visibility = match visibility_text.as_deref() {
            Some("public") => "public",
            Some("protected") => "protected",
            _ => "private",
        };
        // VisibilityKind is a closed metamodel enum — stamp Value::Enum to
        // match membership.rs's MembershipBuilder (the canonical mint site).
        element.set_prop("visibility", Value::Enum(visibility.to_owned()));
        element.spans.push(span);

        if is_expose {
            element.set_prop("isImportAll", Value::Bool(true));
        }

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );
        Some((id, child_key))
    }

    /// Process an expose declaration (standalone `expose <qname>[::*];`).
    ///
    /// Per spec, Expose is an Import subtype with isImportAll=true,
    /// used in ViewUsage bodies.
    pub(super) fn process_expose(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let imported_qname = self
            .find_field_text(node, "target")
            .or_else(|| self.find_qualified_name(node))?;
        let span = self.first_line_span(node);

        let is_recursive = self.node_text(node).contains("::*");

        let kind = if is_recursive {
            ElementKind::NamespaceExpose
        } else {
            ElementKind::MembershipExpose
        };

        let (child_key, mut element) = self.mint_direct_element(parent_key, parent_id, kind, None);
        element.set_prop("unresolved_importedNamespace", imported_qname.clone());
        element.set_prop("importedReference", imported_qname);
        element.set_prop("isNamespace", Value::Bool(is_recursive));
        element.set_prop("isRecursive", Value::Bool(is_recursive));
        element.set_prop("isImportAll", Value::Bool(true));
        let visibility = self
            .find_child_node(node, "visibility_indicator")
            .map(|vi| self.node_text(&vi).trim().to_owned());
        let visibility = match visibility.as_deref() {
            Some("public") => "public",
            Some("protected") => "protected",
            _ => "private",
        };
        // VisibilityKind is a closed metamodel enum — stamp Value::Enum.
        element.set_prop("visibility", Value::Enum(visibility.to_owned()));
        element.spans.push(span);

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );
        Some((id, child_key))
    }

    /// Process a `filter_decl` node — mints an `ElementFilterMembership`.
    ///
    /// Spec: SysML.xtext:229-232
    /// ```text
    /// ElementFilterMember returns SysML::ElementFilterMembership :
    ///     MemberPrefix
    ///     'filter' ownedRelatedElement += OwnedExpression ';'
    /// ;
    /// ```
    /// Routed via PackageBody (xtext:203) and PackageBodyElement
    /// (xtext:213). Mirrors `sysml-parser-batch`'s
    /// `process_element_filter_member` which captures the filter
    /// expression text into a `filterExpression` property.
    pub(super) fn process_filter(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.first_line_span(node);

        // Mint anonymous relationship element with canonical key.
        let (child_key, mut element) =
            self.mint_direct_element(parent_key, parent_id, ElementKind::ElementFilterMembership, None);

        // Extract the filter expression text. The grammar exposes the
        // body via the `expression` field; fall back to stripping the
        // leading `filter` keyword and trailing `;` for robustness on
        // partial parses.
        let expr_text = self
            .find_field_text(node, "expression")
            .or_else(|| {
                let text = self.node_text(node);
                let stripped = text
                    .trim()
                    .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
                    .trim()
                    .strip_prefix("filter")
                    .unwrap_or_else(|| {
                        text.find("filter")
                            .map(|pos| &text[pos + "filter".len()..])
                            .unwrap_or(text)
                    })
                    .trim()
                    .trim_end_matches(';')
                    .trim()
                    .to_owned();
                if stripped.is_empty() {
                    None
                } else {
                    Some(stripped)
                }
            })
            .unwrap_or_default();

        if !expr_text.is_empty() {
            element.set_prop("filterExpression", expr_text);
        }
        // Membership.visibility is a required shape prop (KerML).
        // filter_decl accepts an optional visibility_indicator; default
        // public per Membership semantics.
        let visibility = self
            .node_text(node)
            .trim_start()
            .split_whitespace()
            .next()
            .filter(|w| matches!(*w, "private" | "protected" | "public"))
            .unwrap_or("public")
            .to_owned();
        // VisibilityKind is a closed metamodel enum — stamp Value::Enum.
        element.set_prop("visibility", Value::Enum(visibility));
        element.spans.push(span);

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // G09 (partial): materialise the condition Expression subtree as a
        // child owned by the ElementFilterMembership — the spec models the
        // filter condition as the membership's memberElement, not a string
        // prop. Mirrors the calc/constraint result-expression pattern
        // (TS-1.4 / G20). The `filterExpression` text prop above stays for
        // the view-filter runtime readers.
        //
        // The ElementFilterMembership IS the condition's membership per
        // spec (memberElement/ownedMemberElement), so the root expression's
        // owning_membership points straight at the filter element — no
        // interposed OwningMembership wrapper.
        if let Some(expr_node) = node.child_by_field_name("expression") {
            let builder =
                crate::expression_elements::ExpressionBuilder::new(self.source, self.file_path);
            if let Some(expr_id) =
                builder.process_with_key(expr_node, id.clone(), &child_key, &mut result.graph)
            {
                if let Some(expr_elem) = result.graph.get_element_mut(&expr_id) {
                    expr_elem.owning_membership = Some(id.clone());
                }
            }
        }

        Some((id, child_key))
    }

    /// Process an alias declaration.
    pub(super) fn process_alias(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let name = self.find_name_field(node)?;
        let target = self.find_field_text(node, "target")?;
        let span = self.node_span(node);

        // TS-1.7 gap #13: per SysML.xtext line 234 (`AliasMember returns
        // SysML::Membership`), alias declarations produce a `Membership`
        // relationship, not a `Namespace` element. Previously routed through
        // the generic `Namespace` fallback (~268 instances corpus-wide).
        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::Membership,
            Some(name.as_str()),
        );
        element.name = Some(name);
        element.set_prop("unresolved_aliasTarget", target);
        element.spans.push(span);

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );
        Some((id, child_key))
    }

    /// Process a comment_element or doc_comment node.
    /// Extracts the body text from the doc_string child and creates a Comment/Documentation element.
    pub(super) fn process_comment(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        kind: ElementKind,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);

        // Extract body from doc_string child: /* ... */
        let body = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "doc_string")
            .map(|ds| {
                let text = self.node_text(&ds);
                // Strip /* and */ delimiters
                text.strip_prefix("/*")
                    .and_then(|s| s.strip_suffix("*/"))
                    .unwrap_or(text)
                    .trim()
                    .to_owned()
            })
            .unwrap_or_default();

        if body.is_empty() {
            return None;
        }

        let name = self.find_name_field(node);
        let (child_key, mut element) =
            self.mint_direct_element(parent_key, parent_id, kind, name.as_deref());
        element.set_prop("body", body);

        // Optional name field
        if let Some(n) = name {
            element.name = Some(n);
        }

        element.spans.push(span);
        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // G11: Mint an `Annotation` relationship for each explicit `about`
        // target on this Comment/Documentation. Per Kerml-Vocab.ttl Annotation
        // wraps an AnnotatingElement (Comment/Documentation/TextualRepresentation
        // /MetadataUsage) and its annotatedElement.
        //
        // The `comment_element` grammar rule
        //     "comment" name? ("about" qname ("," qname)*)? doc_string
        // exposes `about` targets as `qualified_name` / `identifier` children
        // (siblings of `doc_string`). We skip the `name` field child so the
        // optional name isn't mistaken for an `about` target. Comments without
        // an `about` clause produce no Annotation, matching the Pest baseline
        // (which only emits Annotation through MetadataUsage paths).
        let name_node_id = node.child_by_field_name("name").map(|n| n.id());
        let mut targets: Vec<String> = Vec::new();
        let mut walker = node.walk();
        for child in node.children(&mut walker) {
            if Some(child.id()) == name_node_id {
                continue;
            }
            let kind_name = child.kind();
            if kind_name == "qualified_name" || kind_name == "identifier" {
                let text = self.node_text(&child).trim().to_owned();
                if !text.is_empty() {
                    targets.push(text);
                }
            }
        }

        for (i, target) in targets.into_iter().enumerate() {
            create_annotation_with_key(
                &mut result.graph,
                id.clone(),
                target,
                None,
                &child_key,
                "annotation",
                i,
            );
        }

        Some((id, child_key))
    }

    /// Process a `textual_representation` node into a KerML TextualRepresentation.
    ///
    /// Grammar (KerML.xtext): `('rep' Identification?)? 'language' STRING_VALUE
    /// body=REGULAR_COMMENT`, returning `SysML::TextualRepresentation`. Per
    /// Kerml-Vocab.ttl it is an AnnotatingElement whose `body` represents its
    /// owner (the representedElement, which must be the owner) in the named
    /// `language`. It differs from Comment/Documentation only by additionally
    /// carrying that `language`. Previously this node fell through to the
    /// generic `_ => None` (no element), losing the language + body; this lowers
    /// it to a distinct `ElementKind::TextualRepresentation` owned by the
    /// annotated element (registry: tree-sitter.textual-representation-generic-lowering).
    pub(super) fn process_textual_representation(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);

        // `language` = the string_literal child, with surrounding quotes stripped.
        let language = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "string_literal")
            .map(|s| {
                let text = self.node_text(&s);
                text.strip_prefix('"')
                    .and_then(|t| t.strip_suffix('"'))
                    .unwrap_or(text)
                    .to_owned()
            });

        // `body` = the doc_string child, with `/* */` delimiters stripped
        // (identical treatment to process_comment).
        let body = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "doc_string")
            .map(|ds| {
                let text = self.node_text(&ds);
                text.strip_prefix("/*")
                    .and_then(|s| s.strip_suffix("*/"))
                    .unwrap_or(text)
                    .trim()
                    .to_owned()
            });

        let name = self.find_name_field(node);
        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::TextualRepresentation,
            name.as_deref(),
        );
        if let Some(lang) = language {
            element.set_prop("language", lang);
        }
        if let Some(b) = body {
            element.set_prop("body", b);
        }
        if let Some(n) = name {
            element.name = Some(n);
        }

        element.spans.push(span);
        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );
        Some((id, child_key))
    }
}
