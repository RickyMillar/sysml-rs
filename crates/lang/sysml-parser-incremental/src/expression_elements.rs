//! Convert tree-sitter expression CST nodes into ModelGraph expression elements.
//!
//! Emits the same shape as the Pest batch parser's
//! `crates/lang/sysml-parser-batch/src/ast/expression.rs::ExpressionBuilder`,
//! so that `compile_expression_ast` in the runtime consumes both without
//! knowing which parser produced the graph.
//!
//! CST node kinds this walker handles map 1:1 to the rules in
//! `tree-sitter/rules/expressions.js` (and the literal/name rules in
//! `types.js`).
//!
//! Recursion depth is bounded; expression trees are shallow.

use std::collections::HashMap;

use sysml_core::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph, Value, VisibilityKind};
use sysml_span::Span;
use tree_sitter::Node;

const MAX_DEPTH: usize = 128;

/// Per-parent recursion context. Tracks the parent's canonical key (when known)
/// and a per-kind sibling counter so newly-minted anonymous expression elements
/// receive deterministic IDs derived via `CanonicalKey::for_anonymous`.
///
/// When `parent_key` is `None`, all minting falls through to the legacy
/// fresh-UUID path (`Element::new_with_kind`), preserving today's behaviour
/// for unmigrated callers (see ADR-009 / S1.T6).
struct Ctx<'k> {
    parent_key: Option<&'k CanonicalKey>,
    /// Per-kind sibling counter, keyed by `ElementKind::as_str()` to avoid
    /// requiring Ord/Hash derives on the generated `ElementKind` enum.
    sibling_counts: HashMap<&'static str, usize>,
}

impl<'k> Ctx<'k> {
    fn new(parent_key: Option<&'k CanonicalKey>) -> Self {
        Self {
            parent_key,
            sibling_counts: HashMap::new(),
        }
    }

    /// Reserve a sibling-index slot for a child of the given kind under this
    /// parent. Returns the canonical key that should be used to mint the
    /// child element, or `None` when no parent_key is threaded (legacy path).
    fn next_child_key(&mut self, kind: &ElementKind) -> Option<CanonicalKey> {
        let parent = self.parent_key?;
        let kind_str: &'static str = kind.as_str();
        let slot = self.sibling_counts.entry(kind_str).or_insert(0);
        let idx = *slot;
        *slot += 1;
        Some(CanonicalKey::for_anonymous(parent, kind_str, idx))
    }
}

/// Builds expression element subtrees from tree-sitter CST nodes.
pub struct ExpressionBuilder<'s> {
    source: &'s str,
    file_path: &'s str,
}

impl<'s> ExpressionBuilder<'s> {
    pub fn new(source: &'s str, file_path: &'s str) -> Self {
        Self { source, file_path }
    }

    /// Process an expression CST node, emitting child elements rooted at
    /// `parent_id`. Returns the id of the topmost expression element.
    ///
    /// This is the legacy entry point: minted elements receive fresh UUIDs.
    /// Callers that have a `CanonicalKey` for `parent_id` should use
    /// [`ExpressionBuilder::process_with_key`] for reparse-stable IDs.
    pub fn process(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
    ) -> Option<ElementId> {
        let mut ctx = Ctx::new(None);
        // ROOT_INDEX = -1 means "no argIndex prop" (root expression).
        self.process_inner(node, parent_id, graph, &mut ctx, 0, -1)
    }

    /// Process an expression CST node with a parent canonical key, deriving
    /// reparse-stable element IDs per ADR-009.
    ///
    /// Each emitted anonymous expression element gets an ID derived from
    /// `CanonicalKey::for_anonymous(parent, kind.as_str(), sibling_index)`,
    /// where `sibling_index` is the zero-based index among the parent's
    /// already-emitted children of the same kind.
    pub fn process_with_key(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        parent_key: &CanonicalKey,
        graph: &mut ModelGraph,
    ) -> Option<ElementId> {
        let mut ctx = Ctx::new(Some(parent_key));
        self.process_inner(node, parent_id, graph, &mut ctx, 0, -1)
    }

