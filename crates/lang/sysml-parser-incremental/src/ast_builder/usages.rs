//! Usage processing — process_usage and the per-keyword usage-kind classifiers
//! (flow_connection_usage_kind, kerml_usage_kind, standard_usage_kind),
//! plus augment_from_split_sibling, process_actor_usage,
//! emit_default_value_expression, create_usage_rels, extract_usage.

use super::{AstBuilder, ModelGraphResult, RelKind};
use sysml_core::{CanonicalKey, ElementId, ElementKind, ModelGraph, Value};
use sysml_parser_trait::extraction::UsageExtraction;
use sysml_parser_trait::relationship_builder::{
    create_conjugated_port_typing_with_key, create_feature_typing_with_key,
    create_redefinition_with_key, create_reference_subsetting_with_key,
    create_subsetting_with_key,
};
use tree_sitter::Node;

/// Count relationship elements of `kind` already owned by `feature_id`.
///
/// Used by [`AstBuilder::create_usage_rels`] to seed per-role sibling counters
/// when the same feature is re-visited via a split sibling (e.g. chained
/// `:>> A :>> B` produces a `standard_usage` + `feature_redefinition` pair
/// that both feed the same feature). Cheap because owner→children is indexed.
fn count_owned_rels(graph: &ModelGraph, feature_id: &ElementId, kind: ElementKind) -> usize {
    graph
        .children_of(feature_id)
        .filter(|e| e.kind == kind)
        .count()
}

impl<'a> AstBuilder<'a> {
    /// Lower a `#keyword` PrefixMetadataAnnotation owned by the declaration
    /// it precedes.
    ///
    /// The annotation itself is an anonymous MetadataUsage with a real
    /// FeatureTyping to the named metadata definition, exactly like `@Type`.
    /// RequirementDerivation's three normative SemanticMetadata definitions
    /// also imply subsetting of their declared `baseType` (§7.27.3):
    ///
    /// - `<derivation>` → `derivations`
    /// - `<original>` → `originalRequirements`
    /// - `<derive>` → `derivedRequirements`
    ///
    /// Those base features remain unresolved names here. The normal workspace
    /// resolution pass must resolve them to the imported normative library
    /// elements; the B1 elaborator only classifies those resolved anchors.
    pub(super) fn process_prefix_metadata_annotation(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let annotation_type = self.find_field_text(node, "type")?.trim().to_owned();
        if annotation_type.is_empty() {
            return None;
        }

        let (annotation_key, mut annotation) =
            self.mint_direct_element(parent_key, parent_id, ElementKind::MetadataUsage, None);
        annotation.spans.push(self.node_span(node));
        annotation.set_prop("annotationType", annotation_type.clone());
        annotation.set_prop("unresolvedTypeName", annotation_type.clone());
        annotation.set_prop("isPrefixMetadata", Value::Bool(true));
        let annotation_id = self.add_with_ownership_keyed(
            annotation,
            parent_id,
            parent_key,
            &annotation_key,
            &mut result.graph,
        );

        let type_span = node
            .child_by_field_name("type")
            .map(|type_node| self.node_span(&type_node));
        create_feature_typing_with_key(
            &mut result.graph,
            annotation_id.clone(),
            annotation_type.clone(),
            type_span.clone(),
            &annotation_key,
            "typing",
            0,
        );

        // The parser's semantic fence has no project/library graph yet, so it
        // records the normative base feature as an unresolved relationship.
        // Resolution later proves the target is the imported library anchor;
        // without that resolution the elaborator deliberately does nothing.
        let local_name = annotation_type
            .rsplit("::")
            .next()
            .unwrap_or(annotation_type.as_str());
        let base_type = match local_name {
            "derivation" | "DerivationMetadata" => Some("derivations"),
            "original" | "OriginalRequirementMetadata" => Some("originalRequirements"),
            "derive" | "DerivedRequirementMetadata" => Some("derivedRequirements"),
            _ => None,
        };
        if let (Some(base_type), Some(annotated_id), Some(key)) =
            (base_type, parent_id.as_ref(), parent_key)
        {
            let index = count_owned_rels(&result.graph, annotated_id, ElementKind::Subsetting);
            create_subsetting_with_key(
                &mut result.graph,
                annotated_id.clone(),
                base_type.to_owned(),
                type_span,
                key,
                "semantic_metadata_base_type",
                index,
            );
        }

        Some((annotation_id, annotation_key))
    }

    /// Augment an existing element from a split sibling CST node.
    ///
    /// The tree-sitter grammar sometimes splits a SysML declaration into two
    /// sibling nodes: one for the keyword (e.g. `part_def`, `standard_usage`)
    /// and one for the rest (`feature_declaration`, `feature_redefinition`).
    ///
    /// This method takes the split sibling node and updates the previously
    /// created element with name, name_span, typing, and specialization
    /// relationships extracted from it. The `build` loop handles re-parenting
    /// the body children under this element.
    pub(super) fn augment_from_split_sibling(
        &self,
        node: &Node<'a>,
        element_id: &ElementId,
        element_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) {
        // Update name and name_span on the element.
        // Also overwrite synthetic names (e.g. `$fork_0`) that were assigned
        // to anonymous control nodes — the real name lives in this split node.
        if let Some(elem) = result.graph.elements.get_mut(element_id) {
            let has_synthetic_name = elem.name.as_ref().is_some_and(|n| n.starts_with('$'));
            if elem.name.is_none() || has_synthetic_name {
                elem.name = self.find_name_field(node);
            }
            if elem.get_prop("declaredShortName").is_none() {
                if let Some(short_name) = self.find_short_name(node) {
                    elem.set_prop("declaredShortName", short_name);
                }
            }
            if elem.name_span.is_none() || has_synthetic_name {
                if let Some(name_node) = self.find_name_node(node) {
                    elem.name_span = Some(self.node_span(&name_node));
                }
            }
            // Extend the element's span to cover the full declaration + body
            let full_span = self.node_span(node);
            if let Some(s) = elem.spans.first_mut() {
                s.end = full_span.end;
            }
        }

        // Create typing/specialization relationships from the split node.
        // When tree-sitter splits a declaration, feature_redefinition contains
        // the supertype_list (`:>` part). Per KerML spec, `:>` means
        // Subclassification on classifiers but Subsetting on features/usages.
        // We must check the element kind to dispatch correctly.
        let element_is_usage = result
            .graph
            .elements
            .get(element_id)
            .map(|e| e.kind.is_usage())
            .unwrap_or(false);

        if node.kind() == "feature_declaration" || element_is_usage {
            self.create_usage_rels(node, element_id, element_key, &mut result.graph);
            // TS-1.4 gap #4: split-sibling feature_declaration / feature_redefinition
            // can carry a `default_value` (e.g. `:>> quantity = isq.T;`). Without
            // this call the expression subtree was silently dropped — the bulk
            // of the FeatureReferenceExpression / LiteralInteger delta against
            // Pest came from this path.
            if let Some(key) = element_key {
                self.emit_default_value_expression(node, element_id, key, &mut result.graph);
            }
            // G15 (TS-3.7a): when the split sibling carries the `default_value`,
            // also lift the literal RHS onto the parent element as a typed
            // `value` prop. Without this, `out attribute y : Real default 10.0;`
            // following a comment with parens-and-`=` (which triggers the
            // standard_usage→feature_declaration split) leaves the AttributeUsage
            // without a `value` prop, so the composite SSR ODE builder reads
            // y's initial condition as 0.0 and the bouncing-ball simulation
            // produces y=-0.049m instead of expected free-fall trajectory.
            // Mirrors the value-extraction half of `extract_usage`/`build_element`.
            self.lift_split_default_value(node, element_id, &mut result.graph);
        } else {
            // feature_redefinition on a definition → Subclassification
            self.create_definition_rels(node, element_id, element_key, &mut result.graph);
        }
    }

