//! State machine processors — extract_transition_usage_props,
//! process_state_subaction, process_state_transition_chain,
//! extract_target_transition_props.

use super::{AstBuilder, ModelGraphResult};
use sysml_core::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Value};
use tree_sitter::Node;

/// Strip ONE layer of balanced outer parentheses from a trigger-expression
/// text, if present. The tree-sitter `_expression` rule captures the whole
/// `(t_dead)` parenthesized form, but the runtime's `parse_trigger_from_event`
/// expects the inner expression text (it re-wraps for `after(...)`).
fn strip_outer_parens(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('(') && trimmed.ends_with(')') {
        // Confirm the leading `(` matches the trailing `)` (not e.g. `(a)+(b)`).
        let mut depth = 0i32;
        for (i, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        // Outer paren closes at the final byte → safe to strip.
                        if i + 1 == trimmed.len() {
                            return trimmed[1..trimmed.len() - 1].trim();
                        }
                        return trimmed;
                    }
                }
                _ => {}
            }
        }
    }
    trimmed
}

impl<'a> AstBuilder<'a> {
    /// Extract transition-specific properties from a transition_usage node.
    pub(super) fn extract_transition_usage_props(&self, node: &Node<'a>, element: &mut Element) {
        if let Some(transition_source) = self.find_child_node(node, "transition_source") {
            if let Some(source_node) = transition_source.child_by_field_name("source") {
                let source = self.node_text(&source_node);
                let source = source.trim_matches('\'').trim_matches('"').trim();
                if !source.is_empty() {
                    element.set_prop("source", source.to_owned());
                    // Capture the source-name reference-site span (byte offsets in
                    // this file) so the semantic-token emitter can colour it by the
                    // resolved state's kind (Phase B.2). `Value` has no Span variant,
                    // so we record start/end as int companion props.
                    element.set_prop("sourceNameStart", Value::Int(source_node.start_byte() as i64));
                    element.set_prop("sourceNameEnd", Value::Int(source_node.end_byte() as i64));
                }
            }
        }

        if let Some(transition_target) = self.find_child_node(node, "transition_target") {
            if let Some(target_node) = transition_target.child_by_field_name("target") {
                let target = self.node_text(&target_node);
                let target = target.trim_matches('\'').trim_matches('"').trim();
                if !target.is_empty() {
                    element.set_prop("target", target.to_owned());
                    // See sourceName* above — target-name reference-site span.
                    element.set_prop("targetNameStart", Value::Int(target_node.start_byte() as i64));
                    element.set_prop("targetNameEnd", Value::Int(target_node.end_byte() as i64));
                }
            }
        }

        // Trigger/guard/effect are NOT props on the TransitionUsage — they are
        // real child usages wrapped in TransitionFeatureMembership(kind), minted
        // post-insertion by `emit_transition_features` (SysML v2 §8.3.18.8).
    }

    /// Extract the guard expression text from a `guard_expression` CST node:
    /// strip the leading `if` keyword and one layer of `[...]` brackets.
    fn extract_guard_text(&self, guard_node: &Node<'a>) -> Option<String> {
        let mut text = self.node_text(guard_node).trim().to_owned();
        if let Some(rest) = text.strip_prefix("if") {
            text = rest.trim().to_owned();
        }
        if text.starts_with('[') && text.ends_with(']') && text.len() >= 2 {
            text = text[1..text.len() - 1].trim().to_owned();
        }
        (!text.is_empty()).then_some(text)
    }

    /// Stamp the `TransitionFeatureKind` label onto the wrapping
    /// `TransitionFeatureMembership` of a freshly minted transition feature.
    fn stamp_transition_membership_kind(graph: &mut ModelGraph, child_id: &ElementId, kind: &str) {
        if let Some(mem_id) = graph.owning_membership_of(child_id).map(|m| m.id.clone()) {
            if let Some(mem) = graph.get_element_mut(&mem_id) {
                mem.set_prop("kind", kind.to_owned());
            }
        }
    }