    fn process_inner(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        if depth > MAX_DEPTH {
            return None;
        }

        match node.kind() {
            // --- Pass-through wrappers ---------------------------------------
            "primary_expression" | "parenthesized_expression" => {
                self.passthrough(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Conditional: if c ? t else e --------------------------------
            "conditional_expression" => {
                let (op_id, op_key) =
                    self.create_op_expr("if", parent_id, graph, ctx, &node, arg_index);
                let mut child_ctx = Ctx::new(op_key.as_ref());
                let mut idx: i64 = 0;
                let mut operand_ids: Vec<(i64, ElementId)> = Vec::new();
                for name in ["condition", "then", "else"] {
                    if let Some(child) = node.child_by_field_name(name) {
                        if let Some(operand_id) = self.process_inner(
                            child,
                            op_id.clone(),
                            graph,
                            &mut child_ctx,
                            depth + 1,
                            idx,
                        ) {
                            operand_ids.push((idx, operand_id));
                        }
                        idx += 1;
                    }
                }
                // G05: synth-mint operand-Feature wrappers (x, y, z) for the
                // ternary conditional's three operands.
                for (i, operand_id) in operand_ids {
                    self.mint_operand_wrapper(
                        op_id.clone(),
                        &mut child_ctx,
                        i,
                        operand_id,
                        &node,
                        graph,
                    );
                }
                Some(op_id)
            }

            // --- Binary chains ----------------------------------------------
            "null_coalesce_expression"
            | "implies_expression"
            | "or_expression"
            | "xor_expression"
            | "and_expression"
            | "equality_expression"
            | "relational_expression"
            | "range_expression"
            | "additive_expression"
            | "multiplicative_expression"
            | "exponentiation_expression" => {
                self.process_binary_chain(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Classification: lhs (istype|hastype|@|@@|as|meta) type_ref --
            "classification_expression" => {
                self.process_classification(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Unary -------------------------------------------------------
            "unary_expression" => {
                self.process_unary(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Literals ----------------------------------------------------
            "literal" => self.passthrough(node, parent_id, graph, ctx, depth, arg_index),
            "integer_literal" => {
                Some(self.emit_literal_integer(node, parent_id, graph, ctx, arg_index))
            }
            "real_literal" => Some(self.emit_literal_real(node, parent_id, graph, ctx, arg_index)),
            "boolean_literal" => {
                Some(self.emit_literal_bool(node, parent_id, graph, ctx, arg_index))
            }
            "string_literal" => {
                Some(self.emit_literal_string(node, parent_id, graph, ctx, arg_index))
            }
            "null_literal" => Some(self.emit_null(node, parent_id, graph, ctx, arg_index)),
            "infinity_literal" => {
                Some(self.emit_literal_infinity(node, parent_id, graph, ctx, arg_index))
            }

            // --- Names / feature refs ---------------------------------------
            "identifier" | "quoted_name" | "feature_chain" | "qualified_name" => {
                // G18: `.metadata` keyword tail → MetadataAccessExpression
                // (KMLExp:411-412). Detect a chain whose final segment is the
                // bare `metadata` keyword and mint the spec-correct kind.
                if self.is_metadata_access(&node) {
                    Some(self.emit_metadata_access(node, parent_id, graph, ctx, arg_index))
                } else {
                    Some(self.emit_feature_ref_from_text(node, parent_id, graph, ctx, arg_index))
                }
            }

            // --- Member access (expr.member) — treat as feature ref of full text
            "member_access" => {
                if self.is_metadata_access(&node) {
                    Some(self.emit_metadata_access(node, parent_id, graph, ctx, arg_index))
                } else {
                    Some(self.emit_feature_ref_from_text(node, parent_id, graph, ctx, arg_index))
                }
            }

            // --- Invocation --------------------------------------------------
            "invocation_expression" => {
                self.emit_invocation(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- `new T(...)` ------------------------------------------------
            "new_expression" => {
                self.emit_constructor(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Index: arr#(idx) -------------------------------------------
            "index_expression" => {
                self.emit_index(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Arrow-call: source->fn or source->fn(args) -----------------
            "arrow_expression" => {
                self.emit_arrow_call(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Arrow with body: source->fn { in x; body } -----------------
            "arrow_body_expression" => {
                self.emit_arrow_body(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Dot-form collections (rare; grammar hook) -------------------
            "collect_expression" => {
                self.emit_dot_collection(node, "collect", parent_id, graph, ctx, depth, arg_index)
            }
            "select_expression" => {
                self.emit_dot_collection(node, "select", parent_id, graph, ctx, depth, arg_index)
            }

            // --- bracket_expression (expr[idx] — treat as IndexExpression) --
            "bracket_expression" => {
                self.emit_bracket_index(node, parent_id, graph, ctx, depth, arg_index)
            }

            // --- Arrow reduce / anything else → feature ref of full text ----
            _ => {
                // Best-effort fallback so unknown nodes don't nuke the subtree.
                Some(self.emit_feature_ref_from_text(node, parent_id, graph, ctx, arg_index))
            }
        }
    }

    /// Mint an element with a canonical-key-derived ID when ctx provides a
    /// parent key, otherwise fall back to the legacy fresh-UUID path.
    /// Returns the new element's id together with the canonical key (when
    /// derived), so callers can build a fresh `Ctx` for the element's
    /// descendants without re-querying the counter.
    fn mint_anonymous(
        kind: ElementKind,
        ctx: &mut Ctx<'_>,
    ) -> (Element, Option<CanonicalKey>) {
        match ctx.next_child_key(&kind) {
            Some(key) => {
                let elem = Element::new_with_key(kind, &key);
                (elem, Some(key))
            }
            None => (Element::new_with_kind(kind), None),
        }
    }

    fn passthrough(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        // G16 sibling fix: a `parenthesized_expression` containing multiple
        // named children separated by `,` is a sequence literal — emit it as
        // an `OperatorExpression(",")` with the children as positional
        // operands, mirroring the Pest baseline. This keeps inline
        // `attribute elements = ((0, 0.5), (1, 1.0));` shaped so the
        // SampledFunction tuple walker in `compiler.rs::extract_tuple_pairs_from_ast`
        // can recover the (domain, range) pairs.
        if node.kind() == "parenthesized_expression" {
            let mut named: Vec<Node<'_>> = Vec::new();
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    named.push(c);
                }
            }
            if named.len() >= 2 {
                // Confirm operands are comma-separated by scanning anonymous
                // tokens — guards against accidental matches on grammar
                // shapes that put two named children inside parens without
                // a `,` separator.
                let mut comma_count = 0usize;
                let mut cursor = node.walk();
                for c in node.children(&mut cursor) {
                    if !c.is_named() && self.node_text(&c).trim() == "," {
                        comma_count += 1;
                    }
                }
                if comma_count >= named.len() - 1 {
                    let (op_id, op_key) =
                        self.create_op_expr(",", parent_id, graph, ctx, &node, arg_index);
                    let mut child_ctx = Ctx::new(op_key.as_ref());
                    let mut operand_ids: Vec<(i64, ElementId)> = Vec::new();
                    for (i, operand) in named.iter().enumerate() {
                        if let Some(operand_id) = self.process_inner(
                            *operand,
                            op_id.clone(),
                            graph,
                            &mut child_ctx,
                            depth + 1,
                            i as i64,
                        ) {
                            operand_ids.push((i as i64, operand_id));
                        }
                    }
                    for (i, operand_id) in operand_ids {
                        self.mint_operand_wrapper(
                            op_id.clone(),
                            &mut child_ctx,
                            i,
                            operand_id,
                            &node,
                            graph,
                        );
                    }
                    return Some(op_id);
                }
            }
        }

        // First named child is the real inner expression.
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                return self.process_inner(child, parent_id, graph, ctx, depth + 1, arg_index);
            }
        }
        None
    }

    /// Flatten a binary chain into operands + operators and emit a
    /// left-associative tree, matching the spec (and the Pest emitter).
    /// Exponentiation is right-recursive in the grammar (`prec.right`), so it
    /// naturally produces 2-operand nodes that recurse through this path.
    fn process_binary_chain(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut operands: Vec<Node<'_>> = Vec::new();
        let mut operators: Vec<String> = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                operands.push(child);
            } else {
                let tok = self.node_text(&child).trim();
                if !tok.is_empty() {
                    operators.push(normalize_operator(tok));
                }
            }
        }

        if operands.is_empty() {
            return None;
        }
        if operands.len() == 1 {
            return self.process_inner(
                operands[0],
                parent_id,
                graph,
                ctx,
                depth + 1,
                arg_index,
            );
        }

        // If we got fewer operators than N-1, pad with gap-text scan.
        while operators.len() < operands.len() - 1 {
            let i = operators.len();
            let gap = self.text_between(&operands[i], &operands[i + 1]);
            operators.push(normalize_operator(gap.trim()));
        }

        self.build_left_chain(
            &operands,
            &operators,
            parent_id,
            graph,
            ctx,
            depth + 1,
            &node,
            arg_index,
        )
    }

    /// Build `(((a op0 b) op1 c) op2 d)` top-down (left-assoc).
    fn build_left_chain(
        &self,
        operands: &[Node<'_>],
        operators: &[String],
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        root_node: &Node<'_>,
        arg_index: i64,
    ) -> Option<ElementId> {
        if operators.is_empty() {
            return self.process_inner(operands[0], parent_id, graph, ctx, depth + 1, arg_index);
        }
        let last_op_idx = operators.len() - 1;
        let (op_id, op_key) = self.create_op_expr(
            &operators[last_op_idx],
            parent_id,
            graph,
            ctx,
            root_node,
            arg_index,
        );
        let mut child_ctx = Ctx::new(op_key.as_ref());
        // Left subtree (argIndex 0).
        let left_id = self.build_left_chain(
            &operands[..operands.len() - 1],
            &operators[..last_op_idx],
            op_id.clone(),
            graph,
            &mut child_ctx,
            depth + 1,
            root_node,
            0,
        );
        // Right operand (argIndex 1).
        let right_id = self.process_inner(
            operands[operands.len() - 1],
            op_id.clone(),
            graph,
            &mut child_ctx,
            depth + 1,
            1,
        );
        // G05: synth-mint the pilot's operand-Feature wrappers so the pilot
        // conformance allowlist (Feature -17 / FeatureValue -35 on
        // protection_core_ode) closes. The pilot wraps each positional operand in
        // a `Feature[name=x|y|...]` plus a child `FeatureValue` binding
        // the operand expression. We keep the operand as a direct child of
        // the OperatorExpression (runtime contract — see
        // `compile_operator_expression`) and add the wrappers as siblings.
        // Matching canonical paths (`<op_path>.x`, `<op_path>.x/FeatureValue[0]`)
        // makes them line up with the pilot dump for the conformance gate.
        if let Some(operand_id) = left_id.as_ref() {
            self.mint_operand_wrapper(
                op_id.clone(),
                &mut child_ctx,
                0,
                operand_id.clone(),
                root_node,
                graph,
            );
        }
        if let Some(operand_id) = right_id.as_ref() {
            self.mint_operand_wrapper(
                op_id.clone(),
                &mut child_ctx,
                1,
                operand_id.clone(),
                root_node,
                graph,
            );
        }
        Some(op_id)
    }

    /// Classification: `lhs (hastype|istype|@|@@|as|meta) type_ref`.
    fn process_classification(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut lhs: Option<Node<'_>> = None;
        let mut rhs: Option<Node<'_>> = None;
        let mut op_kw: Option<String> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                let tok = self.node_text(&child).trim();
                if matches!(tok, "hastype" | "istype" | "@" | "@@" | "as" | "meta")
                    && op_kw.is_none()
                {
                    op_kw = Some(tok.to_owned());
                }
                continue;
            }
            if child.kind() == "type_ref" {
                rhs = Some(child);
            } else if lhs.is_none() {
                lhs = Some(child);
            } else {
                rhs = Some(child);
            }
        }

        let (Some(lhs), Some(rhs), Some(op)) = (lhs, rhs, op_kw) else {
            // Fall back to passthrough if any piece is missing.
            return self.passthrough(node, parent_id, graph, ctx, depth, arg_index);
        };

        let (op_id, op_key) = self.create_op_expr(&op, parent_id, graph, ctx, &node, arg_index);
        let mut child_ctx = Ctx::new(op_key.as_ref());
        let lhs_id = self.process_inner(lhs, op_id.clone(), graph, &mut child_ctx, depth + 1, 0);
        // Emit type_ref as a FeatureReferenceExpression named by its source text.
        let (mut type_ref, type_key) =
            Self::mint_anonymous(ElementKind::FeatureReferenceExpression, &mut child_ctx);
        type_ref.name = Some(self.node_text(&rhs).trim().to_owned());
        type_ref.spans.push(self.node_span(&rhs));
        let rhs_id = self.attach(
            type_ref,
            op_id.clone(),
            graph,
            1,
            &child_ctx,
            type_key.as_ref(),
        );
        // G05: synth-mint operand-Feature wrappers for `lhs op rhs` form.
        if let Some(operand_id) = lhs_id.as_ref() {
            self.mint_operand_wrapper(
                op_id.clone(),
                &mut child_ctx,
                0,
                operand_id.clone(),
                &node,
                graph,
            );
        }
        self.mint_operand_wrapper(op_id.clone(), &mut child_ctx, 1, rhs_id, &node, graph);
        Some(op_id)
    }

    /// Unary: `(op) operand`.
    fn process_unary(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut op_text: Option<String> = None;
        let mut operand: Option<Node<'_>> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                if operand.is_none() {
                    operand = Some(child);
                }
            } else if op_text.is_none() {
                let t = self.node_text(&child).trim();
                if !t.is_empty() {
                    op_text = Some(t.to_owned());
                }
            }
        }

        let operand = operand?;

        if let Some(op) = op_text.as_deref() {
            // Fold `-LIT` / `+LIT` into a single signed literal.
            if matches!(op, "-" | "+") {
                let raw = self.node_text(&node).trim();
                if let Some(folded) = self.try_fold_signed_literal(
                    raw,
                    parent_id.clone(),
                    graph,
                    ctx,
                    &node,
                    arg_index,
                ) {
                    return Some(folded);
                }
            }
            let (op_id, op_key) =
                self.create_op_expr(op, parent_id, graph, ctx, &node, arg_index);
            let mut child_ctx = Ctx::new(op_key.as_ref());
            let operand_id =
                self.process_inner(operand, op_id.clone(), graph, &mut child_ctx, depth + 1, 0);
            // G05: synth-mint operand-Feature wrapper for unary `op x`.
            if let Some(operand_id) = operand_id.as_ref() {
                self.mint_operand_wrapper(
                    op_id.clone(),
                    &mut child_ctx,
                    0,
                    operand_id.clone(),
                    &node,
                    graph,
                );
            }
            Some(op_id)
        } else {
            self.process_inner(operand, parent_id, graph, ctx, depth + 1, arg_index)
        }
    }

    fn try_fold_signed_literal(
        &self,
        text: &str,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        node: &Node<'_>,
        arg_index: i64,
    ) -> Option<ElementId> {
        let trimmed = text.trim();
        let (kind, value) = if trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E') {
            (ElementKind::LiteralRational, Value::Float(trimmed.parse::<f64>().ok()?))
        } else {
            (ElementKind::LiteralInteger, Value::Int(trimmed.parse::<i64>().ok()?))
        };
        let (mut elem, key) = Self::mint_anonymous(kind, ctx);
        elem.set_prop("value", value);
        elem.spans.push(self.node_span(node));
        Some(self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref()))
    }

    fn emit_invocation(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut name: Option<String> = None;
        let mut arg_nodes: Vec<Node<'_>> = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            match child.kind() {
                "feature_chain" | "qualified_name" | "member_access" => {
                    if name.is_none() {
                        name = Some(self.node_text(&child).trim().to_owned());
                    }
                }
                "argument_list" => {
                    let mut c2 = child.walk();
                    for arg in child.children(&mut c2) {
                        if arg.is_named() {
                            arg_nodes.push(arg);
                        }
                    }
                }
                _ => {}
            }
        }

        let (mut elem, child_key) =
            Self::mint_anonymous(ElementKind::InvocationExpression, ctx);
        elem.name = name;
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());

        let mut child_ctx = Ctx::new(child_key.as_ref());
        for (i, arg) in arg_nodes.into_iter().enumerate() {
            // A named_argument wraps `name "=" value`; walk to its value.
            let effective = if arg.kind() == "named_argument" {
                arg.child_by_field_name("value").unwrap_or(arg)
            } else {
                arg
            };
            self.process_inner(
                effective,
                id.clone(),
                graph,
                &mut child_ctx,
                depth + 1,
                i as i64,
            );
        }
        Some(id)
    }

    fn emit_constructor(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        // new_expression = "new" invocation_expression
        let mut cursor = node.walk();
        let mut inv: Option<Node<'_>> = None;
        for child in node.children(&mut cursor) {
            if child.is_named() && child.kind() == "invocation_expression" {
                inv = Some(child);
                break;
            }
        }
        let Some(inv) = inv else {
            return self.passthrough(node, parent_id, graph, ctx, depth, arg_index);
        };

        let mut type_name: Option<String> = None;
        let mut arg_nodes: Vec<Node<'_>> = Vec::new();
        let mut c2 = inv.walk();
        for child in inv.children(&mut c2) {
            if !child.is_named() {
                continue;
            }
            match child.kind() {
                "feature_chain" | "qualified_name" | "member_access" => {
                    if type_name.is_none() {
                        type_name = Some(self.node_text(&child).trim().to_owned());
                    }
                }
                "argument_list" => {
                    let mut c3 = child.walk();
                    for arg in child.children(&mut c3) {
                        if arg.is_named() {
                            arg_nodes.push(arg);
                        }
                    }
                }
                _ => {}
            }
        }

        // `new T(...)` is a ConstructorExpression, not the abstract parent
        // InstantiationExpression. The `new` keyword is the syntactic
        // discriminator vs a plain InvocationExpression `T(...)`
        // (KerMLExpressions.xtext rule `ConstructorExpression` vs
        // `InvocationExpression`; both host an InstantiatedTypeMember, only the
        // constructor is prefixed by `new` and returns SysML::ConstructorExpression).
        let (mut elem, child_key) =
            Self::mint_anonymous(ElementKind::ConstructorExpression, ctx);
        elem.name = type_name.clone();
        if let Some(ref n) = type_name {
            elem.set_prop("type", Value::String(n.clone()));
        }
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());

        let mut child_ctx = Ctx::new(child_key.as_ref());
        for (i, arg) in arg_nodes.into_iter().enumerate() {
            // A-structured: preserve the named-argument NAME so the runtime can
            // bind `new T(field = value)` arguments by feature name into a
            // `Value::Map` (a guard can then read `payload.field`). The spec
            // binds an argument to its parameter by name (ParameterRedefinition,
            // KerMLExpressions.xtext:470-485); we stamp the name on the projected
            // value element (a TS-internal sibling of `argIndex`).
            let arg_name = if arg.kind() == "named_argument" {
                arg.child_by_field_name("name")
                    .map(|n| self.node_text(&n).trim().to_owned())
            } else {
                None
            };
            let effective = if arg.kind() == "named_argument" {
                arg.child_by_field_name("value").unwrap_or(arg)
            } else {
                arg
            };
            let child_id = self.process_inner(
                effective,
                id.clone(),
                graph,
                &mut child_ctx,
                depth + 1,
                i as i64,
            );
            if let (Some(cid), Some(name)) = (child_id, arg_name) {
                if !name.is_empty() {
                    if let Some(e) = graph.get_element_mut(&cid) {
                        e.set_prop("argName", Value::String(name));
                    }
                }
            }
        }
        Some(id)
    }

    fn emit_index(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        // index_expression = <source> "#" "(" argument_list? ")"
        let mut cursor = node.walk();
        let mut source: Option<Node<'_>> = None;
        let mut args: Option<Node<'_>> = None;
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if child.kind() == "argument_list" {
                args = Some(child);
            } else if source.is_none() {
                source = Some(child);
            }
        }
        let source = source?;

        let (mut elem, child_key) = Self::mint_anonymous(ElementKind::IndexExpression, ctx);
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());

        let mut child_ctx = Ctx::new(child_key.as_ref());
        self.process_inner(source, id.clone(), graph, &mut child_ctx, depth + 1, 0);
        if let Some(args) = args {
            let mut idx: i64 = 1;
            let mut c2 = args.walk();
            for arg in args.children(&mut c2) {
                if !arg.is_named() {
                    continue;
                }
                let effective = if arg.kind() == "named_argument" {
                    arg.child_by_field_name("value").unwrap_or(arg)
                } else {
                    arg
                };
                self.process_inner(
                    effective,
                    id.clone(),
                    graph,
                    &mut child_ctx,
                    depth + 1,
                    idx,
                );
                idx += 1;
            }
        }
        Some(id)
    }