    /// Lift a literal `default_value` from a split-sibling node onto the
    /// parent element as a typed `value` prop, plus `isDefault` / `isInitial`.
    ///
    /// Mirrors the value-extraction half of [`Self::extract_usage`] /
    /// `UsageExtraction::build_element`: when the trailing `feature_declaration`
    /// carries `default <literal>` (or `= <literal>`), the parent element
    /// should expose the same typed `value` prop as if the declaration had
    /// not been split. This is what composite SSR (ODE state initial values),
    /// constraint evaluation, and CLI context extraction read.
    ///
    /// Skips when the RHS is a non-literal expression — the structured
    /// subtree is already attached by `emit_default_value_expression`.
    fn lift_split_default_value(
        &self,
        node: &Node<'a>,
        element_id: &ElementId,
        graph: &mut ModelGraph,
    ) {
        let Some(default_value) = self.find_child_node(node, "default_value") else {
            return;
        };

        // G22: classify the FeatureValue separator per KerML `FeatureValue`
        // (KerML.xtext:740-746): `=` ⇒ neither flag; `:=` ⇒ isInitial; `default`
        // ⇒ isDefault; `default :=` ⇒ both. Scan the anonymous separator tokens
        // of `default_value` rather than assuming every bound value is a default
        // — a plain `=` binding is a concrete BindingConnector (isDefault=false),
        // NOT a deferred default. Mirrors the same classification in
        // `extract_usage` / `build_element`.
        let (mut is_initial, mut is_default) = (false, false);
        {
            let mut cursor = default_value.walk();
            for child in default_value.children(&mut cursor) {
                if child.is_named() {
                    continue;
                }
                match self.node_text(&child) {
                    ":=" => is_initial = true,
                    "default" => is_default = true,
                    _ => {}
                }
            }
        }

        let Some(expr) = self.find_first_named_child(&default_value) else {
            return;
        };
        let expr_text = self.node_text(&expr).trim().to_owned();

        let Some(elem) = graph.get_element_mut(element_id) else {
            return;
        };

        // Mark default/initial regardless of whether the RHS is a literal —
        // matches what `build_element` does via `value_is_default` /
        // `value_is_initial`.
        if is_initial {
            elem.set_prop("isInitial", Value::Bool(true));
        }
        if is_default {
            elem.set_prop("isDefault", Value::Bool(true));
        }

        // Only lift literal RHS values. Complex expressions are kept in the
        // AST subtree minted by `emit_default_value_expression`.
        if !UsageExtraction::text_looks_like_literal(&expr_text) {
            return;
        }
        // RSC-5.1 (D-5.0.5): strip a trailing `[unit]` measurement reference and
        // fold the magnitude — same shared `split_unit_annotation` home as
        // `element_builder`'s literal fold, so split-sibling defaults bind a
        // numeric `value` (+ `unit`) rather than the raw `"0 [m]"` string (which
        // type-errors in constraint eval).
        let (mag, unit) = UsageExtraction::split_unit_annotation(&expr_text);
        let text = mag.trim();
        let parsed_value = if let Ok(i) = text.parse::<i64>() {
            Value::Int(i)
        } else if let Ok(f) = text.parse::<f64>() {
            Value::Float(f)
        } else if text == "true" {
            Value::Bool(true)
        } else if text == "false" {
            Value::Bool(false)
        } else {
            Value::String(text.trim_matches('"').to_owned())
        };
        elem.set_prop("value", parsed_value);
        if let Some(unit) = unit {
            elem.set_prop("unit", Value::String(unit.to_owned()));
        }
    }

