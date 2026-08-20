//! Requirement-family processors: subject, requirement-constraint, objective,
//! verify-constraint, and view-rendering (ViewRenderingMembership).

use super::{AstBuilder, ModelGraphResult};
use sysml_core::{CanonicalKey, ElementId, ElementKind, ModelGraph, Value};
use sysml_parser_trait::relationship_builder::{
    create_feature_typing_with_key, create_reference_subsetting_with_key,
};
use sysml_span::Span;
use tree_sitter::Node;

impl<'a> AstBuilder<'a> {
    /// Process a `subject_requirement` node inside a requirement body.
    ///
    /// Grammar: `subject [redefines] <short_name>? <name> ... ;`
    /// Creates a SubjectMembership element named after the subject field,
    /// so the requirement elaboration pass can tag the `subject` property.
    pub(super) fn process_subject_requirement(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);

        // Extract subject name from the "subject" field in the grammar
        let subject_name = self.find_field_text(node, "subject");

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::SubjectMembership,
            subject_name.as_deref(),
        );
        element.spans.push(span);
        if let Some(name) = subject_name {
            element.name = Some(name);
        }
        // The `subject <name>` is a REFERENCE to an existing part — capture its
        // name-node byte span (`subjectNameStart/End`) so the semantic-token
        // emitter can colour it by the resolved subject's kind. Resolution is
        // stamped as `resolvedSubject` in elaboration (tag_subject_refs).
        if let Some(name_node) = node.child_by_field_name("subject") {
            element.set_prop("subjectNameStart", Value::Int(name_node.start_byte() as i64));
            element.set_prop("subjectNameEnd", Value::Int(name_node.end_byte() as i64));
        }

        // Capture the subject's declared type (`subject w : Widget`). Spec-wise the
        // type lives on the subject's `ReferenceUsage` child (Xtext SubjectMember /
        // SubjectUsage, SysML.xtext:2044-2050); we don't yet lower that intermediate
        // usage, so we stamp the type name as `unresolved_type` directly on the
        // SubjectMembership (mirrors the `referencedConstraint` prop above and is
        // read back by `resolve_feature_typing_target`-style lookups). SPEC-SILENT
        // representation simplification — without it the verification runner can't
        // bind subject attribute values (`w.mass`), so subject-referencing
        // constraints verify vacuously / inconclusively.
        if let Some(typing) = self.find_child_node(node, "typing") {
            if let Some(type_name) = self.find_field_text(&typing, "type") {
                let type_name = type_name.trim();
                if !type_name.is_empty() {
                    element.set_prop("unresolved_type", Value::String(type_name.to_owned()));
                }
            }
        }

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // Capture the subject's bound occurrence (`subject testW : Widget = lightWidget;`
        // — spec §7.24 / Annex A:1094 `subject = vehicle_b`; VerificationCases.sysml:25
        // identifies a verified requirement's subject with the case subject). The CST
        // holds the value in a `default_value` node (`= <feature reference>`). Lower it
        // to a real AST expression subtree — the SAME `emit_default_value_expression`
        // path regular usages use (`usages.rs::extract_usage`) — so the bound occurrence
        // reference becomes a child `FeatureReferenceExpression` whose name is the
        // (possibly dotted) feature path. The verification runner reads it back AST-first
        // (`cases/mod.rs::case_subject_occurrence_bindings`) to bind the
        // subject-under-test's attribute values onto the verified requirements'
        // constraints. Occurrence references are feature refs, never literals, so
        // `emit_default_value_expression`'s literal-skip never drops them. Must run after
        // `add_with_ownership_keyed` so the SubjectMembership exists in the graph for the
        // expression subtree to attach to.
        self.emit_default_value_expression(node, &id, &child_key, &mut result.graph);

        Some((id, child_key))
    }

    /// Process the reference form `require <existingUsage>;` /
    /// `assume <existingUsage>;` (spec §7.21.2 reference subsetting — the
    /// HSUV grouping idiom): a RequirementConstraintMembership that OWNS no
    /// new content and points at an existing requirement usage via the
    /// spec's `referencedConstraint` property (a requirement IS a
    /// constraint usage). Before this rule existed the form silently
    /// misparsed as an anonymous feature named `require`.
    pub(super) fn process_referenced_requirement_constraint(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        role: &str,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);
        let target = self.find_field_text(node, "target")?;
        let target = target.trim();
        if target.is_empty() {
            return None;
        }

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::RequirementConstraintMembership,
            None,
        );
        element.spans.push(span.clone());
        element.set_prop("role", Value::String(role.to_owned()));

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // Spec shape (SysML.xtext:2061-2064): the membership owns a
        // ConstraintUsage (`ownedConstraint`) which owns an
        // OwnedReferenceSubsetting → ReferenceSubsetting (:448) whose
        // `referencedFeature` is the referenced usage. `referencedConstraint`
        // is DERIVED from that relationship (SysML-vocab.ttl:2576, a
        // DERIVED_COMPUTED_PROPERTY) — never a parse-time string. Consumers read
        // it via `sysml_core::query::referenced_constraint_target`; the resolver
        // fail-hards the ReferenceSubsetting through the standard path.
        let (usage_id, usage_key) = self.mint_owned_constraint_usage(
            &id,
            &child_key,
            None,
            None,
            span.clone(),
            &mut result.graph,
        );
        create_reference_subsetting_with_key(
            &mut result.graph,
            usage_id,
            target.to_owned(),
            Some(span),
            &usage_key,
            "reference",
            0,
        );

        Some((id, child_key))
    }

    /// Process an assume/require/frame constraint inside a requirement body.
    ///
    /// Creates a RequirementConstraintMembership element (for assume/require)
    /// or FramedConcernMembership (for frame) with a `role` property set to
    /// the constraint kind. Also extracts the constraint expression if present.
    pub(super) fn process_requirement_constraint(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        role: &str,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);

        // Extract optional name from the "name" field
        let name = self.find_field_text(node, "name");

        // Per spec: assume/require → RequirementConstraintMembership,
        //           frame → FramedConcernMembership (subtype of RequirementConstraintMembership)
        let kind = match role {
            "frame" => ElementKind::FramedConcernMembership,
            _ => ElementKind::RequirementConstraintMembership,
        };

        let (child_key, mut element) =
            self.mint_direct_element(parent_key, parent_id, kind, name.as_deref());
        element.spans.push(span);
        if let Some(n) = name.clone() {
            element.name = Some(n);
        }
        element.set_prop("role", Value::String(role.to_owned()));

        // Extract constraint expression from constraint_body (inline form:
        // `require constraint { expr }`). Dual-write like ordinary constraint
        // defs/usages: the `constraint` string prop stays for legacy readers,
        // AND the CST node is walked through ExpressionBuilder below so the
        // body gets a ResultExpressionMembership + structured expression AST
        // (v2 unification, design doc §7.1 — the evaluator and
        // pretty_print_owner read the AST, never a runtime text re-parse).
        let mut pending_result_expr: Option<(Span, String)> = None;
        let mut pending_result_expr_node: Option<Node<'a>> = None;
        if let Some(body) = self.find_child_node(node, "constraint_body") {
            if let Some(result_expr) = self.find_child_node(&body, "result_expression") {
                let expr_span = self.node_span(&result_expr);
                let expr_text = self.node_text(&result_expr).trim().to_owned();
                element.set_prop("constraint", Value::String(expr_text.clone()));
                pending_result_expr = Some((expr_span, expr_text));
                pending_result_expr_node = Some(result_expr);
            }
        }

        // Reference form: `require constraint : SomeConstraintDef;` carries no
        // inline body — the constraint lives in the referenced definition, which
        // the owned ConstraintUsage specializes via a FeatureTyping (grammar:
        // SysML.xtext:2062-2064 `... FeatureSpecialization*`). Capture the type
        // name here; the FeatureTyping is minted onto the owned usage below so
        // `referencedConstraint` derives from the relationship
        // (SysML-vocab.ttl:2576) rather than a parse-time string prop. Without
        // the relationship the constraint silently vanishes and the requirement
        // passes vacuously.
        let mut pending_typing: Option<String> = None;
        if element.get_prop("constraint").is_none() {
            if let Some(typing) = self.find_child_node(node, "typing") {
                if let Some(constraint_ref) = self.find_field_text(&typing, "type") {
                    let constraint_ref = constraint_ref.trim();
                    if !constraint_ref.is_empty() {
                        pending_typing = Some(constraint_ref.to_owned());
                    }
                }
            }
        }

        let id = self.add_with_ownership_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            &mut result.graph,
        );

        // Spec shape (§8.3.21.7): the membership owns exactly one
        // ConstraintUsage (`ownedConstraint`). The body's
        // ResultExpressionMembership + structured expression subtree hang on
        // THAT usage — result expressions belong to function-like Types
        // (rule S051), never to a membership. The `: Def` reference form hangs a
        // FeatureTyping on the same usage. Mint the usage once for either form
        // (SysML.xtext:2061-2064) so `requirement_constraint_body_owner` /
        // `referenced_constraint_target` read one consistent shape.
        if pending_result_expr.is_some() || pending_typing.is_some() {
            // The `require constraint <name>` declaration name is a declaration
            // site — record its `name_span` on the owned ConstraintUsage (the
            // real semantic element, coloured Struct) so the model-token walker
            // emits it instead of leaving it magenta.
            let name_span = node
                .child_by_field_name("name")
                .map(|n| self.node_span(&n));
            let usage_span = pending_result_expr
                .as_ref()
                .map(|(s, _)| s.clone())
                .unwrap_or_else(|| self.node_span(node));
            let (usage_id, usage_key) = self.mint_owned_constraint_usage(
                &id,
                &child_key,
                name.as_deref(),
                name_span,
                usage_span,
                &mut result.graph,
            );

            if let Some((expr_span, expr_text)) = pending_result_expr {
                if let Some(usage) = result.graph.get_element_mut(&usage_id) {
                    usage.set_prop("constraint", Value::String(expr_text.clone()));
                }

                let (_, mut rem) = self.mint_direct_element(
                    Some(&usage_key),
                    &Some(usage_id.clone()),
                    ElementKind::ResultExpressionMembership,
                    None,
                );
                rem.spans.push(expr_span);
                rem.set_prop("expression", Value::String(expr_text));
                rem.owner = Some(usage_id.clone());
                result.graph.add_element(rem);

                if let Some(result_expr) = pending_result_expr_node {
                    if let Some(inner) = self.find_first_named_child(&result_expr) {
                        let builder = crate::expression_elements::ExpressionBuilder::new(
                            self.source,
                            self.file_path,
                        );
                        builder.process_with_key(
                            inner,
                            usage_id.clone(),
                            &usage_key,
                            &mut result.graph,
                        );
                    }
                }
            }

            // `: Def` reference form → FeatureTyping onto the owned usage
            // (SysML-vocab.ttl:2576 derives `referencedConstraint` from it).
            if let Some(type_qname) = pending_typing {
                create_feature_typing_with_key(
                    &mut result.graph,
                    usage_id,
                    type_qname,
                    Some(self.node_span(node)),
                    &usage_key,
                    "typing",
                    0,
                );
            }
        }

        // Redefinition clause (`require constraint foo :>> Base::foo { … }`,
        // full-chain ruling §2.1a(b)): mint a real `Redefinition` element
        // owned by the membership — the SAME lowering ordinary usages get in
        // `create_usage_rels` — so `redefined_feature_name` (the shared
        // sysml-core reader) and the chain walker's suppression see it.
        // Before the grammar admitted `_feature_specialization` here, the
        // `:>>` clause shattered into error-recovery debris (a phantom
        // ReferenceUsage carrying the Redefinition, expression body lost).
        let mut redefinition_idx = 0usize;
        let membership_owner_key = self.resolve_parent_key(Some(&child_key));
        let rel_count = node.child_count();
        for i in 0..rel_count {
            if let Some(child) = node.child(i) {
                if child.kind() == "redefinition" {
                    redefinition_idx = self.create_keyed_rels_from_type_refs(
                        &child,
                        &id,
                        &membership_owner_key,
                        "redefinition",
                        redefinition_idx,
                        &mut result.graph,
                        super::RelKind::Redefinition,
                    );
                }
            }
        }

        Some((id, child_key))
    }

    /// Process an objective requirement inside a case body → ObjectiveMembership.
    ///
    /// Grammar: `objective [<short_name>] [<name>] [<specializations>] [<body>] [;]`
    /// The spec wraps the objective RequirementUsage in an ObjectiveMembership.
    pub(super) fn process_objective_requirement(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);
        let name = self.find_field_text(node, "name");

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::ObjectiveMembership,
            name.as_deref(),
        );
        element.spans.push(span);
        if let Some(n) = name {
            element.name = Some(n);
        }
        // The `objective <name>` declaration name is a declaration site, not a
        // reference — record its `name_span` so the model-token walker emits it
        // (by the ObjectiveMembership's kind). Without it the name hits the
        // magenta hard-fail default in the sim-app editor.
        if let Some(name_node) = node.child_by_field_name("name") {
            element.name_span = Some(self.node_span(&name_node));
        }

        // Capture the objective's declared type (`objective o : Def`). Like the
        // subject case, the type formally lives on the objective's
        // RequirementUsage child, which we don't yet lower; stamp the type name
        // as `unresolved_type` directly on the ObjectiveMembership so the
        // resolver's role-membership-typing arm resolves it (and counts a
        // missing target as unresolved) rather than silently dropping it.
        if let Some(typing) = self.find_child_node(node, "typing") {
            if let Some(type_name) = self.find_field_text(&typing, "type") {
                let type_name = type_name.trim();
                if !type_name.is_empty() {
                    element.set_prop("unresolved_type", Value::String(type_name.to_owned()));
                }
            }
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

    /// Process a verify constraint inside an objective body → RequirementVerificationMembership.
    ///
    /// Grammar: `verify <target> [;]`
    /// The spec uses RequirementVerificationMembership to identify requirements
    /// verified by a VerificationCase.
    pub(super) fn process_verify_constraint(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);

        // Extract the verified requirement reference from the "target" field
        let target_ref = self.find_field_text(node, "target");

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::RequirementVerificationMembership,
            None,
        );
        element.spans.push(span);
        if let Some(t) = target_ref {
            element.set_prop("verifiedRequirement", Value::String(t));
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


    /// Process a render usage in view bodies → ViewRenderingMembership.
    ///
    /// Grammar: `render <specializations> <body> ;`
    ///        | `render rendering [<name>] <specializations> <body> [;]`
    pub(super) fn process_render_usage(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let span = self.node_span(node);
        let name = self.find_field_text(node, "name");

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::ViewRenderingMembership,
            name.as_deref(),
        );
        element.spans.push(span);
        if let Some(n) = name {
            element.name = Some(n);
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


    // ========== Relationship creation with precise spans ==========

    /// Mint the `ConstraintUsage` a RequirementConstraintMembership /
    /// FramedConcernMembership owns (the spec's `ownedConstraint`,
    /// SysML.xtext:2052-2055 `ownedRelatedElement += RequirementConstraintUsage`;
    /// §8.3.21.7 — exactly one). Returns `(usage_id, usage_key)` so the caller
    /// can hang the body's ResultExpressionMembership, a ReferenceSubsetting
    /// (bare-name reference form) or a FeatureTyping (`: Def` reference form) on
    /// it. Keeping the usage-mint in one place lets all three forms share the
    /// spec shape that `requirement_constraint_body_owner` reads back.
    fn mint_owned_constraint_usage(
        &mut self,
        membership_id: &ElementId,
        membership_key: &CanonicalKey,
        name: Option<&str>,
        name_span: Option<Span>,
        span: Span,
        graph: &mut ModelGraph,
    ) -> (ElementId, CanonicalKey) {
        let membership_id_opt = Some(membership_id.clone());
        let (usage_key, mut usage) = self.mint_direct_element(
            Some(membership_key),
            &membership_id_opt,
            ElementKind::ConstraintUsage,
            name,
        );
        usage.spans.push(span);
        if let Some(n) = name {
            usage.name = Some(n.to_owned());
        }
        if let Some(ns) = name_span {
            usage.name_span = Some(ns);
        }
        usage.owner = Some(membership_id.clone());
        let usage_id = usage.id.clone();
        graph.add_element(usage);
        (usage_id, usage_key)
    }
}