    /// Materialize a TransitionUsage's trigger/guard/effect as real owned
    /// children wrapped in `TransitionFeatureMembership(kind)` per SysML v2
    /// §8.3.18.8 / SysML.xtext:1884-1914:
    ///
    /// - trigger → `AcceptActionUsage` (TriggerActionMember, kind = trigger;
    ///   `TriggerAction returns SysML::AcceptActionUsage`, xtext:1892). For a
    ///   port trigger the accept-parameter name becomes a real `ReferenceUsage`
    ///   payload child (spec `payloadParameter`, xtext:1444-1456).
    /// - guard → `Expression` (GuardExpressionMember, kind = guard, xtext:1896).
    /// - effect → `ActionUsage` (EffectBehaviorMember, kind = effect;
    ///   `EffectBehaviorUsage returns SysML::ActionUsage`, xtext:1912).
    ///
    /// Each child carries the exact former string prop as its `text` prop —
    /// the single textual source consumers derive from (the TransitionUsage
    /// itself no longer carries `trigger`/`guard`/`effect`/`accept_param`).
    /// Read back via `ModelGraph::transition_feature_text` /
    /// `transition_accept_param`.
    ///
    /// `transition_key` is `None` only on the dispatch merge path (a
    /// `target_transition_usage` merged into a transition minted in an earlier
    /// loop iteration, whose `CanonicalKey` is no longer in hand); there the
    /// parent key is derived from the transition id — itself canonical — per
    /// the elaboration edge-key precedent, so ids stay reparse-stable.
    pub(super) fn emit_transition_features(
        &mut self,
        node: &Node<'a>,
        transition_id: &ElementId,
        transition_key: Option<&CanonicalKey>,
        graph: &mut ModelGraph,
    ) {
        let synth_key;
        let parent_key: &CanonicalKey = match transition_key {
            Some(k) => k,
            None => {
                synth_key = CanonicalKey::root(&transition_id.to_string());
                &synth_key
            }
        };
        let parent_id_opt = Some(transition_id.clone());

        // Trigger: `trigger_action` (transition_usage / target_transition_usage)
        // or `trigger_accept` (state_transition_chain) — differing CST field
        // names for the accept-parameter slot.
        let trigger_node = self
            .find_child_node(node, "trigger_action")
            .map(|n| (n, false))
            .or_else(|| {
                self.find_child_node(node, "trigger_accept")
                    .map(|n| (n, true))
            });
        if let Some((tnode, is_accept_chain)) = trigger_node {
            let text = if is_accept_chain {
                self.extract_trigger_accept_string(&tnode)
            } else {
                self.extract_trigger_action_string(&tnode)
            };
            if let Some(text) = text {
                let (child_key, mut elem) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::AcceptActionUsage,
                    None,
                );
                elem.set_prop("text", text);
                elem.spans.push(self.node_span(&tnode));
                let accept_id = self.add_with_membership_kind_keyed(
                    elem,
                    &parent_id_opt,
                    Some(parent_key),
                    &child_key,
                    ElementKind::TransitionFeatureMembership,
                    graph,
                );
                Self::stamp_transition_membership_kind(graph, &accept_id, "trigger");

                // Port-trigger payload parameter (`accept <name> via <port>`):
                // the canonical trigger string cannot carry the name, so it
                // becomes a real named ReferenceUsage child of the accept
                // action (spec payloadParameter slot).
                let name_field = if is_accept_chain { "trigger_name" } else { "trigger" };
                if let Some(param) = self.extract_accept_param(&tnode, name_field) {
                    let span = self.node_span(&tnode);
                    self.mint_action_slot_child(
                        &accept_id,
                        &child_key,
                        ElementKind::ReferenceUsage,
                        Some(&param),
                        span,
                        graph,
                    );
                }
            }
        }