    /// Determine the ElementKind for a `flow_connection_usage` node.
    ///
    /// The grammar uses `choice("flow", seq("succession", "flow"))` so both `flow`
    /// and `succession flow` map to the same node type. Scan anonymous children for
    /// the `"succession"` keyword to distinguish `SuccessionFlowUsage` from `FlowUsage`.
    pub(super) fn flow_connection_usage_kind(&self, node: &Node<'a>) -> ElementKind {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                continue;
            }
            let text = self.node_text(&child);
            if text == "succession" {
                return ElementKind::SuccessionFlowUsage;
            }
            // Stop scanning once we hit "flow" — "succession" always precedes it
            if text == "flow" {
                break;
            }
        }
        ElementKind::FlowUsage
    }

    /// Determine the ElementKind for a `kerml_usage` node.
    ///
    /// The grammar covers keywords: step, expr, bool, use, include, message,
    /// subset, superset, redefine. We map known keywords to specific element kinds
    /// and fall back to `Usage` for the rest.
    pub(super) fn kerml_usage_kind(&self, node: &Node<'a>) -> ElementKind {
        // Per KerML.xtext (org.omg.kerml.xtext / KerML.xtext):
        //   - `step <name>` → `Step` (mapped to ActionUsage)
        //   - `message <name>` → `FlowUsage` (with isMessage=true, set in caller)
        //   - `expr <name>` → `Expression` (KerML.xtext:951-953)
        //   - `bool <name>` → `BooleanExpression` (KerML.xtext:969-971)
        //   - `subset` / `superset` / `redefine` keywords introduce specialization
        //     relationships rather than standalone usage elements; until a dedicated
        //     dispatch arm lifts them onto the parent (see G08e in
        //     `Architectural-cleanup/tree-sitter-canonical-plan/grammar-gaps-inventory.md`),
        //     they continue to fall through to the defensive `Usage` default below.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                continue;
            }
            match self.node_text(&child) {
                "message" => return ElementKind::FlowUsage,
                "step" => return ElementKind::ActionUsage,
                "expr" => return ElementKind::Expression,
                "bool" => return ElementKind::BooleanExpression,
                _ => {}
            }
        }
        ElementKind::Usage
    }

    /// Determine the ElementKind for a `standard_usage` node by reading its `keyword` field.
    ///
    /// The tree-sitter grammar merges part_usage, attribute_usage, item_usage,
    /// occurrence_usage, and ref_usage into a single `standard_usage` rule with
    /// a `keyword` field that distinguishes them.
    ///
    /// G10: when the optional `event_prefix` field is present AND keyword is
    /// `occurrence`, the spec mints an EventOccurrenceUsage (SysML.xtext:862).
    /// Mirrors `flow_connection_usage_kind` — the broader rule still parses
    /// the syntax; ast_builder routes to the specialised ElementKind.
    pub(super) fn standard_usage_kind(&self, node: &Node<'a>) -> ElementKind {
        // The grammar (rules/usages.js:17 standard_usage) constrains the `keyword`
        // field to a fully-mapped closed set, so the `_ => Usage` default is a
        // defensive fallback (see G08d audit in grammar-gaps-inventory.md).
        if let Some(kw_node) = node.child_by_field_name("keyword") {
            let start = kw_node.start_byte();
            let end = kw_node.end_byte().min(self.source.len());
            match &self.source[start..end] {
                "part" => ElementKind::PartUsage,
                "attribute" => ElementKind::AttributeUsage,
                "item" => ElementKind::ItemUsage,
                "occurrence" => {
                    if node.child_by_field_name("event_prefix").is_some() {
                        ElementKind::EventOccurrenceUsage
                    } else {
                        ElementKind::OccurrenceUsage
                    }
                }
                "ref" => ElementKind::ReferenceUsage,
                _ => ElementKind::Usage,
            }
        } else {
            ElementKind::Usage
        }
    }

    /// Process a usage (PartUsage, ActionUsage, etc.).
    pub(super) fn process_usage(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        kind: ElementKind,
    ) -> Option<(ElementId, CanonicalKey)> {
        let mut extraction = self.extract_usage(node);
        let span = self.node_span(node);

        // Detect keyword variants that specialize the element kind.
        // "perform action" → PerformActionUsage (assert is now handled by assert_constraint_usage)
        let actual_kind =
            if kind == ElementKind::ActionUsage && self.has_anonymous_child(node, "perform") {
                ElementKind::PerformActionUsage
            } else {
                kind
            };

        // Occurrence-usage prefixes (`individual` / `snapshot` / `timeslice`) per
        // SysML §8.3.9: the xtext rules IndividualUsage and PortionUsage both
        // `returns SysML::OccurrenceUsage` with `isIndividual ?= 'individual'` /
        // `portionKind = PortionKind`. `extract_usage` already lifts those flags
        // to `isIndividual`/`portionKind` props, but a prefixed usage arrives
        // here as the generic feature-declaration default (ReferenceUsage). The
        // prefix is only well-formed on an occurrence usage, so promote the
        // generic kind to OccurrenceUsage — a usage that already carries a more
        // specific occurrence subtype (PartUsage, ItemUsage, …) keeps its kind
        // and just gains the flag.
        let actual_kind = if (extraction.is_individual || extraction.portion_kind.is_some())
            && matches!(actual_kind, ElementKind::ReferenceUsage | ElementKind::Usage)
        {
            ElementKind::OccurrenceUsage
        } else {
            actual_kind
        };

        let is_constraint_or_calc = matches!(
            actual_kind,
            ElementKind::ConstraintUsage
                | ElementKind::AssertConstraintUsage
                | ElementKind::ConstraintDefinition
                | ElementKind::CalculationDefinition
                | ElementKind::CalculationUsage
                // Invariant has a constraint_body containing a boolean
                // expression per KerML.xtext:976 — share the body
                // extraction path with constraint usages.
                | ElementKind::Invariant
        );
        let is_transition_usage = actual_kind == ElementKind::TransitionUsage;

        let (parent_key_resolved, child_key, sibling_index) = self.prep_canonical_key(
            parent_key,
            parent_id,
            &actual_kind,
            extraction.name.as_deref(),
        );
        let owner_key_for_membership = parent_key_resolved.clone();
        extraction.parent_key = Some(parent_key_resolved);
        extraction.sibling_index = sibling_index;

        let mut element = extraction.build_element(actual_kind.clone(), Some(span));
        if let Some(name_node) = self.find_name_node(node) {
            element.name_span = Some(self.node_span(&name_node));
        }

        // Extract result expression from constraint_body or function_body.
        // Store as string prop AND pending child element for ResultExpressionMembership.
        // TS-1.4: also capture the CST node so we can walk the inner expression
        // through `ExpressionBuilder` and emit a structured subtree (matching
        // what Pest does for constraint bodies; calc bodies were string-only on
        // both sides before TS-1.4).
        let mut pending_result_expr: Option<(crate::Span, String)> = None;
        let mut pending_result_expr_node: Option<Node<'a>> = None;
        if is_constraint_or_calc {
            let body = self
                .find_child_node(node, "constraint_body")
                .or_else(|| self.find_child_node(node, "function_body"));
            if let Some(body) = body {
                if let Some(result_expr) = self.find_child_node(&body, "result_expression") {
                    let expr_span = self.node_span(&result_expr);
                    let expr_text = self.node_text(&result_expr).trim().to_owned();
                    // String prop: "constraint" for constraint kinds, "expr" for calc kinds
                    let prop_name = if matches!(
                        actual_kind,
                        ElementKind::CalculationDefinition | ElementKind::CalculationUsage
                    ) {
                        "expr"
                    } else {
                        "constraint"
                    };
                    element.set_prop(prop_name, Value::String(expr_text.clone()));
                    pending_result_expr = Some((expr_span, expr_text));
                    pending_result_expr_node = Some(result_expr);
                }
            }
        }

        // Transition usages carry explicit source/target/trigger fields in grammar.
        // Extract them into element properties so elaboration can derive relationships.
        if is_transition_usage {
            self.extract_transition_usage_props(node, &mut element);
        }

        // Extract source/target endpoint properties for all connector-like usages.
        match actual_kind {
            ElementKind::FlowUsage | ElementKind::SuccessionFlowUsage => {
                // `flow`/`succession flow` use `flow_ends`; a `message` (also a
                // FlowUsage) uses `message_ends` (SysML.xtext MessageDeclaration
                // from/to). Both expose the same source/target fields, so one
                // endpoint extractor serves both (one home).
                if !self.try_extract_endpoint_props(node, &mut element, "flow_ends") {
                    self.try_extract_endpoint_props(node, &mut element, "message_ends");
                }
            }
            ElementKind::ConnectionUsage | ElementKind::InterfaceUsage => {
                self.extract_endpoint_props(node, &mut element, "connection_ends");
            }
            ElementKind::ConnectorAsUsage => {
                self.extract_endpoint_props(node, &mut element, "connector_ends");
            }
            ElementKind::BindingConnectorAsUsage => {
                // Try binding_ends child first, then fall back to direct source/target fields
                // (covers the `bind X = Y` pattern where fields are on the node itself).
                if !self.try_extract_endpoint_props(node, &mut element, "binding_ends") {
                    self.extract_direct_field_props(node, &mut element, "source", "target");
                }
            }
            ElementKind::AllocationUsage => {
                self.extract_allocation_endpoint_props(node, &mut element);
            }
            ElementKind::SuccessionAsUsage => {
                self.extract_succession_endpoint_props(node, &mut element);
            }
            _ => {}
        }

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            Some(&owner_key_for_membership),
            &child_key,
            &mut result.graph,
        );

        // Mint trigger/guard/effect as real child usages wrapped in
        // TransitionFeatureMembership(kind) (SysML v2 §8.3.18.8). This covers
        // transition_usage, state_transition_chain and the standalone
        // target_transition_usage — all route through process_usage; only the
        // dispatch merge path calls emit_transition_features itself.
        if is_transition_usage {
            self.emit_transition_features(node, &id, Some(&child_key), &mut result.graph);
        }

        // Create ResultExpressionMembership child for the result expression
        if let Some((expr_span, expr_text)) = pending_result_expr {
            let owner_id = Some(id.clone());
            let (_, mut rem) = self.mint_direct_element(
                Some(&child_key),
                &owner_id,
                ElementKind::ResultExpressionMembership,
                None,
            );
            rem.spans.push(expr_span);
            rem.set_prop("expression", Value::String(expr_text));
            rem.owner = Some(id.clone());
            result.graph.add_element(rem);
        }

        // TS-1.4 gap #4: emit a structured expression subtree for constraint
        // and calc body result expressions, mirroring what Pest does for
        // constraint bodies. The walker descends into the `result_expression`'s
        // first named child (the actual expression) so we don't get a stray
        // wrapper element in the output.
        if let Some(result_expr) = pending_result_expr_node {
            if let Some(inner) = self.find_first_named_child(&result_expr) {
                let builder =
                    crate::expression_elements::ExpressionBuilder::new(self.source, self.file_path);
                builder.process_with_key(inner, id.clone(), &child_key, &mut result.graph);
            }
        }

        // Emit a structured expression subtree for `= <expr>` / `default <expr>`,
        // mirroring what the Pest parser does. Pre-Phase-6D this slot wrote a
        // legacy `unresolved_value` string; the runtime now consumes the AST
        // children directly via `compile_expression_ast`.
        self.emit_default_value_expression(node, &id, &child_key, &mut result.graph);

        // Create relationship elements with precise per-node spans
        self.create_usage_rels(node, &id, Some(&child_key), &mut result.graph);

        // Lower type-relationship clauses on features (`featured by` →
        // TypeFeaturing, `inverse of` → FeatureInverting; also unions/etc. if
        // present on a usage).
        self.create_type_relationship_rels(node, &id, Some(&child_key), &mut result.graph);

        // Flows and messages: ADDITIONALLY materialize the spec ends/payload
        // nesting as real children (the flat source/target props above stay —
        // runtime and diagram consumers read them).
        if matches!(
            actual_kind,
            ElementKind::FlowUsage | ElementKind::SuccessionFlowUsage
        ) {
            self.emit_flow_connection_ends(node, &id, &child_key, &mut result.graph);
        }

        Some((id, child_key))
    }

    /// Walk the `default_value` CST child (if present) and project its inner
    /// expression node onto an AST subtree owned by `parent_id`. Skips the
    /// emission for purely literal RHS, since `extract_usage` already stored
    /// the typed `value` prop and a duplicate literal child would create
    /// noise the runtime would have to filter out.
    pub(super) fn emit_default_value_expression(
        &self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(default_value) = self.find_child_node(node, "default_value") else {
            return;
        };
        let Some(expr) = self.find_first_named_child(&default_value) else {
            return;
        };
        // Skip literals — the typed `value` prop on the parent already
        // carries them via `UsageExtraction::value_is_literal`.
        if matches!(
            expr.kind(),
            "integer_literal"
                | "real_literal"
                | "string_literal"
                | "boolean_literal"
                | "null_literal"
                | "infinity_literal"
                | "literal"
        ) {
            return;
        }
        let builder =
            crate::expression_elements::ExpressionBuilder::new(self.source, self.file_path);
        // S1.T11b: thread the parent canonical key into the expression walker
        // so the structured subtree's element IDs are reparse-stable.
        builder.process_with_key(expr, parent_id.clone(), parent_key, graph);
    }

    /// Mint a ReferenceUsage as a direct child of `parent_id`. Used by
    /// the for-loop loopVariable and accept payload parameter slots, where
    /// the spec wants a ParameterMembership-wrapped ReferenceUsage but the
    /// undifferentiated-child encoding (OwningMembership wrapper + plain
    /// ReferenceUsage child) is the established TS pattern for action
    /// body slot data (see `bodyAction`/`thenAction` derivation note in
    /// `spec_property_conformance.rs`). Downstream consumers reach the
    /// data by walking children; the spec slot becomes Derived rather than
    /// requiring a separate ParameterMembership envelope.
    ///
    /// Returns the minted ReferenceUsage id and its canonical key (so callers
    /// can project further reparse-stable children — e.g. a FeatureBinding
    /// expression — beneath the slot child).
    pub(super) fn mint_action_slot_child(
        &mut self,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        inner_kind: ElementKind,
        inner_name: Option<&str>,
        span: crate::Span,
        graph: &mut ModelGraph,
    ) -> (ElementId, CanonicalKey) {
        let parent_id_opt = Some(parent_id.clone());
        let (inner_key, mut inner_elem) =
            self.mint_direct_element(Some(parent_key), &parent_id_opt, inner_kind, inner_name);
        if let Some(n) = inner_name {
            inner_elem.name = Some(n.to_owned());
        }
        inner_elem.spans.push(span);
        let id = self.add_with_ownership_keyed(
            inner_elem,
            &parent_id_opt,
            Some(parent_key),
            &inner_key,
            graph,
        );
        (id, inner_key)
    }

    /// Locate the `_expression` child of a control-flow action node that
    /// sits between two reference children (the var/keyword anchor and the
    /// terminating action_body). Returns the first named child that is
    /// neither the anchor nor an action_body / typing child.
    pub(super) fn find_action_expression_child(
        &self,
        node: &Node<'a>,
        skip_var: bool,
    ) -> Option<Node<'a>> {
        let var_node = if skip_var {
            node.child_by_field_name("var")
        } else {
            None
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if let Some(ref v) = var_node {
                if child.id() == v.id() {
                    continue;
                }
            }
            match child.kind() {
                "action_body" | "typing" => continue,
                _ => {}
            }
            return Some(child);
        }
        None
    }

    /// Emit a ForLoopActionUsage's `loopVariable` slot as a ReferenceUsage
    /// child named after the `var` field. The data is reachable as an
    /// undifferentiated child (same pattern as `bodyAction`); the spec
    /// classifier marks `loopVariable` as Derived to reflect this
    /// encoding (matching the `bodyAction`/`thenAction` precedent).
    pub(super) fn emit_for_loop_variable(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(var_node) = node.child_by_field_name("var") else {
            return;
        };
        let var_text = self.node_text(&var_node).to_owned();
        if var_text.is_empty() {
            return;
        }
        let span = self.node_span(&var_node);
        let _ = self.mint_action_slot_child(
            parent_id,
            parent_key,
            ElementKind::ReferenceUsage,
            Some(&var_text),
            span,
            graph,
        );
    }

    /// Emit a ForLoopActionUsage's `seqArgument` slot as a structured
    /// expression subtree owned by the for-loop usage. Data is reachable
    /// as an undifferentiated child; classifier marks Derived.
    pub(super) fn emit_for_seq_argument(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(expr_node) = self.find_action_expression_child(node, true) else {
            return;
        };
        self.project_argument_expression(expr_node, parent_id, parent_key, graph);
    }

    /// Emit a WhileLoopActionUsage's `whileArgument` slot. Grammar puts the
    /// condition expression directly after the `while`/`until` keyword.
    pub(super) fn emit_while_argument(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(expr_node) = self.find_action_expression_child(node, false) else {
            return;
        };
        self.project_argument_expression(expr_node, parent_id, parent_key, graph);
    }

    /// Emit an IfActionUsage's `ifArgument` slot. Grammar puts the
    /// condition expression directly after the `if` keyword.
    pub(super) fn emit_if_argument(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(expr_node) = self.find_action_expression_child(node, false) else {
            return;
        };
        self.project_argument_expression(expr_node, parent_id, parent_key, graph);
    }

    /// Emit an AcceptActionUsage's `payloadParameter` slot as a
    /// ReferenceUsage child that mirrors the accept's name + (optional)
    /// typing.
    ///
    /// We intentionally KEEP the name on the AcceptActionUsage itself
    /// (per `test_accept_action_dispatch` invariant); the child mirrors
    /// the same name so the spec slot is reachable too.
    pub(super) fn emit_accept_payload_parameter(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let name_text = node
            .child_by_field_name("name")
            .map(|n| self.node_text(&n).to_owned());
        let typing_qname = self.find_child_node(node, "typing").and_then(|t| {
            self.find_child_node(&t, "type_ref")
                .and_then(|tr| self.extract_type_ref_from_node(&tr))
        });
        if name_text.is_none() && typing_qname.is_none() {
            return;
        }
        let span = node
            .child_by_field_name("name")
            .or_else(|| self.find_child_node(node, "typing"))
            .map(|n| self.node_span(&n))
            .unwrap_or_else(|| self.node_span(node));

        let (inner_id, _) = self.mint_action_slot_child(
            parent_id,
            parent_key,
            ElementKind::ReferenceUsage,
            name_text.as_deref(),
            span,
            graph,
        );

        if let Some(qname) = typing_qname {
            if let Some(elem) = graph.get_element_mut(&inner_id) {
                elem.set_prop("unresolved_type", Value::String(qname));
            }
        }
    }

    /// Emit a TerminateActionUsage's terminated-occurrence argument in the
    /// spec `NodeParameterMember` shape (SysML.xtext: `NodeParameterMember`
    /// returns ParameterMembership; `NodeParameter` returns ReferenceUsage;
    /// `FeatureBinding` returns FeatureValue owning the OwnedExpression that
    /// names the terminated occurrence — vocab derived property
    /// `terminatedOccurrenceArgument`, SysML-vocab.ttl). House encoding: an
    /// unnamed ReferenceUsage slot child (`mint_action_slot_child` — the
    /// ParameterMembership envelope is the undifferentiated-child encoding,
    /// same as the for-loop/accept slots) with the FeatureBinding expression
    /// projected as a structured subtree beneath it, so a bare-name target
    /// becomes a FeatureReferenceExpression the reference-resolution pass
    /// resolves additively.
    pub(super) fn emit_terminate_node_parameter(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let Some(target_node) = node.child_by_field_name("target") else {
            return;
        };
        let span = self.node_span(&target_node);
        let (inner_id, inner_key) = self.mint_action_slot_child(
            parent_id,
            parent_key,
            ElementKind::ReferenceUsage,
            None,
            span,
            graph,
        );
        self.project_argument_expression(target_node, &inner_id, &inner_key, graph);
    }

    /// Emit a SendActionUsage's `payloadArgument` slot. Grammar puts the
    /// payload expression directly after the `send` keyword.
    pub(super) fn emit_send_payload_argument(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let mut cursor = node.walk();
        let expr_node = node.children(&mut cursor).find(|c| c.is_named());
        let Some(expr_node) = expr_node else { return };
        self.project_argument_expression(expr_node, parent_id, parent_key, graph);
    }

    /// Helper: project the given `_expression` CST node into a structured
    /// expression subtree directly under `parent_id`. The data lands as
    /// an undifferentiated child of the action usage; the spec slot
    /// becomes Derived (matching the existing `bodyAction`/`thenAction`
    /// precedent in the conformance metric).
    fn project_argument_expression(
        &mut self,
        expr_node: Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let builder =
            crate::expression_elements::ExpressionBuilder::new(self.source, self.file_path);
        builder.process_with_key(expr_node, parent_id.clone(), parent_key, graph);
    }

    /// Collect a `feature_chain` CST node's segments as `(text, span)` pairs,
    /// in source order. Falls back to the whole trimmed chain text as a single
    /// segment when the chain has no named children (defensive — the grammar
    /// always emits identifier/quoted-name children).
    fn feature_chain_segments(&self, chain: &Node<'a>) -> Vec<(String, crate::Span)> {
        let mut segs = Vec::new();
        let mut cursor = chain.walk();
        for child in chain.children(&mut cursor) {
            if child.is_named() {
                let text = self.node_text(&child).trim().to_owned();
                if !text.is_empty() {
                    segs.push((text, self.node_span(&child)));
                }
            }
        }
        if segs.is_empty() {
            let text = self.node_text(chain).trim().to_owned();
            if !text.is_empty() {
                segs.push((text, self.node_span(chain)));
            }
        }
        segs
    }

    /// Materialize the spec ends/payload nesting of a flow or message usage as
    /// real owned children, ADDITIVELY to the flat `source`/`target` props
    /// (which runtime and diagram consumers keep reading).
    ///
    /// Payload `of X` (both flow and message — SysML.xtext
    /// PayloadFeatureMember:1289 `returns SysML::FeatureMembership`,
    /// PayloadFeature:1293 `returns SysML::PayloadFeature`, typed per the
    /// Payload fragment's OwnedFeatureTyping arm :1300):
    /// FeatureMembership → unnamed PayloadFeature → FeatureTyping
    /// (`unresolved_type`, resolved by pass 1; `elaborate::flows`
    /// derives the flat `payloadType` prop from it).
    ///
    /// Message ends (SysML.xtext MessageEventMember:1254 `returns
    /// SysML::ParameterMembership`, MessageEvent:1258 `returns
    /// SysML::EventOccurrenceUsage` owning an OwnedReferenceSubsetting):
    /// ParameterMembership → unnamed EventOccurrenceUsage →
    /// ReferenceSubsetting carrying the whole end chain
    /// (`unresolved_referencedFeature`, resolved chain-aware by pass 2).
    ///
    /// Flow ends (SysML.xtext FlowEndMember:1309 `returns
    /// SysML::EndFeatureMembership`, FlowEnd:1313 `returns SysML::FlowEnd`,
    /// FlowEndSubsetting:1318, FlowFeatureMember:1330 `returns
    /// SysML::FeatureMembership`, FlowFeature:1334 `returns
    /// SysML::ReferenceUsage`, FlowRedefinition:1338 `returns
    /// SysML::Redefinition`): EndFeatureMembership → unnamed FlowEnd
    /// (isEnd) → [ReferenceSubsetting on the chain PREFIX when the end is
    /// dotted] + FeatureMembership → unnamed ReferenceUsage → Redefinition.
    ///
    /// The FlowRedefinition stores the BARE LAST SEGMENT (`out1` of `a.out1`),
    /// exactly as the spec's own flow desugaring spells it
    /// (SysML-spec-r2025-04.txt:21842-21865: `end … references src {
    /// redefines …, src_out; }` — the prefix is consumed entirely by the
    /// FlowEndSubsetting). Resolution follows KerML 8.2.3.5.1 (derived text
    /// :6439): a redefinedFeature whose owningFeature has an owningType
    /// resolves with the general Type of each ownedSpecialization of that
    /// owningType as the local Namespace — for a FlowEnd that general IS the
    /// FlowEndSubsetting's end prefix — and FAILS if no general resolves it
    /// (no lexical fallback; pass-2 resolve_redefined_feature implements the
    /// rule literally).
    pub(super) fn emit_flow_connection_ends(
        &mut self,
        node: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) {
        let parent_id_opt = Some(parent_id.clone());

        // Payload: `of X` — the CST field is `payload` on message_usage and
        // `flow_type` on flow_connection_usage (same grammar slot).
        let payload_node = node
            .child_by_field_name("payload")
            .or_else(|| node.child_by_field_name("flow_type"));
        if let Some(pn) = payload_node {
            if let Some(qname) = self.extract_type_ref_from_node(&pn) {
                let span = self.node_span(&pn);
                let (pf_key, mut pf) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::PayloadFeature,
                    None,
                );
                pf.spans.push(span.clone());
                let pf_id = self.add_with_membership_kind_keyed(
                    pf,
                    &parent_id_opt,
                    Some(parent_key),
                    &pf_key,
                    ElementKind::FeatureMembership,
                    graph,
                );
                create_feature_typing_with_key(
                    graph,
                    pf_id,
                    qname,
                    Some(span),
                    &pf_key,
                    "typing",
                    0,
                );
            }
        }

        if let Some(ends) = self.find_child_node(node, "message_ends") {
            for field in ["source", "target"] {
                let Some(chain) = ends.child_by_field_name(field) else {
                    continue;
                };
                let text = self.node_text(&chain).trim().to_owned();
                if text.is_empty() {
                    continue;
                }
                let span = self.node_span(&chain);
                let (eou_key, mut eou) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::EventOccurrenceUsage,
                    None,
                );
                eou.spans.push(span.clone());
                let eou_id = self.add_with_membership_kind_keyed(
                    eou,
                    &parent_id_opt,
                    Some(parent_key),
                    &eou_key,
                    ElementKind::ParameterMembership,
                    graph,
                );
                create_reference_subsetting_with_key(
                    graph,
                    eou_id,
                    text,
                    Some(span),
                    &eou_key,
                    "reference",
                    0,
                );
            }
        } else if let Some(ends) = self.find_child_node(node, "flow_ends") {
            for field in ["source", "target"] {
                let Some(chain) = ends.child_by_field_name(field) else {
                    continue;
                };
                let segs = self.feature_chain_segments(&chain);
                let Some((last_text, last_span)) = segs.last().cloned() else {
                    continue;
                };
                let chain_span = self.node_span(&chain);

                let (fe_key, mut fe) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::FlowEnd,
                    None,
                );
                fe.set_prop("isEnd", true);
                fe.spans.push(chain_span.clone());
                let fe_id = self.add_with_membership_kind_keyed(
                    fe,
                    &parent_id_opt,
                    Some(parent_key),
                    &fe_key,
                    ElementKind::EndFeatureMembership,
                    graph,
                );
                let fe_id_opt = Some(fe_id.clone());

                // FlowEndSubsetting (:1318): only when the end is a dotted
                // chain — the prefix (all but the last segment) is the
                // subsetted context feature.
                if segs.len() > 1 {
                    let prefix = segs[..segs.len() - 1]
                        .iter()
                        .map(|(t, _)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let mut prefix_span = segs[0].1.clone();
                    if let Some((_, end_span)) = segs.get(segs.len() - 2) {
                        prefix_span.end = end_span.end;
                    }
                    create_reference_subsetting_with_key(
                        graph,
                        fe_id.clone(),
                        prefix,
                        Some(prefix_span),
                        &fe_key,
                        "reference",
                        0,
                    );
                }

                // FlowFeatureMember → FlowFeature → FlowRedefinition: the bare
                // last segment, per the spec's desugaring (see the doc note).
                let (ru_key, mut ru) = self.mint_direct_element(
                    Some(&fe_key),
                    &fe_id_opt,
                    ElementKind::ReferenceUsage,
                    None,
                );
                ru.spans.push(last_span.clone());
                let ru_id = self.add_with_membership_kind_keyed(
                    ru,
                    &fe_id_opt,
                    Some(&fe_key),
                    &ru_key,
                    ElementKind::FeatureMembership,
                    graph,
                );
                create_redefinition_with_key(
                    graph,
                    ru_id,
                    last_text,
                    Some(last_span),
                    &ru_key,
                    "redefinition",
                    0,
                );
            }
        }
    }

    /// Process an actor usage inside a requirement/case body → ActorMembership.
    ///
    /// Grammar: `actor [<name>] [<specializations>] [<body>] [;]`
    /// Per spec (SysML.xtext:2084-2091): ActorMembership wraps an ActorUsage
    /// which returns PartUsage. We use process_usage to handle full usage
    /// extraction (name, typing, multiplicity, etc.).
    pub(super) fn process_actor_usage(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        self.process_usage(
            node,
            parent_id,
            parent_key,
            result,
            ElementKind::ActorMembership,
        )
    }

    /// Process a stakeholder usage inside a requirement/concern/viewpoint body
    /// → StakeholderMembership wrapping a PartUsage.
    ///
    /// Grammar: `stakeholder [<name>] [<specializations>] [<body>] [;]`
    /// Per spec (SysML.xtext:2093-2099): StakeholderMembership wraps a
    /// StakeholderUsage which returns PartUsage. Mirrors `process_actor_usage`. (G08f)
    pub(super) fn process_stakeholder_usage(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        self.process_usage(
            node,
            parent_id,
            parent_key,
            result,
            ElementKind::StakeholderMembership,
        )
    }

    /// Create FeatureTyping, Subsetting, Redefinition, and ReferenceSubsetting elements
    /// from usage child nodes, using each child's span for precision.
    pub(super) fn create_usage_rels(
        &self,
        node: &Node<'a>,
        feature_id: &ElementId,
        feature_key: Option<&CanonicalKey>,
        graph: &mut ModelGraph,
    ) {
        let owner_key = self.resolve_parent_key(feature_key);
        // Per-role sibling counters so distinct (role, kind) tuples carry
        // distinct canonical keys per ADR-009 §Relationships.
        //
        // Seed from existing relationships already owned by `feature_id` so a
        // second invocation via `augment_from_split_sibling` (chained `:>>`
        // produces a sibling `feature_redefinition` node) keeps minting fresh
        // keys instead of colliding on `(parent_key, role, 0)` with the first
        // pass. Without this seeding, chained redefinitions were silently
        // de-duplicated at element insertion time.
        let mut typing_idx: usize = count_owned_rels(graph, feature_id, ElementKind::FeatureTyping);
        let mut conj_typing_idx: usize =
            count_owned_rels(graph, feature_id, ElementKind::ConjugatedPortTyping);
        let mut subsetting_idx: usize =
            count_owned_rels(graph, feature_id, ElementKind::Subsetting);
        let mut redefinition_idx: usize =
            count_owned_rels(graph, feature_id, ElementKind::Redefinition);
        let mut crosses_idx: usize =
            count_owned_rels(graph, feature_id, ElementKind::CrossSubsetting);
        let mut reference_idx: usize =
            count_owned_rels(graph, feature_id, ElementKind::ReferenceSubsetting);
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    // FeatureTyping / ConjugatedPortTyping: typing nodes contain type_ref
                    "typing" => {
                        let type_count = child.child_count();
                        for j in 0..type_count {
                            if let Some(type_child) = child.child(j) {
                                if type_child.kind() == "type_ref" {
                                    let text = self.node_text(&type_child).trim();
                                    if text.starts_with('~') {
                                        // Conjugated port typing: ~PortDef
                                        let name = text.trim_start_matches('~').trim();
                                        if !name.is_empty() {
                                            create_conjugated_port_typing_with_key(
                                                graph,
                                                feature_id.clone(),
                                                name.to_owned(),
                                                Some(self.node_span(&type_child)),
                                                &owner_key,
                                                "conjugated_typing",
                                                conj_typing_idx,
                                            );
                                            conj_typing_idx += 1;
                                        }
                                    } else if let Some(qname) =
                                        self.extract_type_ref_from_node(&type_child)
                                    {
                                        create_feature_typing_with_key(
                                            graph,
                                            feature_id.clone(),
                                            qname,
                                            Some(self.node_span(&type_child)),
                                            &owner_key,
                                            "typing",
                                            typing_idx,
                                        );
                                        typing_idx += 1;
                                    }
                                }
                            }
                        }
                    }
                    // Subsetting: supertype_list with :> contains type_ref children
                    "supertype_list" => {
                        let text = self.node_text(&child);
                        if text.contains(":>>") {
                            // Redefinitions from :>> syntax
                            redefinition_idx = self.create_keyed_rels_from_type_refs(
                                &child,
                                feature_id,
                                &owner_key,
                                "redefinition",
                                redefinition_idx,
                                graph,
                                RelKind::Redefinition,
                            );
                        } else {
                            // Subsettings from :> syntax
                            subsetting_idx = self.create_keyed_rels_from_type_refs(
                                &child,
                                feature_id,
                                &owner_key,
                                "subsetting",
                                subsetting_idx,
                                graph,
                                RelKind::Subsetting,
                            );
                        }
                    }
                    // Redefinition: redefinition nodes contain type_ref
                    "redefinition" => {
                        redefinition_idx = self.create_keyed_rels_from_type_refs(
                            &child,
                            feature_id,
                            &owner_key,
                            "redefinition",
                            redefinition_idx,
                            graph,
                            RelKind::Redefinition,
                        );
                    }
                    // Subsetting via `subsets` keyword form
                    // (`attribute proxy subsets target;`) — emits a
                    // `subsets_clause` CST node carrying bare `feature_chain` /
                    // `qualified_name` children. Same fix shape as `redefinition`
                    // / `crosses_clause` (TS-1.2). The `:>` operator form is
                    // already handled via the `supertype_list` arm above.
                    "subsets_clause" => {
                        subsetting_idx = self.create_keyed_rels_from_type_refs(
                            &child,
                            feature_id,
                            &owner_key,
                            "subsetting",
                            subsetting_idx,
                            graph,
                            RelKind::Subsetting,
                        );
                    }
                    // CrossSubsetting: crosses_clause contains feature_chain
                    "crosses_clause" => {
                        crosses_idx = self.create_keyed_rels_from_type_refs(
                            &child,
                            feature_id,
                            &owner_key,
                            "crosses",
                            crosses_idx,
                            graph,
                            RelKind::CrossSubsetting,
                        );
                    }
                    // ReferenceSubsetting: `references` keyword form
                    // (`attribute proxy references target;`). The grammar emits
                    // a bare `feature_chain` / `qualified_name` child under
                    // `references_clause` — same shape as `redefinition` /
                    // `crosses_clause` (TS-1.2). Each target produces one
                    // ReferenceSubsetting element owned by the referencing
                    // feature.
                    "references_clause" => {
                        let inner_count = child.child_count();
                        for j in 0..inner_count {
                            if let Some(inner) = child.child(j) {
                                let qname_opt = match inner.kind() {
                                    "feature_chain" | "qualified_name" => {
                                        Some(self.node_text(&inner).to_owned())
                                    }
                                    "type_ref" => self.extract_type_ref_from_node(&inner),
                                    _ => None,
                                };
                                if let Some(qname) = qname_opt {
                                    create_reference_subsetting_with_key(
                                        graph,
                                        feature_id.clone(),
                                        qname,
                                        Some(self.node_span(&inner)),
                                        &owner_key,
                                        "reference",
                                        reference_idx,
                                    );
                                    reference_idx += 1;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Extract usage information from a node.
    pub(super) fn extract_usage(&self, node: &Node<'a>) -> UsageExtraction {
        let mut extraction = UsageExtraction {
            name: self.find_name_field(node),
            short_name: self.find_short_name(node),
            ..Default::default()
        };

        // Check for flags in usage_prefix by iterating child tokens.
        // Each child of usage_prefix is an anonymous keyword token.
        // Exact matching eliminates substring bugs (e.g. "end" matching "send").
        if let Some(prefix) = self.find_child_node(node, "usage_prefix") {
            let mut cursor = prefix.walk();
            for child in prefix.children(&mut cursor) {
                match self.node_text(&child) {
                    "abstract" => extraction.is_abstract = true,
                    "variation" => extraction.is_variation = true,
                    "readonly" => extraction.is_readonly = true,
                    "derived" => extraction.is_derived = true,
                    "end" => extraction.is_end = true,
                    "ref" => extraction.is_reference = true,
                    "composite" => extraction.is_composite = true,
                    "portion" => extraction.is_portion = true,
                    "constant" => extraction.is_constant = true,
                    "variable" => extraction.is_variable = true,
                    "individual" => extraction.is_individual = true,
                    "snapshot" => {
                        extraction.is_portion = true;
                        extraction.portion_kind = Some("snapshot".to_owned());
                    }
                    "timeslice" => {
                        extraction.is_portion = true;
                        extraction.portion_kind = Some("timeslice".to_owned());
                    }
                    "in" => extraction.direction = Some("in".to_owned()),
                    "out" => extraction.direction = Some("out".to_owned()),
                    "inout" => extraction.direction = Some("inout".to_owned()),
                    _ => {}
                }
            }
        }

        // Extract typing (: Type) and conjugated typing (: ~Type)
        let (typings, conjugated) = self.extract_typings_with_conjugation(node);
        extraction.typings = typings;
        extraction.conjugated_typings = conjugated;

        // Extract specializations (:> SubsettedFeature)
        extraction.subsettings = self.extract_subsettings(node);

        // Extract redefinitions (:>> RedefinedFeature)
        extraction.redefinitions = self.extract_redefinitions(node);

        // Extract multiplicity if present (handles symbolic bounds)
        self.extract_multiplicity_full(node, &mut extraction);

        // Extract multiplicity modifiers (ordered/nonunique)
        self.extract_multiplicity_modifiers(node, &mut extraction);

        // Extract default value if present
        // Navigate to the expression child, skipping "=" / ":=" / "default" tokens
        if let Some(default_value) = self.find_child_node(node, "default_value") {
            let expr_text = self
                .find_first_named_child(&default_value)
                .map(|expr| self.node_text(&expr).to_owned())
                .unwrap_or_else(|| self.node_text(&default_value).to_owned());
            extraction.value_expression = Some(expr_text.clone());
            // G22: classify the FeatureValue separator per KerML `FeatureValue`
            // (KerML.xtext:740-746): `=` ⇒ neither flag; `:=` ⇒ isInitial;
            // `default` ⇒ isDefault; `default :=` ⇒ both. A plain `=` binding is
            // a concrete BindingConnector, NOT a deferred default — so it must
            // not carry isDefault. (Previously every bound value was flagged
            // isDefault=true, collapsing `=` bindings into defaults and never
            // surfacing isInitial for `:=`.)
            let mut cursor = default_value.walk();
            for child in default_value.children(&mut cursor) {
                if child.is_named() {
                    continue;
                }
                match self.node_text(&child) {
                    ":=" => extraction.value_is_initial = true,
                    "default" => extraction.value_is_default = true,
                    _ => {}
                }
            }
            if UsageExtraction::text_looks_like_literal(&expr_text) {
                extraction.value_is_literal = true;
            }
        }

        extraction
    }

    /// Process a `dependency_usage` node into an `ElementKind::Dependency`
    /// element.
    ///
    /// SysML.xtext Dependency: `PrefixMetadataAnnotation* 'dependency'
    /// (Identification? 'from')? client+ 'to' supplier+ RelationshipBody`.
    /// Without a `from` clause the leading identifier is the first CLIENT,
    /// not this element's name — mirror that here: `dependency a to b;`
    /// yields an anonymous Dependency with client `a`.
    ///
    /// Client/supplier references are recorded as `unresolved_client` /
    /// `unresolved_supplier` props; elaboration resolves them and
    /// synthesizes Trace (plain) or Refine (@Refinement-annotated) edges.
    /// Body annotations (`@Refinement`) become MetadataUsage children of
    /// the Dependency via the normal walk descent.
    pub(super) fn process_dependency_usage(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);
        let name_node = node.child_by_field_name("name");
        let mut endpoint_cursor = node.walk();
        let client_nodes: Vec<Node<'a>> = node
            .children_by_field_name("client", &mut endpoint_cursor)
            .collect();
        let mut endpoint_cursor = node.walk();
        let supplier_nodes: Vec<Node<'a>> = node
            .children_by_field_name("supplier", &mut endpoint_cursor)
            .collect();

        // GLR shatter recovery: a preceding sl_note can make the resolver
        // split `dependency [X from] a to b [body]` into a KEYWORD-ONLY
        // dependency_usage followed by one feature_declaration sibling per
        // token run (`X`, `from`, `a`, `to`, `b`+body). Reassemble by
        // consuming those siblings; the trailing body (if any) is processed
        // below so its annotations attach to the Dependency.
        let mut shattered_words: Vec<String> = Vec::new();
        let mut trailing_body: Option<Node<'a>> = None;
        if name_node.is_none() && client_nodes.is_empty() && supplier_nodes.is_empty() {
            let mut sib = node.next_sibling();
            while let Some(s) = sib {
                if matches!(s.kind(), "sl_note" | "ml_note" | "comment") {
                    sib = s.next_sibling();
                    continue;
                }
                if s.kind() != "feature_declaration" {
                    break;
                }
                let Some(ident) = self.find_child_node(&s, "identifier") else {
                    break;
                };
                shattered_words.push(self.node_text(&ident).trim().to_owned());
                self.consumed_nodes.insert(s.id());
                let body = self.find_child_node(&s, "usage_body");
                let terminated = body.is_some() || self.node_text(&s).trim_end().ends_with(';');
                if let Some(b) = body {
                    trailing_body = Some(b);
                }
                if terminated {
                    break;
                }
                sib = s.next_sibling();
            }
        }

        let (name, client_texts, supplier_texts) = if !shattered_words.is_empty() {
            let from_pos = shattered_words.iter().position(|w| w == "from");
            let to_pos = shattered_words.iter().position(|w| w == "to");
            match (from_pos, to_pos) {
                // `dependency X from a to b` → [X, from, a, to, b]
                (Some(f), Some(t)) if f == 1 && t == f + 2 && shattered_words.len() == t + 2 => (
                    Some(shattered_words[0].clone()),
                    vec![shattered_words[f + 1].clone()],
                    vec![shattered_words[t + 1].clone()],
                ),
                // `dependency from a to b` → [from, a, to, b]
                (Some(0), Some(2)) if shattered_words.len() == 4 => (
                    None,
                    vec![shattered_words[1].clone()],
                    vec![shattered_words[3].clone()],
                ),
                // `dependency a to b` → [a, to, b] (leading name = client)
                (None, Some(1)) if shattered_words.len() == 3 => (
                    None,
                    vec![shattered_words[0].clone()],
                    vec![shattered_words[2].clone()],
                ),
                // Unknown shatter shape: mint the element without endpoint
                // props (elaboration skips it) rather than guessing.
                _ => (None, Vec::new(), Vec::new()),
            }
        } else {
            let name = name_node
                .as_ref()
                .map(|nn| self.node_text(nn).trim().to_owned());
            let client_texts = client_nodes
                .iter()
                .map(|cn| self.node_text(cn).trim().to_owned())
                .collect();
            let supplier_texts = supplier_nodes
                .iter()
                .map(|sn| self.node_text(sn).trim().to_owned())
                .collect();
            (name, client_texts, supplier_texts)
        };

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::Dependency,
            name.as_deref(),
        );
        element.spans.push(span);
        if let Some(n) = name {
            element.name = Some(n);
            if let Some(nn) = &name_node {
                element.name_span = Some(self.node_span(nn));
            }
        }
        // Preserve the complete spec-level endpoint lists. The singular
        // properties stay populated with the first endpoint for compatibility
        // with the existing B1 elaborator; the list properties are the
        // lossless lowering contract for client×supplier elaboration.
        if let Some(c) = client_texts.first() {
            element.set_prop("unresolved_client", Value::String(c.clone()));
        }
        if let Some(s) = supplier_texts.first() {
            element.set_prop("unresolved_supplier", Value::String(s.clone()));
        }
        element.set_prop(
            "unresolved_clients",
            Value::List(client_texts.into_iter().map(Value::String).collect()),
        );
        element.set_prop(
            "unresolved_suppliers",
            Value::List(supplier_texts.into_iter().map(Value::String).collect()),
        );

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // A shatter-recovered body lives inside a CONSUMED sibling the main
        // walk will skip — process its members here so annotations
        // (`@Refinement`) attach to the Dependency.
        if let Some(body) = trailing_body {
            let mut cursor = body.walk();
            let children: Vec<Node<'a>> = body
                .children(&mut cursor)
                .filter(|c| c.is_named())
                .collect();
            for child in children {
                self.process_node(&child, &Some(id.clone()), Some(&child_key), result);
            }
        }

        Some((id, child_key))
    }
}