    fn emit_bracket_index(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        // bracket_expression = <source> "[" expr "]"
        let mut source: Option<Node<'_>> = None;
        let mut idx_node: Option<Node<'_>> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if source.is_none() {
                source = Some(child);
            } else {
                idx_node = Some(child);
            }
        }
        let (Some(source), Some(idx_node)) = (source, idx_node) else {
            return self.passthrough(node, parent_id, graph, ctx, depth, arg_index);
        };

        // RSC-5.1 (D-5.0.5): `num [unit]` is the spec measurement-reference operator
        // `'['(num, mRef)` (a quantity literal), NOT an index. A numeric literal can
        // never be indexed, so a literal source unambiguously marks the unit form at
        // parse time. Lower it to the magnitude literal carrying the unit reference as
        // a `unit` property — the single representation both evaluation regimes read
        // (model-level FeatureValue eval AND occurrence-level slot mint / `infer_m_ref`
        // inference path #1). The unit text is kept verbatim (qualified refs like
        // `SI::kg` preserved); the downstream resolver maps it to a dimension/scale.
        if let Some((lit_kind, lit_value)) = self.numeric_literal_value(&source) {
            let unit = self.node_text(&idx_node).trim().to_string();
            let (mut elem, key) = Self::mint_anonymous(lit_kind, ctx);
            elem.set_prop("value", lit_value);
            elem.set_prop("unit", Value::String(unit));
            elem.spans.push(self.node_span(&node));
            return Some(self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref()));
        }

        let (mut elem, child_key) = Self::mint_anonymous(ElementKind::IndexExpression, ctx);
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());
        let mut child_ctx = Ctx::new(child_key.as_ref());
        self.process_inner(source, id.clone(), graph, &mut child_ctx, depth + 1, 0);
        self.process_inner(idx_node, id.clone(), graph, &mut child_ctx, depth + 1, 1);
        Some(id)
    }

    /// RSC-5.1 (D-5.0.5): if `node` is a numeric literal (`real_literal` /
    /// `integer_literal`, possibly wrapped in `primary_expression` / `literal`),
    /// return the `LiteralRational`/`LiteralInteger` kind and parsed `Value`.
    /// Used to recognise the `num [unit]` quantity form in `emit_bracket_index`.
    /// Signed literals (`-5 [W]`) are left to the IndexExpression path for now —
    /// the corpus has none, and the unary form needs separate handling.
    fn numeric_literal_value(&self, node: &Node<'_>) -> Option<(ElementKind, Value)> {
        // Descend through `primary_expression` / `literal` wrappers to the leaf.
        let mut cur = *node;
        loop {
            match cur.kind() {
                "real_literal" => {
                    let v = self.node_text(&cur).trim().parse::<f64>().ok()?;
                    return Some((ElementKind::LiteralRational, Value::Float(v)));
                }
                "integer_literal" => {
                    let v = self.node_text(&cur).trim().parse::<i64>().ok()?;
                    return Some((ElementKind::LiteralInteger, Value::Int(v)));
                }
                "primary_expression" | "literal" => {
                    cur = cur.named_child(0)?;
                }
                _ => return None,
            }
        }
    }

    /// Arrow-call without body: `source -> name` or `source -> name(args)`.
    /// Emits `InvocationExpression { name }` with operand[0]=source, then args.
    fn emit_arrow_call(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut source: Option<Node<'_>> = None;
        let mut fn_name: Option<String> = None;
        let mut args: Option<Node<'_>> = None;
        // Grammar: source "->" fn_ref ( "(" argument_list? ")" )?
        let mut saw_arrow = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                if self.node_text(&child).trim() == "->" {
                    saw_arrow = true;
                }
                continue;
            }
            if !saw_arrow {
                if source.is_none() {
                    source = Some(child);
                }
            } else if fn_name.is_none() {
                fn_name = Some(self.node_text(&child).trim().to_owned());
            } else if child.kind() == "argument_list" {
                args = Some(child);
            }
        }

        let source = source?;

        let (mut elem, child_key) =
            Self::mint_anonymous(ElementKind::InvocationExpression, ctx);
        elem.name = fn_name;
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());

        let mut child_ctx = Ctx::new(child_key.as_ref());
        self.process_inner(source, id.clone(), graph, &mut child_ctx, depth + 1, 0);
        if let Some(args) = args {
            let mut idx: i64 = 1;
            let mut c2 = args.walk();
            for arg in args.children(&mut c2) {
                if !arg.is_named() {
                    continue;
                }
                let effective = if arg.kind() == "named_argument" {
                    arg.child_by_field_name("value").unwrap_or(arg)
                } else {
                    arg
                };
                self.process_inner(
                    effective,
                    id.clone(),
                    graph,
                    &mut child_ctx,
                    depth + 1,
                    idx,
                );
                idx += 1;
            }
        }
        Some(id)
    }

    /// Arrow-call with lambda body: `arrow_expression { in x; body }`.
    /// Extracts the inner arrow's source + name, then walks the function_body
    /// for a body-parameter (`in x;`) and a result expression.
    fn emit_arrow_body(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        // Find the inner arrow_expression to extract source + fn_name.
        let mut arrow: Option<Node<'_>> = None;
        let mut body: Option<Node<'_>> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            match child.kind() {
                "arrow_expression" => arrow = Some(child),
                "lambda_body" | "function_body" | "definition_body" | "usage_body" => body = Some(child),
                _ => {}
            }
        }
        let arrow = arrow?;

        // Reuse the arrow-call extraction to find source + fn_name.
        let mut source: Option<Node<'_>> = None;
        let mut fn_name: Option<String> = None;
        let mut saw_arrow = false;
        let mut c2 = arrow.walk();
        for child in arrow.children(&mut c2) {
            if !child.is_named() {
                if self.node_text(&child).trim() == "->" {
                    saw_arrow = true;
                }
                continue;
            }
            if !saw_arrow {
                if source.is_none() {
                    source = Some(child);
                }
            } else if fn_name.is_none() {
                fn_name = Some(self.node_text(&child).trim().to_owned());
            }
        }
        let source = source?;

        let (mut elem, child_key) =
            Self::mint_anonymous(ElementKind::InvocationExpression, ctx);
        elem.name = fn_name;
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());

        let mut child_ctx = Ctx::new(child_key.as_ref());
        // operand[0] = source
        self.process_inner(source, id.clone(), graph, &mut child_ctx, depth + 1, 0);

        // Walk the body: emit each `in x;` as a body-parameter Feature,
        // then the result expression as the next operand.
        if let Some(body) = body {
            let mut idx: i64 = 1;
            let mut result_node: Option<Node<'_>> = None;
            let mut c3 = body.walk();
            for child in body.children(&mut c3) {
                if !child.is_named() {
                    continue;
                }
                match child.kind() {
                    // tree-sitter emits a DefaultReferenceUsage / feature with
                    // direction=in for "in x;" — if the name is extractable, emit
                    // a body-parameter Feature.
                    "lambda_parameter"
                    | "default_reference_usage"
                    | "reference_usage"
                    | "feature_usage" => {
                        if let Some(n) = self.extract_in_name(&child) {
                            let (mut f, f_key) =
                                Self::mint_anonymous(ElementKind::Feature, &mut child_ctx);
                            f.name = Some(n);
                            f.set_prop("isBodyParameter", Value::Bool(true));
                            f.spans.push(self.node_span(&child));
                            self.attach(
                                f,
                                id.clone(),
                                graph,
                                idx,
                                &child_ctx,
                                f_key.as_ref(),
                            );
                            idx += 1;
                        }
                    }
                    "result_expression" => {
                        result_node = Some(child);
                    }
                    _ => {
                        // Heuristic: if no result_expression node, the last
                        // expression-looking child is the body.
                        if self.is_expression_kind(child.kind()) {
                            result_node = Some(child);
                        }
                    }
                }
            }
            if let Some(result) = result_node {
                // If wrapped in result_expression, descend to its named child.
                let effective = if result.kind() == "result_expression" {
                    result.named_child(0).unwrap_or(result)
                } else {
                    result
                };
                self.process_inner(
                    effective,
                    id.clone(),
                    graph,
                    &mut child_ctx,
                    depth + 1,
                    idx,
                );
            }
        }

        Some(id)
    }

    /// `source.{body}` / `source.?{body}` — dot-form collect / select.
    fn emit_dot_collection(
        &self,
        node: Node<'_>,
        fn_name: &str,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        depth: usize,
        arg_index: i64,
    ) -> Option<ElementId> {
        let mut source: Option<Node<'_>> = None;
        let mut body: Option<Node<'_>> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                continue;
            }
            if source.is_none() {
                source = Some(child);
            } else {
                body = Some(child);
            }
        }
        let source = source?;

        let (mut elem, child_key) =
            Self::mint_anonymous(ElementKind::InvocationExpression, ctx);
        elem.name = Some(fn_name.to_owned());
        elem.spans.push(self.node_span(&node));
        let id = self.attach(elem, parent_id, graph, arg_index, ctx, child_key.as_ref());
        let mut child_ctx = Ctx::new(child_key.as_ref());
        self.process_inner(source, id.clone(), graph, &mut child_ctx, depth + 1, 0);
        if let Some(body) = body {
            // Emit the body expression as operand[1]. (Lambda binding extraction
            // in dot-form is deferred; arrow-body is the canonical form.)
            let effective = if self.is_expression_kind(body.kind()) {
                body
            } else {
                body.named_child(0).unwrap_or(body)
            };
            self.process_inner(effective, id.clone(), graph, &mut child_ctx, depth + 1, 1);
        }
        Some(id)
    }

    fn extract_in_name(&self, node: &Node<'_>) -> Option<String> {
        // Look for the pattern `"in" name` in the child sequence; return name.
        let mut cursor = node.walk();
        let mut saw_in = false;
        for child in node.children(&mut cursor) {
            if !child.is_named() {
                if self.node_text(&child).trim() == "in" {
                    saw_in = true;
                }
                continue;
            }
            if saw_in {
                let t = self.node_text(&child).trim().to_owned();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        None
    }

    fn is_expression_kind(&self, kind: &str) -> bool {
        matches!(
            kind,
            "conditional_expression"
                | "null_coalesce_expression"
                | "implies_expression"
                | "or_expression"
                | "xor_expression"
                | "and_expression"
                | "equality_expression"
                | "classification_expression"
                | "relational_expression"
                | "range_expression"
                | "additive_expression"
                | "multiplicative_expression"
                | "exponentiation_expression"
                | "unary_expression"
                | "primary_expression"
                | "literal"
                | "integer_literal"
                | "real_literal"
                | "boolean_literal"
                | "string_literal"
                | "null_literal"
                | "infinity_literal"
                | "identifier"
                | "quoted_name"
                | "feature_chain"
                | "qualified_name"
                | "member_access"
                | "invocation_expression"
                | "new_expression"
                | "index_expression"
                | "arrow_expression"
                | "arrow_body_expression"
                | "collect_expression"
                | "select_expression"
                | "bracket_expression"
                | "parenthesized_expression"
        )
    }

    // ========== Element emission ==========

    fn emit_feature_ref_from_text(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let (mut elem, key) =
            Self::mint_anonymous(ElementKind::FeatureReferenceExpression, ctx);
        let name = self.node_text(&node).trim().to_owned();
        if !name.is_empty() {
            elem.name = Some(name);
        }
        // The pushed span already covers exactly the reference identifier/chain
        // text — the resolution-backed token emitter reads `spans[0]` for the
        // reference site and the resolver selects FRE by kind (reading `name`),
        // so no `name_span` / `unresolved_*` prop is minted here. Keeping this
        // untouched avoids shifting the ~106 canonical-JSON baselines that
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    /// G18: Detect the `<qualified-name>.metadata` keyword-tail form per
    /// KMLExp:411-412 (Pest rule `MetadataAccessExpression =
    /// QualifiedName ~ "." ~ KW_METADATA`).
    ///
    /// Tree-sitter parses `arr.metadata` as a `feature_chain` whose final
    /// child identifier has the text `metadata`. There must be at least one
    /// preceding segment — a lone `metadata` identifier is a normal feature
    /// reference, not a metadata-access form.
    fn is_metadata_access(&self, node: &Node<'_>) -> bool {
        match node.kind() {
            "feature_chain" | "qualified_name" | "member_access" => {}
            _ => return false,
        }
        let n = node.named_child_count();
        if n < 2 {
            return false;
        }
        let last = match node.named_child(n - 1) {
            Some(c) => c,
            None => return false,
        };
        self.node_text(&last).trim() == "metadata"
    }

    /// G18: Mint a `MetadataAccessExpression` element for `<expr>.metadata`.
    /// Mirrors the Pest emitter's shape (`ast/expression.rs::emit_metadata_access`).
    fn emit_metadata_access(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let (mut elem, key) =
            Self::mint_anonymous(ElementKind::MetadataAccessExpression, ctx);
        let name = self.node_text(&node).trim().to_owned();
        if !name.is_empty() {
            elem.name = Some(name);
        }
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_literal_bool(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let val = self.node_text(&node).trim() == "true";
        let (mut elem, key) = Self::mint_anonymous(ElementKind::LiteralBoolean, ctx);
        elem.set_prop("value", Value::Bool(val));
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_literal_string(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let text = self.node_text(&node);
        let val = if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
            text[1..text.len() - 1].to_owned()
        } else {
            text.to_owned()
        };
        let (mut elem, key) = Self::mint_anonymous(ElementKind::LiteralString, ctx);
        elem.set_prop("value", Value::String(val));
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_literal_integer(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let text = self.node_text(&node).trim();
        let v = text.parse::<i64>().unwrap_or(0);
        let (mut elem, key) = Self::mint_anonymous(ElementKind::LiteralInteger, ctx);
        elem.set_prop("value", Value::Int(v));
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_literal_real(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let text = self.node_text(&node).trim();
        let v = text.parse::<f64>().unwrap_or(0.0);
        let (mut elem, key) = Self::mint_anonymous(ElementKind::LiteralRational, ctx);
        elem.set_prop("value", Value::Float(v));
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_literal_infinity(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let (mut elem, key) = Self::mint_anonymous(ElementKind::LiteralInfinity, ctx);
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    fn emit_null(
        &self,
        node: Node<'_>,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
    ) -> ElementId {
        let (mut elem, key) = Self::mint_anonymous(ElementKind::NullExpression, ctx);
        elem.spans.push(self.node_span(&node));
        self.attach(elem, parent_id, graph, arg_index, ctx, key.as_ref())
    }

    /// G05 — Synth-mint the pilot's bare operand-Feature wrapper for one
    /// positional operand of an `OperatorExpression`.
    ///
    /// The pilot's `JsonElementProcessingFacade` (per migration-plan §1.7)
    /// wraps each positional operand of an `OperatorExpression` in a
    /// `Feature` whose `name` is `x` / `y` / `z`, plus a sibling
    /// `FeatureValue` element linking the Feature to the operand
    /// expression. This is grammar-bound on the pilot side (the spec's
    /// metamodel mandates the wrapper); on TS we synth-mint it from
    /// ast_builder because the grammar only emits the operand directly.
    ///
    /// The operand stays a direct child of the `OperatorExpression`
    /// (runtime contract — `compile_operator_expression` collects
    /// expression-kind children via `argIndex`; `Feature`/`FeatureValue`
    /// are filtered out by `is_expression_element`). The wrapper Feature
    /// is added as a sibling, named so its pilot-normalised path matches
    /// the pilot dump's `<op_path>.x` / `.y` / `.z` shape. The
    /// FeatureValue element is parented to the wrapper Feature and
    /// records source=Feature / target=operand so projection-side
    /// `map_relationship_endpoints` picks the endpoints up; its path
    /// resolves to `<op_path>.x/FeatureValue[0]` to match the pilot.
    ///
    /// This closes the `Feature -17` / `FeatureValue -35` row in the
    /// `protection_core_ode` pilot-conformance allowlist.
    fn mint_operand_wrapper(
        &self,
        op_id: ElementId,
        ctx: &mut Ctx<'_>,
        arg_index: i64,
        operand_id: ElementId,
        node: &Node<'_>,
        graph: &mut ModelGraph,
    ) {
        // Pilot wrapper names: positional operand 0 -> "x", 1 -> "y", 2 -> "z",
        // higher arities reuse "arg{n}" (rare; binary chains dominate).
        let name: String = match arg_index {
            0 => "x".to_owned(),
            1 => "y".to_owned(),
            2 => "z".to_owned(),
            n => format!("arg{n}"),
        };

        // Feature wrapper. Use the named canonical-key path when ctx has a
        // parent key so the wrapper's id is reparse-stable; otherwise fall
        // back to `Element::new_with_kind` (legacy UUID path).
        let (feature_elem, feature_key) = match ctx.parent_key {
            Some(parent_key) => {
                let key = CanonicalKey::for_named(parent_key, ElementKind::Feature.as_str(), &name);
                (Element::new_with_key(ElementKind::Feature, &key), Some(key))
            }
            None => (Element::new_with_kind(ElementKind::Feature), None),
        };
        let mut feature_elem = feature_elem;
        feature_elem.name = Some(name);
        feature_elem.spans.push(self.node_span(node));
        let feature_id = match (ctx.parent_key, feature_key.as_ref()) {
            (Some(owner_key), Some(child_key)) => graph.add_owned_element_with_key(
                feature_elem,
                op_id.clone(),
                VisibilityKind::Private,
                owner_key,
                child_key,
            ),
            _ => graph.add_owned_element(feature_elem, op_id.clone(), VisibilityKind::Private),
        };

        // FeatureValue relationship-shaped element. Parent is the Feature
        // wrapper; source/target record the bind (Feature -> operand).
        let mut fv_ctx = Ctx::new(feature_key.as_ref());
        let (mut fv_elem, fv_key) =
            Self::mint_anonymous(ElementKind::FeatureValue, &mut fv_ctx);
        fv_elem.set_prop("specific", Value::Ref(feature_id.clone()));
        fv_elem.set_prop("general", Value::Ref(operand_id));
        fv_elem.spans.push(self.node_span(node));
        match (feature_key.as_ref(), fv_key.as_ref()) {
            (Some(owner_key), Some(child_key)) => {
                graph.add_owned_element_with_key(
                    fv_elem,
                    feature_id,
                    VisibilityKind::Private,
                    owner_key,
                    child_key,
                );
            }
            _ => {
                graph.add_owned_element(fv_elem, feature_id, VisibilityKind::Private);
            }
        }
    }

    fn create_op_expr(
        &self,
        operator: &str,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        ctx: &mut Ctx<'_>,
        node: &Node<'_>,
        arg_index: i64,
    ) -> (ElementId, Option<CanonicalKey>) {
        let (mut elem, key) = Self::mint_anonymous(ElementKind::OperatorExpression, ctx);
        elem.set_prop("operator", Value::String(operator.to_owned()));
        if arg_index >= 0 {
            elem.set_prop("argIndex", Value::Int(arg_index));
        }
        elem.spans.push(self.node_span(node));
        let id = match (ctx.parent_key, key.as_ref()) {
            (Some(owner_key), Some(child_key)) => graph.add_owned_element_with_key(
                elem,
                parent_id,
                VisibilityKind::Private,
                owner_key,
                child_key,
            ),
            _ => graph.add_owned_element(elem, parent_id, VisibilityKind::Private),
        };
        (id, key)
    }

    /// Attach `elem` under `parent_id` and tag it with `argIndex` so the
    /// runtime can recover operand order. When the caller threaded a
    /// canonical key through `Ctx` and `child_key` for this mint, the
    /// wrapping `OwningMembership` is also reparse-stable (ADR-009
    /// §Relationships); otherwise we fall back to the legacy fresh-UUID
    /// `add_owned_element` path.
    fn attach(
        &self,
        mut elem: Element,
        parent_id: ElementId,
        graph: &mut ModelGraph,
        arg_index: i64,
        ctx: &Ctx<'_>,
        child_key: Option<&CanonicalKey>,
    ) -> ElementId {
        if arg_index >= 0 {
            elem.set_prop("argIndex", Value::Int(arg_index));
        }
        match (ctx.parent_key, child_key) {
            (Some(owner_key), Some(ck)) => graph.add_owned_element_with_key(
                elem,
                parent_id,
                VisibilityKind::Private,
                owner_key,
                ck,
            ),
            _ => graph.add_owned_element(elem, parent_id, VisibilityKind::Private),
        }
    }

    // ========== Node utilities ==========

    fn node_text(&self, node: &Node<'_>) -> &'s str {
        let start = node.start_byte();
        let end = node.end_byte();
        if end <= self.source.len() {
            &self.source[start..end]
        } else {
            ""
        }
    }

    fn node_span(&self, node: &Node<'_>) -> Span {
        let mut span = Span::new(self.file_path, node.start_byte(), node.end_byte());
        let pos = node.start_position();
        span.line = Some(pos.row as u32 + 1);
        span.col = Some(pos.column as u32);
        span
    }

    /// Text between the end of `a` and the start of `b`, from the source.
    fn text_between(&self, a: &Node<'_>, b: &Node<'_>) -> &'s str {
        let start = a.end_byte();
        let end = b.start_byte();
        if start <= end && end <= self.source.len() {
            &self.source[start..end]
        } else {
            ""
        }
    }
}

/// Canonicalize an operator token. Matches the runtime's expected operator
/// strings (see `compile_operator_expression` in the expression compiler).
fn normalize_operator(raw: &str) -> String {
    match raw.trim() {
        "&&" | "&" | "and" => "and".into(),
        "||" | "|" | "or" => "or".into(),
        "xor" => "xor".into(),
        "implies" => "implies".into(),
        "not" => "not".into(),
        "hastype" => "hastype".into(),
        "istype" => "istype".into(),
        "as" => "as".into(),
        "meta" => "meta".into(),
        other => other.to_owned(),
    }
}