        // Guard: owned Boolean-valued Expression (text-only in this arc; the
        // structured expression subtree is a documented follow-up).
        if let Some(guard_node) = self.find_child_node(node, "guard_expression") {
            if let Some(text) = self.extract_guard_text(&guard_node) {
                let (child_key, mut elem) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::Expression,
                    None,
                );
                elem.set_prop("text", text);
                elem.spans.push(self.node_span(&guard_node));
                let guard_id = self.add_with_membership_kind_keyed(
                    elem,
                    &parent_id_opt,
                    Some(parent_key),
                    &child_key,
                    ElementKind::TransitionFeatureMembership,
                    graph,
                );
                Self::stamp_transition_membership_kind(graph, &guard_id, "guard");
            }
        }

        // Effect: owned ActionUsage; `text` keeps the raw CST clause (e.g.
        // `do action { eff = 1; }`) exactly as the former `effect` prop did —
        // the runtime unwraps/parses it unchanged.
        if let Some(effect_node) = self
            .find_child_node(node, "effect_action")
            .or_else(|| self.find_child_node(node, "effect_do"))
        {
            let effect = self.node_text(&effect_node).trim().to_owned();
            if !effect.is_empty() {
                let (child_key, mut elem) = self.mint_direct_element(
                    Some(parent_key),
                    &parent_id_opt,
                    ElementKind::ActionUsage,
                    None,
                );
                elem.set_prop("text", effect);
                elem.spans.push(self.node_span(&effect_node));
                let effect_id = self.add_with_membership_kind_keyed(
                    elem,
                    &parent_id_opt,
                    Some(parent_key),
                    &child_key,
                    ElementKind::TransitionFeatureMembership,
                    graph,
                );
                Self::stamp_transition_membership_kind(graph, &effect_id, "effect");
            }
        }
    }

    /// Process a state subaction (entry/do/exit) → ActionUsage with stateSubactionKind.
    ///
    /// The tree-sitter grammar sometimes splits `entry action startHeater;` into
    /// two sibling CST nodes: a bare `entry_action` (just the keyword) and a
    /// separate `action_usage` with the name. When the "action" field isn't found
    /// inside the entry_action node, we look at the next sibling to absorb the
    /// action_usage's name and mark it as consumed so the build loop skips it.
    ///
    /// **TS-1.1 (gap #1 + part of gap #6)**: when an inline action body is
    /// present (`entry action { x = 20; y = 42; }`), this method also walks
    /// the body and emits an `AssignmentActionUsage` per bare-assignment
    /// statement. Tree-sitter parses `x = 20;` inside an `action_body` as a
    /// `feature_declaration` with a `default_value` (not as the
    /// `assignment_action` grammar rule, which requires the `assign` keyword).
    /// Pest's `process_bare_assignment` recognises the same shape and emits
    /// AssignmentActionUsage; we mirror that here so
    /// `StateMachineCompiler::compile_named` builds a structured entry/do/exit
    /// action (`Structured { assignments: […] }`) instead of degrading to the
    /// empty `Simple("")` variant — which silently drops every assignment.
    ///
    /// All mints go through `add_with_ownership_keyed` so the wrapping
    /// `OwningMembership` element is derived from canonical keys per ADR-009
    /// §Relationships (gap #6 — full audit lands in TS-1.6, but the state
    /// subaction sites ship with TS-1.1 because they are the critical path
    /// for runtime correctness on entry-action assignments).
    pub(super) fn process_state_subaction(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
        kind: &str,
    ) -> Option<(ElementId, CanonicalKey)> {
        // Resolve the element name first so the canonical key follows the
        // named-element rule when we manage to recover a real name (covers
        // both the in-field case and the split-sibling fallback).
        let mut name: Option<String> = None;
        if let Some(name_text) = self.find_field_text(node, "action") {
            let trimmed = name_text.trim();
            if !trimmed.is_empty() {
                name = Some(trimmed.to_owned());
            }
        }

        // Locate the inline action body. Two CST shapes are possible:
        //   1. The body is a direct child of the entry/do/exit_action node
        //      (e.g. `entry { x = 20; }` — no `action` keyword).
        //   2. The action keyword causes the grammar to split into a bare
        //      `entry_action` keyword node + a sibling `action_usage` whose
        //      `action_body` child carries the inline statements (e.g.
        //      `entry action { x = 20; }`, the common orchestration form).
        // We collect the body node from whichever shape applies; both feed the
        // same downstream assignment-emission loop.
        let mut action_body_node: Option<Node<'a>> = self.find_child_node(node, "action_body");

        // Fallback: grammar split `entry action X;` (or `entry action { ... }`)
        // into bare entry_action + action_usage sibling. Absorb the sibling's
        // name AND its inline action_body if present.
        if name.is_none() || action_body_node.is_none() {
            if let Some(next) = node.next_named_sibling() {
                if next.kind() == "action_usage" {
                    if name.is_none() {
                        if let Some(name_text) = self.find_field_text(&next, "name") {
                            let name_text = name_text.trim();
                            if !name_text.is_empty() {
                                name = Some(name_text.to_owned());
                            }
                        } else {
                            // Try first identifier child
                            for i in 0..next.named_child_count() {
                                if let Some(child) = next.named_child(i) {
                                    if child.kind() == "identifier" {
                                        let name_text = self.node_text(&child).trim().to_owned();
                                        if !name_text.is_empty() {
                                            name = Some(name_text);
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if action_body_node.is_none() {
                        action_body_node = self.find_child_node(&next, "action_body");
                    }
                    // Mark the action_usage sibling as consumed so the build loop skips it.
                    self.consumed_nodes.insert(next.id());
                }
            }
        }

        let (child_key, mut element) = self.mint_direct_element(
            parent_key,
            parent_id,
            ElementKind::ActionUsage,
            name.as_deref(),
        );
        if let Some(n) = name {
            element.name = Some(n);
        }

        // `stateSubactionKind` on the ActionUsage is a co-authored MIRROR of the
        // wrapping StateSubactionMembership's `kind` (set together below from the
        // same CST keyword, so they cannot drift). The membership kind is the
        // spec-faithful source of truth (SysML.xtext:1767-1785 —
        // Entry/Do/ExitActionMember `returns StateSubactionMembership` with
        // `kind = Entry/Do/ExitActionKind`). The prop is retained because the
        // read surface (runtime state-machine compiler, elaboration, diagram,
        // ide-db tokens, cardinality) still reads it; migrating those consumers
        // to read the membership kind is a follow-up (would span four crates).
        element.set_prop("stateSubactionKind", kind.to_owned());
        element.spans.push(self.node_span(node));

        // Materialize the spec `StateSubactionMembership` wrapper (a subtype of
        // OwningMembership) instead of a plain OwningMembership, carrying the
        // entry/do/exit discriminator as its `kind`. Keyed via canonical keys so
        // the ID is reparse-stable per ADR-009 §Relationships. The owned
        // ActionUsage — not the membership — lands in `owner_to_children`, so
        // consumers that walk `children_of(state)` are unchanged.
        let id = self.add_with_membership_kind_keyed(
            element,
            parent_id,
            parent_key,
            &child_key,
            ElementKind::StateSubactionMembership,
            &mut result.graph,
        );
        if let Some(mem_id) = result
            .graph
            .owning_membership_of(&id)
            .map(|m| m.id.clone())
        {
            if let Some(mem) = result.graph.get_element_mut(&mem_id) {
                mem.set_prop("kind", kind.to_owned());
            }
        }

        // Gap #1: walk the inline action body (if any) and emit one
        // AssignmentActionUsage per bare-assignment statement. The grammar
        // parses `x = 20;` inside an action_body as a `feature_declaration`
        // with a `default_value` child — extract that shape and synthesize the
        // structured assignment element the runtime compiler expects.
        if let Some(body) = action_body_node {
            self.emit_state_subaction_assignments(&body, &id, &child_key, result);
        }

        // Other children (e.g. send_inline) are visited by the build loop automatically.
        Some((id, child_key))
    }

    /// Walk an inline `action_body` belonging to a state subaction (entry/do/
    /// exit) and emit one `AssignmentActionUsage` per bare-assignment
    /// statement.
    ///
    /// Tree-sitter parses `x = 20;` inside an action body as a
    /// `feature_declaration` with a `default_value` child (because the
    /// grammar reuses the `_definition_or_usage` rule for action body items —
    /// see `tree-sitter/rules/actions.js`). We recognise that shape and mint
    /// a structured `AssignmentActionUsage` element with:
    /// - `name = target`
    /// - `target`, `targetFeature` props pointing at the LHS identifier
    /// - `value` prop for literal RHS (Int / Float / Bool / String),
    ///   matching what `StateMachineCompiler::compile_action_from_children`
    ///   reads in `crates/lang/sysml-runtime/src/statemachine/mod.rs`.
    ///
    /// The feature_declaration node is marked as consumed so the build loop
    /// skips it — preventing a double-emission as a stray `Feature` element
    /// owned by the subaction wrapper.
    fn emit_state_subaction_assignments(
        &mut self,
        action_body: &Node<'a>,
        parent_id: &ElementId,
        parent_key: &CanonicalKey,
        result: &mut ModelGraphResult,
    ) {
        let mut cursor = action_body.walk();
        // Collect feature_declaration nodes first to avoid mutating the tree
        // walk in-place while marking consumed_nodes.
        let mut fdecls: Vec<Node<'a>> = Vec::new();
        let mut assigns: Vec<Node<'a>> = Vec::new();
        for child in action_body.children(&mut cursor) {
            match child.kind() {
                "feature_declaration" => fdecls.push(child),
                "assignment_action" => assigns.push(child),
                _ => {}
            }
        }
        // G19: spec-standard `assign x := value;` form. The grammar parses
        // it as an `assignment_action` node with two unnamed `_expression`
        // children (target, value). The dispatch path that normally lowers
        // `assignment_action` never fires inside a state subaction body
        // because the wrapping `action_usage` is marked consumed_nodes
        // (see process_state_subaction), so the build loop never descends
        // into the action_body. Mint the AssignmentActionUsage directly
        // under the subaction wrapper so
        // `StateMachineCompiler::compile_action_from_children` finds it.
        for assign in assigns {
            let mut acursor = assign.walk();
            let mut named_children: Vec<Node<'a>> = Vec::new();
            for c in assign.children(&mut acursor) {
                if c.is_named() {
                    named_children.push(c);
                }
            }
            let Some(target_node) = named_children.first() else {
                continue;
            };
            let target = self.node_text(target_node).trim().to_owned();
            if target.is_empty() {
                continue;
            }

            let (child_key, mut element) = self.mint_direct_element(
                Some(parent_key),
                &Some(parent_id.clone()),
                ElementKind::AssignmentActionUsage,
                Some(&target),
            );
            element.name = Some(target.clone());
            element.set_prop("target", Value::String(target.clone()));
            element.set_prop("targetFeature", Value::String(target));
            element.spans.push(self.node_span(&assign));
            // The assignment TARGET (`<target> = …`) references an existing
            // feature — capture its name-node byte span so the semantic-token
            // emitter can colour it by the resolved feature's kind. Resolution
            // is stamped as `resolvedAssignmentTarget` in elaboration. (The RHS
            // is already a FeatureReferenceExpression, handled by B.1.)
            element.set_prop(
                "targetNameStart",
                Value::Int(target_node.start_byte() as i64),
            );
            element.set_prop("targetNameEnd", Value::Int(target_node.end_byte() as i64));

            // Extract a literal RHS value when the value expression is a
            // literal — descends `primary_expression` → `literal` → typed
            // literal, mirroring `extract_literal_value`. Non-literal RHS
            // leaves `value` unset; the runtime falls back to
            // `compile_expression`.
            if let Some(value_node) = named_children.get(1) {
                if let Some(literal_value) = self.extract_literal_from_expr(value_node) {
                    element.set_prop("value", literal_value);
                }
            }

            let _new_id = self.add_with_ownership_keyed(
                element,
                &Some(parent_id.clone()),
                Some(parent_key),
                &child_key,
                &mut result.graph,
            );

            self.consumed_nodes.insert(assign.id());
        }
        for fdecl in fdecls {
            let Some(default_value) = self.find_child_node(&fdecl, "default_value") else {
                // Not a bare assignment shape — leave for the build loop.
                continue;
            };
            let Some(name_text) = self.find_field_text(&fdecl, "name") else {
                // No identifiable LHS — skip.
                continue;
            };
            let target = name_text.trim().to_owned();
            if target.is_empty() {
                continue;
            }

            let (child_key, mut element) = self.mint_direct_element(
                Some(parent_key),
                &Some(parent_id.clone()),
                ElementKind::AssignmentActionUsage,
                Some(&target),
            );
            element.name = Some(target.clone());
            element.set_prop("target", Value::String(target.clone()));
            element.set_prop("targetFeature", Value::String(target));
            element.spans.push(self.node_span(&fdecl));
            // Capture the assignment TARGET name span (`<target> = …`) so the
            // semantic-token emitter can colour it by the resolved feature's
            // kind (elaboration stamps `resolvedAssignmentTarget`). This is the
            // bare-assignment shape (`V_applied = V_supply` parses as a
            // feature_declaration), the common case inside entry/do/exit actions.
            if let Some(name_node) = fdecl.child_by_field_name("name") {
                element.set_prop(
                    "targetNameStart",
                    Value::Int(name_node.start_byte() as i64),
                );
                element.set_prop("targetNameEnd", Value::Int(name_node.end_byte() as i64));
            }

            // Populate `value` with a typed literal when the RHS is one. The
            // CST shape for `x = 20;` is:
            //   feature_declaration
            //     name: identifier
            //     default_value
            //       primary_expression
            //         literal
            //           integer_literal | real_literal | boolean_literal
            //                          | string_literal | null_literal
            // (`null_literal` is intentionally not coerced — Pest stores it
            // as Value::Null and the runtime is happy with that.)
            let rhs_is_literal = if let Some(literal_value) = self.extract_literal_value(&default_value) {
                element.set_prop("value", literal_value);
                true
            } else {
                false
            };

            let new_id = self.add_with_ownership_keyed(
                element,
                &Some(parent_id.clone()),
                Some(parent_key),
                &child_key,
                &mut result.graph,
            );

            // Non-literal RHS (e.g. `V_applied = 0 - V_supply`): project the
            // value-expression subtree onto the AssignmentActionUsage so the
            // runtime's `compile_action_from_children` →
            // `compile_expression(child, graph)` fallback has an expression to
            // evaluate. Without this the assignment compiles to Value::Null,
            // which surfaces downstream as NaN once the ODE reads the target.
            if !rhs_is_literal {
                self.emit_default_value_expression(&fdecl, &new_id, &child_key, &mut result.graph);
            }

            // Prevent the build loop from emitting a stray Feature element
            // for the bare-assignment feature_declaration we just absorbed.
            self.consumed_nodes.insert(fdecl.id());
        }
    }

    /// Walk a `default_value` CST node and extract a typed `Value` if its
    /// expression is a literal. Returns `None` for non-literal RHS (the
    /// caller leaves `value` unset and the runtime falls back to expression
    /// compilation).
    fn extract_literal_value(&self, default_value: &Node<'a>) -> Option<Value> {
        // Descend through wrapper nodes (primary_expression → literal) to the
        // innermost typed-literal node.
        let mut cur = self.find_first_named_child(default_value)?;
        loop {
            match cur.kind() {
                "primary_expression" | "literal" => {
                    cur = self.find_first_named_child(&cur)?;
                }
                _ => break,
            }
        }
        let text = self.node_text(&cur).trim().to_owned();
        match cur.kind() {
            "integer_literal" => text.parse::<i64>().ok().map(Value::Int),
            "real_literal" => text.parse::<f64>().ok().map(Value::Float),
            "boolean_literal" => match text.as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            "string_literal" => {
                // String literals include surrounding quotes; strip them.
                let stripped = text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&text)
                    .to_owned();
                Some(Value::String(stripped))
            }
            _ => None,
        }
    }

    /// G19 helper: walk an expression CST node (e.g. the RHS of an
    /// `assignment_action`) and extract a typed `Value` if its expression
    /// is a literal. Returns `None` for non-literal RHS. Unlike
    /// `extract_literal_value` which expects a `default_value` wrapper,
    /// this starts from the expression node itself (typically
    /// `primary_expression`).
    fn extract_literal_from_expr(&self, expr_node: &Node<'a>) -> Option<Value> {
        let mut cur = *expr_node;
        loop {
            match cur.kind() {
                "primary_expression" | "literal" => {
                    cur = self.find_first_named_child(&cur)?;
                }
                _ => break,
            }
        }
        let text = self.node_text(&cur).trim().to_owned();
        match cur.kind() {
            "integer_literal" => text.parse::<i64>().ok().map(Value::Int),
            "real_literal" => text.parse::<f64>().ok().map(Value::Float),
            "boolean_literal" => match text.as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            "string_literal" => {
                let stripped = text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&text)
                    .to_owned();
                Some(Value::String(stripped))
            }
            _ => None,
        }
    }

    /// Process a state_transition_chain → TransitionUsage with trigger/guard/effect/target.
    pub(super) fn process_state_transition_chain(
        &mut self,
        node: &Node<'a>,
        parent_id: &Option<ElementId>,
        parent_key: Option<&CanonicalKey>,
        result: &mut ModelGraphResult,
    ) -> Option<(ElementId, CanonicalKey)> {
        let (id, child_key) = self.process_usage(
            node,
            parent_id,
            parent_key,
            result,
            ElementKind::TransitionUsage,
        )?;

        // Extract target from transition_target child
        if let Some(target_node) = self.find_child_node(node, "transition_target") {
            if let Some(target) = self.find_field_text(&target_node, "target") {
                let target = target.trim();
                if !target.is_empty() {
                    if let Some(elem) = result.graph.get_element_mut(&id) {
                        elem.set_prop("target", target.to_owned());
                    }
                }
            }
        }

        // Trigger/guard/effect child usages + TransitionFeatureMemberships are
        // minted by `emit_transition_features`, called from `process_usage`
        // (this method routes through it above).

        Some((id, child_key))
    }

    /// Extract properties from target_transition_usage (inline transitions in state bodies).
    ///
    /// Grammar: `optional(trigger_action) optional(guard_expression) optional(effect) "then" target`
    /// The target field is directly on the node, not inside a transition_target child.
    pub(super) fn extract_target_transition_props(
        &self,
        node: &Node<'a>,
        id: &ElementId,
        graph: &mut ModelGraph,
    ) {
        // Extract target from direct field
        if let Some(target) = self.find_field_text(node, "target") {
            let target = target.trim();
            if !target.is_empty() {
                if let Some(elem) = graph.get_element_mut(id) {
                    elem.set_prop("target", target.to_owned());
                }
            }
        }

        // Trigger/guard/effect child usages + TransitionFeatureMemberships:
        // the standalone path routes through `process_usage` (which calls
        // `emit_transition_features`); the merge path calls it explicitly
        // from dispatch with the merged-into transition's id.
    }

    /// Build the trigger string for a `trigger_action` CST node.
    ///
    /// Grammar (`rules/states.js`):
    ///   trigger_action = "accept" optional(field("trigger", name))
    ///                    optional(typing)
    ///                    optional(choice(
    ///                      "via" <port>,
    ///                      "when" field("when_guard", expr),
    ///                      "after" field("after_expr", expr),
    ///                    ))
    ///
    /// G17 (TS-3.7a): the previous lowering only emitted the bare `trigger`
    /// name and dropped `after_expr` / `when_guard`, so the runtime SM
    /// compiler never saw `after(t_dead)` triggers. We now reconstruct the
    /// canonical string form the runtime's `parse_trigger_from_event`
    /// expects: `after(<expr>)`, `when <expr>`, or just `<name>`.
    pub(super) fn extract_trigger_action_string(&self, trigger_action: &Node<'a>) -> Option<String> {
        // Port trigger (`accept <name?> via <port>`): the spec keys the trigger
        // on the receiver port (TransitionPerformances.kerml:28-46 binds the
        // `via` expression to triggerTarget). Lower to the canonical
        // `accept via <port>` form the runtime's `parse_trigger_from_event`
        // decodes into `TriggerKind::PortMessage`. The accepted name/type bind
        // the payload (SysML-vocab.ttl:2495) — not yet carried here (L34/RSC-3.5b
        // payload-type matching is a documented follow-up).
        if let Some(via_port) = self.find_field_text(trigger_action, "via_port") {
            let port = via_port.trim();
            if !port.is_empty() {
                return Some(format!("accept via {}", port));
            }
        }
        if let Some(after_expr) = self.find_field_text(trigger_action, "after_expr") {
            let inner = strip_outer_parens(after_expr.trim());
            if !inner.is_empty() {
                return Some(format!("after({})", inner));
            }
        }
        if let Some(when_guard) = self.find_field_text(trigger_action, "when_guard") {
            let inner = strip_outer_parens(when_guard.trim());
            if !inner.is_empty() {
                return Some(format!("when {}", inner));
            }
        }
        if let Some(trigger) = self.find_field_text(trigger_action, "trigger") {
            let trigger = trigger.trim_matches('\'').trim_matches('"').trim();
            if !trigger.is_empty() {
                return Some(trigger.to_owned());
            }
        }
        None
    }

    /// Extract the accept-parameter name from a port trigger
    /// (`accept <name> via <port>`). Returns the `<name>` ONLY when the trigger
    /// is a port trigger (has a `via_port` field) — for non-port triggers the
    /// name field IS the event/trigger name and is already lowered into the
    /// canonical trigger string. The canonical string cannot carry this name,
    /// so it is stamped as a separate `accept_param` prop and threaded into the
    /// runtime `PortMessage` trigger to bind the delivered payload
    /// (Transfers.kerml:254-266 `binding payload = acceptedTransfer.payload`).
    /// `name_field` is `trigger` for `trigger_action`, `trigger_name` for
    /// `trigger_accept`.
    pub(super) fn extract_accept_param(
        &self,
        trigger_node: &Node<'a>,
        name_field: &str,
    ) -> Option<String> {
        self.find_field_text(trigger_node, "via_port")?;
        let name = self.find_field_text(trigger_node, name_field)?;
        let name = name.trim_matches('\'').trim_matches('"').trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    /// Build the trigger string for a `trigger_accept` CST node (inline
    /// `accept ... then T` chains inside state bodies).
    ///
    /// Grammar (`rules/states.js`):
    ///   trigger_accept = "accept" optional(field("trigger_name", name))
    ///                    optional(":" field("trigger_type", type_ref))
    ///                    optional(choice(
    ///                      "via" <port>,
    ///                      "when" field("guard", expr),
    ///                      "after" field("after_expr", expr),
    ///                    ))
    pub(super) fn extract_trigger_accept_string(&self, trigger_accept: &Node<'a>) -> Option<String> {
        // Port trigger — see `extract_trigger_action_string`. Lower `accept ...
        // via <port>` to the canonical `accept via <port>` form decoded into
        // `TriggerKind::PortMessage` by the runtime.
        if let Some(via_port) = self.find_field_text(trigger_accept, "via_port") {
            let port = via_port.trim();
            if !port.is_empty() {
                return Some(format!("accept via {}", port));
            }
        }
        if let Some(after_expr) = self.find_field_text(trigger_accept, "after_expr") {
            let inner = strip_outer_parens(after_expr.trim());
            if !inner.is_empty() {
                return Some(format!("after({})", inner));
            }
        }
        // The `trigger_accept` rule uses field name "guard" for the when-clause
        // (vs "when_guard" in `trigger_action`); accept either spelling so the
        // helper is tolerant of future grammar tweaks.
        if let Some(when_guard) = self
            .find_field_text(trigger_accept, "when_guard")
            .or_else(|| self.find_field_text(trigger_accept, "guard"))
        {
            let inner = strip_outer_parens(when_guard.trim());
            if !inner.is_empty() {
                return Some(format!("when {}", inner));
            }
        }
        if let Some(trigger) = self.find_field_text(trigger_accept, "trigger_name") {
            let trigger = trigger.trim();
            if !trigger.is_empty() {
                return Some(trigger.to_owned());
            }
        }
        None
    }
}
