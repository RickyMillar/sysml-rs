//! Expression evaluator.
//!
//! Recursive interpreter for ExprIR expressions.

#![allow(clippy::indexing_slicing)]
use super::ir::{BinOp, ExprIR, UnaryOp};
use super::stdlib::{
    eval_function, numeric_binop, numeric_cmp, promote_to_complex, promote_to_float,
    value_matches_type_name, value_to_bool, value_to_list, values_equal,
};
use super::{EvalContext, EvalResult, EvaluationError};
use sysml_core::physics::DimensionVector;
use sysml_core::ElementId;
use sysml_core::{Element, ElementKind, ModelGraph, Value};

/// Maximum recursion depth for expression evaluation.
const MAX_EVAL_DEPTH: usize = 128;

/// The expression evaluator.
///
/// Recursively evaluates an [`ExprIR`] expression tree in a given [`EvalContext`].
#[derive(Clone)]
pub struct ExpressionEvaluator {
    _private: (), // force use of ::new()
}

impl ExpressionEvaluator {
    /// Create a new evaluator.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Evaluate an expression in the given context.
    pub fn eval(&self, expr: &ExprIR, ctx: &EvalContext) -> EvalResult {
        self.eval_inner(expr, ctx, 0)
    }

    /// Internal recursive evaluator with depth tracking.
    fn eval_inner(&self, expr: &ExprIR, ctx: &EvalContext, depth: usize) -> EvalResult {
        if depth > MAX_EVAL_DEPTH {
            return Err(EvaluationError::RecursionLimit {
                max_depth: MAX_EVAL_DEPTH,
            });
        }

        match expr {
            // Literals
            ExprIR::LiteralInt(n) => Ok(Value::Int(*n)),
            ExprIR::LiteralReal(n) => Ok(Value::Float(*n)),
            ExprIR::LiteralQuantity {
                value,
                dimension,
                unit,
            } => Ok(Value::Quantity {
                value: *value,
                dimension: *dimension,
                unit: Some(unit.as_str().into()),
            }),
            ExprIR::LiteralBool(b) => Ok(Value::Bool(*b)),
            ExprIR::LiteralString(s) => Ok(Value::String(s.clone())),
            ExprIR::LiteralNull => Ok(Value::Null),

            // References
            ExprIR::FeatureRef(name) => {
                let val = ctx
                    .get(name)
                    .cloned()
                    .ok_or_else(|| EvaluationError::UndefinedVariable(name.clone()))?;
                self.finish_feature_value(name, val, ctx)
            }

            // RSC-2.3: compile-bound reference (design doc D-2.0.4). Context
            // names win — scoped views, RK4 stage bindings (the ODE RHS
            // binds intermediate stage values by name), and lambda/override
            // shadowing must see exactly what `FeatureRef` saw, which is
            // what keeps the RSC-2.B0 baselines byte-identical. The
            // by-`SlotId` read serves names absent from the context; it is
            // the path that retires the eval-time graph-wide same-name scan
            // at RSC-2.5.
            ExprIR::SlotRef { slot, name } => {
                // OPT #1 (runtime-hotpath-perf-plan): the slot-fast lane wins
                // first. It is populated ONLY in the ODE RHS scratch context,
                // and only for state-var slots the binder proved (no surviving
                // `FeatureRef` for the same name), so a hit is exactly the
                // current RK-stage value. A float never needs
                // `finish_feature_value` (that only resolves `Value::Ref`), so
                // returning it directly is byte-identical to the name-first
                // path that previously produced the same float. Fast-lane-first
                // (not name-first) is REQUIRED: the scratch base map may carry
                // a stale prior-tick value for this name, which the RHS no
                // longer overwrites for slotted vars. Every other context
                // leaves the lane empty, so this falls through unchanged.
                if let Some(f) = ctx.fast_slot(*slot) {
                    Ok(Value::Float(f))
                } else if let Some(val) = ctx.get(name) {
                    let val = val.clone();
                    self.finish_feature_value(name, val, ctx)
                } else if let Some(val) = ctx.get_slot(*slot) {
                    self.finish_feature_value(name, val, ctx)
                } else {
                    Err(EvaluationError::UndefinedVariable(name.clone()))
                }
            }

            ExprIR::FeatureChain(names) => self.eval_feature_chain(names, ctx),

            // RSC-2.3: chain with a compile-bound head. Follows the
            // `FeatureChain` path verbatim whenever the context can resolve
            // it (flat-key fast path, then the per-segment walk); the slot
            // serves the head only when the context has no binding for it.
            ExprIR::SlotChainHead { slot, names, bound } => {
                self.eval_slot_chain_head(*slot, names, *bound, ctx)
            }

            // Binary
            ExprIR::BinaryOp { op, left, right } => {
                let lv = self.eval_inner(left, ctx, depth + 1)?;
                // Short-circuit for logical operators
                match op {
                    BinOp::And => {
                        if !value_to_bool(&lv)? {
                            return Ok(Value::Bool(false));
                        }
                        let rv = self.eval_inner(right, ctx, depth + 1)?;
                        Ok(Value::Bool(value_to_bool(&rv)?))
                    }
                    BinOp::Or => {
                        if value_to_bool(&lv)? {
                            return Ok(Value::Bool(true));
                        }
                        let rv = self.eval_inner(right, ctx, depth + 1)?;
                        Ok(Value::Bool(value_to_bool(&rv)?))
                    }
                    BinOp::Implies => {
                        if !value_to_bool(&lv)? {
                            return Ok(Value::Bool(true));
                        }
                        let rv = self.eval_inner(right, ctx, depth + 1)?;
                        Ok(Value::Bool(value_to_bool(&rv)?))
                    }
                    // Type classification operators — need graph access
                    BinOp::IsType | BinOp::HasType | BinOp::As | BinOp::Meta => {
                        let rv = self.eval_inner(right, ctx, depth + 1)?;
                        self.eval_type_op(*op, &lv, &rv, ctx)
                    }
                    _ => {
                        let rv = self.eval_inner(right, ctx, depth + 1)?;
                        self.eval_binary(*op, &lv, &rv)
                    }
                }
            }

            // Unary
            ExprIR::UnaryOp { op, operand } => {
                let v = self.eval_inner(operand, ctx, depth + 1)?;
                self.eval_unary(*op, &v)
            }

            // Conditional
            ExprIR::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let cond = self.eval_inner(condition, ctx, depth + 1)?;
                if value_to_bool(&cond)? {
                    self.eval_inner(then_expr, ctx, depth + 1)
                } else {
                    self.eval_inner(else_expr, ctx, depth + 1)
                }
            }

            ExprIR::NullCoalescing { expr, default } => {
                let v = self.eval_inner(expr, ctx, depth + 1)?;
                if v == Value::Null {
                    self.eval_inner(default, ctx, depth + 1)
                } else {
                    Ok(v)
                }
            }

            // Collection operations
            ExprIR::Select {
                source,
                binding,
                predicate,
            } => self.eval_collection_filter(source, binding, predicate, ctx, true, depth),

            ExprIR::Reject {
                source,
                binding,
                predicate,
            } => self.eval_collection_filter(source, binding, predicate, ctx, false, depth),

            ExprIR::Collect {
                source,
                binding,
                transform,
            } => self.eval_collect(source, binding, transform, ctx, depth),

            ExprIR::ForAll {
                source,
                binding,
                predicate,
            } => self.eval_quantifier(source, binding, predicate, ctx, true, depth),

            ExprIR::Exists {
                source,
                binding,
                predicate,
            } => self.eval_quantifier(source, binding, predicate, ctx, false, depth),

            // Indexing (SysML uses 1-based indexing)
            ExprIR::Index { sequence, index } => {
                let seq_val = self.eval_inner(sequence, ctx, depth + 1)?;
                let idx_val = self.eval_inner(index, ctx, depth + 1)?;
                self.eval_index(&seq_val, &idx_val)
            }

            // Function calls: analysis IR evaluates standard-library functions only.
            // The execution runtime layers user-defined `calc def` lookup on top of
            // this lower crate when building executable contexts.
            // Function calls: try stdlib first, then calc def registry
            ExprIR::FunctionCall { name, args } => {
                let arg_vals: Result<Vec<Value>, _> = args
                    .iter()
                    .map(|a| self.eval_inner(a, ctx, depth + 1))
                    .collect();
                let arg_vals = arg_vals?;
                match eval_function(name, &arg_vals, ctx) {
                    Err(EvaluationError::UnknownFunction(_)) => {
                        // Fall back to calculation registry
                        if let Some(ref registry) = ctx.calculations {
                            if let Some(calc) = registry.get(name) {
                                let named_args: Vec<(String, Value)> = calc
                                    .parameters
                                    .iter()
                                    .zip(arg_vals.iter())
                                    .map(|(p, v)| (p.name.clone(), v.clone()))
                                    .collect();
                                // If more args than params, bind extras by position name
                                let named_args = if named_args.len() < arg_vals.len() {
                                    // Use free variables from the expression as fallback names
                                    let free = calc.result_expr.free_variables();
                                    let mut all: Vec<(String, Value)> = free
                                        .into_iter()
                                        .zip(arg_vals.into_iter())
                                        .map(|(n, v)| (n, v))
                                        .collect();
                                    // Override with named params where available
                                    for (p, v) in &named_args {
                                        if let Some(existing) = all.iter_mut().find(|(n, _)| n == p)
                                        {
                                            existing.1 = v.clone();
                                        } else {
                                            all.push((p.clone(), v.clone()));
                                        }
                                    }
                                    all
                                } else {
                                    named_args
                                };
                                return crate::calculations::evaluate_calculation(
                                    calc,
                                    &named_args,
                                    ctx,
                                );
                            }
                        }
                        Err(EvaluationError::UnknownFunction(name.clone()))
                    }
                    other => other,
                }
            }

            // Sequence construction
            ExprIR::Sequence(exprs) => {
                let vals: Result<Vec<Value>, _> = exprs
                    .iter()
                    .map(|e| self.eval_inner(e, ctx, depth + 1))
                    .collect();
                Ok(Value::List(vals?))
            }

            // Range (lower..upper inclusive, integer only)
            ExprIR::Range { lower, upper } => {
                let l = self.eval_inner(lower, ctx, depth + 1)?;
                let u = self.eval_inner(upper, ctx, depth + 1)?;
                self.eval_range(&l, &u)
            }

            // Classification / metadata access.
            //
            // Single `@` is the KerML classification operator (spec
            // KerML-r2025-04: grammar lists `@` alongside `istype`/`hastype`,
            // §8567 "filter conditions … test for the abstract syntax
            // metaclass of an element"). In the filter shorthand `@T` the
            // left operand is implicit (`self`), so this evaluates to a
            // Boolean: does the bound `self` element's abstract-syntax
            // metaclass conform to `T` (subtype-inclusive)? When `T` is not a
            // built-in metaclass it falls back to a user metadata/stereotype
            // test. Double `@@` (metaclassification → metaobject sequence) is
            // not needed by filters and is still unsupported.
            ExprIR::MetaAccess { operand, is_double } => {
                if *is_double {
                    return Err(EvaluationError::UnsupportedOperator(
                        "@@ (metaclassification requires metaobject support)".to_owned(),
                    ));
                }
                self.eval_classification(operand, ctx, depth)
            }

            // Tier 2: ConstructorCall — a payload/instance literal
            // `T(field = value, ...)`. Evaluate each named argument and
            // collect into a `Value::Map` keyed by field name, so a receiver
            // guard can read `payload.field` (member access already walks
            // `Value::Map` via the FeatureChain path). The type name is not
            // retained in the value — payload-TYPE conformance is a separate
            // concern (carried on the trigger, not the value). With no named
            // args the map is empty (the arg-name-carrying parser work is the
            // structured-payload follow-up; until then this is inert).
            ExprIR::ConstructorCall { named_args, .. } => {
                let mut fields = std::collections::BTreeMap::new();
                for (name, arg) in named_args {
                    let value = self.eval_inner(arg, ctx, depth + 1)?;
                    fields.insert(name.clone(), value);
                }
                Ok(Value::Map(fields))
            }
        }
    }

    // -- Private helpers ---------------------------------------------------

    /// Shared tail of `FeatureRef` / `SlotRef` evaluation: resolve a
    /// `Value::Ref` placeholder to the element's concrete value via the
    /// graph (literal extraction → computed-expression eval), pass every
    /// other value through unchanged.
    ///
    /// RSC-2.5: the graph-wide same-name fallback scan that used to run
    /// after these two steps is DELETED (design doc D-2.0.4 / §4 row
    /// RSC-2.5) — its RSC-2.3 hit counter stayed zero across the corpus
    /// and gates. A genuinely-unbound name now surfaces as the eval-time
    /// `UndefinedVariable` error below (and compile-time `RS003` for
    /// bound scopes), instead of being soft-served by an unrelated
    /// same-named element elsewhere in the graph.
    fn finish_feature_value(&self, name: &str, val: Value, ctx: &EvalContext) -> EvalResult {
        // Resolve Value::Ref to actual values using graph lookup
        if let Value::Ref(ref element_id) = val {
            if let Some(graph) = ctx.graph.as_ref() {
                if let Some(element) = graph.get_element(element_id) {
                    if let Some(resolved) = extract_element_value(element) {
                        return Ok(resolved);
                    }
                    if let Some(resolved) = try_eval_unresolved(element, self, ctx) {
                        return Ok(resolved);
                    }
                }
            }
            // Ref could not be resolved — attribute has no assigned value
            return Err(EvaluationError::UndefinedVariable(format!(
                "{} (attribute has no assigned value)",
                name,
            )));
        }
        Ok(val)
    }

    /// RSC-2.3: evaluate a [`ExprIR::SlotChainHead`]. Parity contract: when
    /// the context can resolve the chain (flat key or head name), behave
    /// exactly like [`ExprIR::FeatureChain`]; only when the head name is
    /// absent from the context is the bound slot consulted, after which the
    /// remaining tail segments are walked with the standard chain logic.
    fn eval_slot_chain_head(
        &self,
        slot: crate::slots::SlotId,
        names: &[String],
        bound: usize,
        ctx: &EvalContext,
    ) -> EvalResult {
        if names.is_empty() {
            return Ok(Value::Null);
        }
        // FeatureChain parity #1: flat-key fast path over the full chain.
        if names.len() >= 2 {
            let flat_key = names.join(".");
            if let Some(val) = ctx.get(&flat_key) {
                return Ok(val.clone());
            }
        }
        // FeatureChain parity #2: head name resolvable in the context —
        // identical per-segment walk.
        if ctx.get(&names[0]).is_some() {
            return self.eval_feature_chain(names, ctx);
        }
        // Head absent from the context: serve the bound head from the slot
        // store (master handle or the read-only propagated handle), then
        // walk the unbound tail. Falls back to the original chain walk —
        // and therefore the original error — when no handle is attached.
        let Some(head) = ctx.get_slot(slot) else {
            return self.eval_feature_chain(names, ctx);
        };
        let tail = names.get(bound..).unwrap_or(&[]);
        self.walk_chain_tail(head, tail, ctx)
    }

    #[allow(clippy::indexing_slicing)] // names[0] and names[1..] are safe: is_empty() checked above
    fn eval_feature_chain(&self, names: &[String], ctx: &EvalContext) -> EvalResult {
        if names.is_empty() {
            return Ok(Value::Null);
        }

        // Fast path: try looking up the full dotted path as a flat key first.
        // The orchestrator stores per-instance variables like "circuit1.loadCurrent"
        // as flat dotted keys in the shared context — these should resolve directly
        // without walking through a Ref chain.
        if names.len() >= 2 {
            let flat_key = names.join(".");
            if let Some(val) = ctx.get(&flat_key) {
                return Ok(val.clone());
            }
        }

        let current = ctx
            .get(&names[0])
            .cloned()
            .ok_or_else(|| EvaluationError::UndefinedVariable(names[0].clone()))?;
        self.walk_chain_tail(current, &names[1..], ctx)
    }

    /// Per-segment chain walk shared by `FeatureChain` and the
    /// `SlotChainHead` tail (RSC-2.3): Map projection, lazy `Value::Ref`
    /// graph navigation, intrinsic fields, redefinition values.
    fn walk_chain_tail(
        &self,
        mut current: Value,
        tail: &[String],
        ctx: &EvalContext,
    ) -> EvalResult {
        use sysml_core::resolution::scoping::chaining::{
            find_feature_type, resolve_with_feature_chaining,
        };
        use sysml_core::resolution::scoping::ScopedResolution;

        for name in tail {
            match current {
                Value::Map(ref map) => {
                    current = map
                        .get(name)
                        .cloned()
                        .ok_or_else(|| EvaluationError::UndefinedVariable(name.clone()))?;
                }
                Value::Ref(ref element_id) => {
                    // Lazy graph resolution: follow the element's type to find the named feature
                    let graph = ctx.graph.as_ref().ok_or_else(|| {
                        EvaluationError::TypeError(format!(
                            "cannot resolve `{}` on element ref (no graph)",
                            name
                        ))
                    })?;
                    // Element-intrinsic fields (kind/name/id) — project directly
                    // off the Element struct so filter expressions like
                    // `self.kind == "PartUsage"` work without a SysML feature
                    // by that name existing.
                    if let Some(intrinsic) = project_element_intrinsic(graph, element_id, name) {
                        current = intrinsic;
                        continue;
                    }
                    match resolve_with_feature_chaining(graph, element_id, name) {
                        ScopedResolution::Found(resolved_id) => {
                            // Try to extract a literal value from the resolved element
                            if let Some(element) = graph.get_element(&resolved_id) {
                                if let Some(val) = extract_element_value(element) {
                                    current = val;
                                } else if let Some(val) =
                                    find_redefinition_value(graph, element_id, name)
                                {
                                    current = val;
                                } else if let Some(val) = try_eval_unresolved(element, self, ctx) {
                                    // Computed attribute: evaluate its expression
                                    current = val;
                                } else {
                                    // No value available — return Ref for further chaining
                                    current = Value::Ref(resolved_id);
                                }
                            } else {
                                return Err(EvaluationError::UndefinedVariable(name.clone()));
                            }
                        }
                        _ => {
                            // Try direct children as fallback (for unresolved types)
                            let graph_ref = graph.as_ref();
                            let found = graph_ref
                                .children_of(element_id)
                                .find(|c| c.name.as_deref() == Some(name));
                            if let Some(child) = found {
                                current = resolve_element_value(child, self, ctx);
                            } else {
                                // Also check via type's children
                                if let Some(type_id) = find_feature_type(graph, element_id) {
                                    let found = graph_ref
                                        .children_of(&type_id)
                                        .find(|c| c.name.as_deref() == Some(name));
                                    if let Some(child) = found {
                                        current = resolve_element_value(child, self, ctx);
                                    } else {
                                        return Err(EvaluationError::UndefinedVariable(
                                            name.clone(),
                                        ));
                                    }
                                } else {
                                    return Err(EvaluationError::UndefinedVariable(name.clone()));
                                }
                            }
                        }
                    }
                }
                _ => {
                    return Err(EvaluationError::TypeError(format!(
                        "cannot access field `{}` on {:?}",
                        name, current
                    )));
                }
            }
        }
        Ok(current)
    }

    /// Evaluate collection indexing with SysML 1-based indices.
    #[allow(clippy::indexing_slicing)] // Index bounds validated above
    fn eval_index(&self, seq: &Value, idx: &Value) -> EvalResult {
        let items = value_to_list(seq)?;
        let i = match idx {
            Value::Int(n) => *n,
            _ => {
                return Err(EvaluationError::TypeError(
                    "index must be an integer".into(),
                ));
            }
        };
        // SysML uses 1-based indexing
        if i < 1 || i as usize > items.len() {
            return Err(EvaluationError::IndexOutOfBounds {
                index: i as usize,
                size: items.len(),
            });
        }
        Ok(items[(i - 1) as usize].clone())
    }

    /// Evaluate a range expression producing a list of integers.
    fn eval_range(&self, lower: &Value, upper: &Value) -> EvalResult {
        let l = match lower {
            Value::Int(n) => *n,
            _ => {
                return Err(EvaluationError::TypeError(
                    "range lower bound must be an integer".into(),
                ));
            }
        };
        let u = match upper {
            Value::Int(n) => *n,
            _ => {
                return Err(EvaluationError::TypeError(
                    "range upper bound must be an integer".into(),
                ));
            }
        };
        if l > u {
            return Ok(Value::List(Vec::new()));
        }
        let vals: Vec<Value> = (l..=u).map(Value::Int).collect();
        Ok(Value::List(vals))
    }

    fn eval_binary(&self, op: BinOp, left: &Value, right: &Value) -> EvalResult {
        // Complex arithmetic needs special handling for *, /, **
        let either_complex =
            matches!(left, Value::Complex { .. }) || matches!(right, Value::Complex { .. });
        // Quantity arithmetic needs dimensional analysis
        let either_quantity =
            matches!(left, Value::Quantity { .. }) || matches!(right, Value::Quantity { .. });

        if either_quantity && !either_complex {
            return self.eval_quantity_binary(op, left, right);
        }

        match op {
            // Arithmetic — checked integer operations
            BinOp::Add => numeric_binop(left, right, |a, b| a.checked_add(b), |a, b| a + b),
            BinOp::Subtract => numeric_binop(left, right, |a, b| a.checked_sub(b), |a, b| a - b),
            BinOp::Multiply if either_complex => {
                let (a_re, a_im) = promote_to_complex(left)?;
                let (b_re, b_im) = promote_to_complex(right)?;
                // (a + bi)(c + di) = (ac - bd) + (ad + bc)i
                Ok(Value::Complex {
                    re: a_re * b_re - a_im * b_im,
                    im: a_re * b_im + a_im * b_re,
                })
            }
            BinOp::Multiply => numeric_binop(left, right, |a, b| a.checked_mul(b), |a, b| a * b),
            BinOp::Divide if either_complex => {
                let (a_re, a_im) = promote_to_complex(left)?;
                let (b_re, b_im) = promote_to_complex(right)?;
                let denom = b_re * b_re + b_im * b_im;
                if denom == 0.0 {
                    return Err(EvaluationError::DivisionByZero);
                }
                // (a + bi)/(c + di) = ((ac + bd) + (bc - ad)i) / (c² + d²)
                Ok(Value::Complex {
                    re: (a_re * b_re + a_im * b_im) / denom,
                    im: (a_im * b_re - a_re * b_im) / denom,
                })
            }
            BinOp::Divide => {
                // Check for zero
                match (left, right) {
                    (Value::Int(_), Value::Int(0)) => Err(EvaluationError::DivisionByZero),
                    (_, Value::Float(f)) if *f == 0.0 => Err(EvaluationError::DivisionByZero),
                    _ => numeric_binop(left, right, |a, b| a.checked_div(b), |a, b| a / b),
                }
            }
            BinOp::Remainder => numeric_binop(left, right, |a, b| a.checked_rem(b), |a, b| a % b),
            BinOp::Power => {
                let (l, r) = promote_to_float(left, right)?;
                Ok(Value::Float(l.powf(r)))
            }

            // Comparison
            BinOp::Equal => Ok(Value::Bool(values_equal(left, right))),
            BinOp::NotEqual => Ok(Value::Bool(!values_equal(left, right))),
            BinOp::ReferenceEqual => Ok(Value::Bool(values_equal(left, right))),
            BinOp::ReferenceNotEqual => Ok(Value::Bool(!values_equal(left, right))),
            BinOp::LessThan => numeric_cmp(left, right, |a, b| a < b, |a, b| a < b),
            BinOp::LessEqual => numeric_cmp(left, right, |a, b| a <= b, |a, b| a <= b),
            BinOp::GreaterThan => numeric_cmp(left, right, |a, b| a > b, |a, b| a > b),
            BinOp::GreaterEqual => numeric_cmp(left, right, |a, b| a >= b, |a, b| a >= b),

            // Logical (short-circuit handled in eval)
            BinOp::And | BinOp::Or | BinOp::Implies => {
                unreachable!("short-circuit operators handled in eval()")
            }
            BinOp::Xor => {
                let lb = value_to_bool(left)?;
                let rb = value_to_bool(right)?;
                Ok(Value::Bool(lb ^ rb))
            }

            // Bitwise
            BinOp::BitAnd => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a & b)),
                _ => Err(EvaluationError::TypeError(
                    "bitwise AND requires integer operands".into(),
                )),
            },
            BinOp::BitOr => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a | b)),
                _ => Err(EvaluationError::TypeError(
                    "bitwise OR requires integer operands".into(),
                )),
            },

            // Classification operators now handled in eval_type_op (before eval_binary is called)
            BinOp::HasType | BinOp::IsType | BinOp::As | BinOp::Meta => {
                // Should never reach here — intercepted in BinaryOp match above
                Err(EvaluationError::UnsupportedOperator(format!(
                    "{:?} should be handled by eval_type_op",
                    op
                )))
            }
        }
    }

    /// Evaluate type classification operators (`istype`, `hastype`, `as`, `meta`).
    ///
    /// These require access to the model graph to resolve type relationships.
    /// - `istype`: true if the value's element is typed exactly as the given type
    /// - `hastype`: true if the value's element is typed as or specializes the given type
    /// - `as`: returns the value unchanged if it conforms to the type, Null otherwise
    /// - `meta`: returns the type name(s) of the value's element
    fn eval_type_op(
        &self,
        op: BinOp,
        left: &Value,
        right: &Value,
        ctx: &EvalContext,
    ) -> EvalResult {
        let graph = ctx.graph.as_deref();

        // Resolve the target type name from the RHS
        let type_name = match right {
            Value::String(s) => s.clone(),
            Value::Ref(id) => graph
                .and_then(|g| g.get_element(id))
                .and_then(|e| e.name.clone())
                .unwrap_or_else(|| format!("{}", id)),
            _ => {
                return Err(EvaluationError::TypeError(format!(
                    "type operator RHS must be a type name, got {:?}",
                    right
                )))
            }
        };

        // Resolve the LHS to an element ID (if possible)
        let element_id = match left {
            Value::Ref(id) => Some(id.clone()),
            _ => None,
        };

        match op {
            BinOp::IsType => {
                if let (Some(ref eid), Some(g)) = (&element_id, graph) {
                    Ok(Value::Bool(self.element_has_exact_type(g, eid, &type_name)))
                } else {
                    Ok(Value::Bool(value_matches_type_name(left, &type_name)))
                }
            }
            BinOp::HasType => {
                if let (Some(ref eid), Some(g)) = (&element_id, graph) {
                    Ok(Value::Bool(
                        self.element_conforms_to_type(g, eid, &type_name),
                    ))
                } else {
                    Ok(Value::Bool(value_matches_type_name(left, &type_name)))
                }
            }
            BinOp::As => {
                let conforms = if let (Some(ref eid), Some(g)) = (&element_id, graph) {
                    self.element_conforms_to_type(g, eid, &type_name)
                } else {
                    value_matches_type_name(left, &type_name)
                };
                if conforms {
                    Ok(left.clone())
                } else {
                    Ok(Value::Null)
                }
            }
            BinOp::Meta => {
                if let (Some(ref eid), Some(g)) = (&element_id, graph) {
                    let type_names = self.element_type_names(g, eid);
                    if type_names.is_empty() {
                        Ok(Value::Null)
                    } else if type_names.len() == 1 {
                        Ok(Value::String(type_names.into_iter().next().unwrap()))
                    } else {
                        Ok(Value::List(
                            type_names.into_iter().map(Value::String).collect(),
                        ))
                    }
                } else {
                    Ok(Value::String(left.type_name().to_string()))
                }
            }
            _ => unreachable!(),
        }
    }

    /// Check if an element is typed exactly as `type_name`.
    fn element_has_exact_type(
        &self,
        graph: &sysml_core::ModelGraph,
        element_id: &sysml_core::ElementId,
        type_name: &str,
    ) -> bool {
        // Check unresolvedTypeName property
        if let Some(element) = graph.get_element(element_id) {
            if let Some(Value::String(s)) = element.get_prop("unresolvedTypeName") {
                let name = s.rsplit("::").next().unwrap_or(s);
                if name == type_name {
                    return true;
                }
            }
        }
        // Check resolved FeatureTyping relationships
        for type_id in
            sysml_core::resolution::scoping::chaining::find_feature_types(graph, element_id)
        {
            if let Some(type_elem) = graph.get_element(&type_id) {
                if type_elem.name.as_deref() == Some(type_name) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if an element conforms to `type_name` (typed as or specializes).
    fn element_conforms_to_type(
        &self,
        graph: &sysml_core::ModelGraph,
        element_id: &sysml_core::ElementId,
        type_name: &str,
    ) -> bool {
        // Direct type match
        if self.element_has_exact_type(graph, element_id, type_name) {
            return true;
        }
        // Check element kind name matches
        if let Some(element) = graph.get_element(element_id) {
            let kind_name = format!("{:?}", element.kind);
            if kind_name == type_name {
                return true;
            }
        }
        // Check supertype chain via Specialization relationships
        for type_id in
            sysml_core::resolution::scoping::chaining::find_feature_types(graph, element_id)
        {
            if self.type_specializes(graph, &type_id, type_name, 0) {
                return true;
            }
        }
        false
    }

    /// Walk the specialization chain to check if a type ultimately specializes `target_name`.
    fn type_specializes(
        &self,
        graph: &sysml_core::ModelGraph,
        type_id: &sysml_core::ElementId,
        target_name: &str,
        depth: usize,
    ) -> bool {
        if depth > 20 {
            return false; // prevent infinite loops
        }
        if let Some(elem) = graph.get_element(type_id) {
            if elem.name.as_deref() == Some(target_name) {
                return true;
            }
        }
        // Walk outgoing Specialization relationships
        for rel in graph.outgoing(type_id) {
            if rel.kind == sysml_core::RelationshipKind::Specialize {
                if self.type_specializes(graph, &rel.target, target_name, depth + 1) {
                    return true;
                }
            }
        }
        false
    }

    /// Get all type names for an element.
    fn element_type_names(
        &self,
        graph: &sysml_core::ModelGraph,
        element_id: &sysml_core::ElementId,
    ) -> Vec<String> {
        let mut names = Vec::new();
        // From unresolvedTypeName
        if let Some(element) = graph.get_element(element_id) {
            if let Some(Value::String(s)) = element.get_prop("unresolvedTypeName") {
                let name = s.rsplit("::").next().unwrap_or(s);
                names.push(name.to_string());
            }
        }
        // From resolved FeatureTyping
        for type_id in
            sysml_core::resolution::scoping::chaining::find_feature_types(graph, element_id)
        {
            if let Some(type_elem) = graph.get_element(&type_id) {
                if let Some(ref name) = type_elem.name {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
        }
        names
    }

    /// Evaluate the single-`@` classification operator (filter shorthand
    /// `@T`, i.e. `self @ T`).
    ///
    /// Returns `Value::Bool`: `true` iff the `self`-bound element is
    /// classified by `T`. `T` is resolved first as a built-in abstract-syntax
    /// metaclass ([`ElementKind`], subtype-inclusive per the KerML `@`
    /// classification semantics); if it is not a known metaclass, it is
    /// treated as a user metadata/stereotype name and matched against the
    /// element's `MetadataUsage` children (reusing
    /// [`sysml_core::metadata::is_metadata_typed_as`], the same matcher
    /// `ViewFilter` stereotypes use). Errors only when no `self` element is
    /// bound or the operand is not a name — never a soft fall-through.
    fn eval_classification(
        &self,
        operand: &ExprIR,
        ctx: &EvalContext,
        depth: usize,
    ) -> EvalResult {
        let metaclass_name = self.classification_operand_name(operand, ctx, depth)?;

        // The classified subject is the implicit `self` (filter shorthand).
        let Some(graph) = ctx.graph.as_deref() else {
            return Err(EvaluationError::TypeError(
                "@ classification requires a model graph".to_owned(),
            ));
        };
        let self_elem = match ctx.get("self") {
            Some(Value::Ref(id)) => graph.get_element(id),
            _ => None,
        };
        let Some(elem) = self_elem else {
            return Err(EvaluationError::TypeError(
                "@ classification has no `self` element bound".to_owned(),
            ));
        };

        // Abstract-syntax metaclass classification (subtype-inclusive).
        if let Some(target) = ElementKind::from_str(&metaclass_name) {
            let k = elem.kind.clone();
            return Ok(Value::Bool(k == target || k.is_subtype_of(target)));
        }

        // Not a built-in metaclass → user metadata / stereotype test.
        let tagged = graph.children_of(&elem.id).any(|child| {
            child.kind == ElementKind::MetadataUsage
                && sysml_core::metadata::is_metadata_typed_as(graph, child, &metaclass_name)
        });
        Ok(Value::Bool(tagged))
    }

    /// Extract the metaclass/type name from an `@` operand. Handles the
    /// compiler's qualified-name lowering (`@SysML::RequirementUsage` →
    /// `LiteralString("RequirementUsage")`), bare identifiers, and any
    /// operand that evaluates to a name string/ref. Strips qualifier
    /// segments to the leaf name.
    fn classification_operand_name(
        &self,
        operand: &ExprIR,
        ctx: &EvalContext,
        depth: usize,
    ) -> Result<String, EvaluationError> {
        let raw = match operand {
            ExprIR::LiteralString(s) => s.clone(),
            ExprIR::FeatureRef(n) => n.clone(),
            other => match self.eval_inner(other, ctx, depth + 1)? {
                Value::String(s) => s,
                Value::Ref(id) => ctx
                    .graph
                    .as_deref()
                    .and_then(|g| g.get_element(&id))
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| id.to_string()),
                _ => {
                    return Err(EvaluationError::TypeError(
                        "@ operator expects a metaclass or type name".to_owned(),
                    ))
                }
            },
        };
        Ok(raw.rsplit("::").next().unwrap_or(&raw).trim().to_owned())
    }

    fn eval_unary(&self, op: UnaryOp, val: &Value) -> EvalResult {
        match op {
            UnaryOp::Negate => match val {
                Value::Int(n) => n
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or(EvaluationError::Overflow),
                Value::Float(f) => Ok(Value::Float(-f)),
                Value::Complex { re, im } => Ok(Value::Complex { re: -re, im: -im }),
                Value::Quantity {
                    value,
                    dimension,
                    unit,
                } => Ok(Value::Quantity {
                    value: -value,
                    dimension: *dimension,
                    unit: unit.clone(),
                }),
                _ => Err(EvaluationError::TypeError(format!(
                    "cannot negate {:?}",
                    val
                ))),
            },
            UnaryOp::Plus => match val {
                Value::Int(_)
                | Value::Float(_)
                | Value::Complex { .. }
                | Value::Quantity { .. } => Ok(val.clone()),
                _ => Err(EvaluationError::TypeError(format!("unary + on {:?}", val))),
            },
            UnaryOp::Not => {
                let b = value_to_bool(val)?;
                Ok(Value::Bool(!b))
            }
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                _ => Err(EvaluationError::TypeError(
                    "bitwise NOT requires integer operand".into(),
                )),
            },
        }
    }

    /// Extract (f64, DimensionVector, Option<unit>) from a numeric value.
    /// Non-Quantity numeric types get dimensionless (zero) dimension.
    fn extract_quantity(
        v: &Value,
    ) -> Result<(f64, DimensionVector, Option<String>), EvaluationError> {
        match v {
            Value::Quantity {
                value,
                dimension,
                unit,
            } => Ok((*value, *dimension, unit.clone())),
            Value::Float(f) => Ok((*f, DimensionVector::default(), None)),
            Value::Int(n) => Ok((*n as f64, DimensionVector::default(), None)),
            _ => Err(EvaluationError::TypeError(format!(
                "expected numeric or quantity, got {}",
                v.type_name()
            ))),
        }
    }

    /// Align the right-hand magnitude into the left-hand operand's unit so that
    /// same-dimension / different-scale quantities are operated on a common
    /// basis (RSC-5.1b: convert-before-operate).
    ///
    /// Returns the RHS magnitude expressed in `lu`'s unit when **both** operands
    /// carry resolvable, differing unit names of the same dimension; otherwise
    /// returns `rv` unchanged. The conversion reuses the single SI conversion
    /// home ([`convert_quantity`](super::units::convert_quantity)), so when the
    /// units are equal or absent the result is byte-identical to the bare
    /// magnitude (no behavioural change for the unit-less ISQ-base case).
    fn align_rhs(rv: f64, rd: &DimensionVector, ru: Option<&str>, lu: Option<&str>) -> f64 {
        match (lu, ru) {
            (Some(l), Some(r)) if l != r => super::units::convert_quantity(rv, rd, Some(r), l)
                .map(|(v, _, _)| v)
                .unwrap_or(rv),
            _ => rv,
        }
    }

    /// Quantity-aware binary operations with dimensional analysis.
    ///
    /// Rules:
    /// - Add/Subtract: dimensions must match; RHS is converted into the LHS unit
    ///   (scale-aware), result keeps the LHS dimension and unit
    /// - Multiply: dimensions add (exponents sum)
    /// - Divide: dimensions subtract (exponents difference)
    /// - Power: dimension scaled by integer exponent
    /// - Comparison: dimensions must match (hard error otherwise, Q4); RHS is
    ///   converted into the LHS unit before comparing magnitudes
    fn eval_quantity_binary(&self, op: BinOp, left: &Value, right: &Value) -> EvalResult {
        let (lv, ld, lu) = Self::extract_quantity(left)?;
        let (rv, rd, ru) = Self::extract_quantity(right)?;

        match op {
            BinOp::Add | BinOp::Subtract => {
                // Dimensions must match for addition/subtraction.
                // Exception: one operand is dimensionless (e.g., adding 0 to a quantity).
                if ld != rd && !ld.is_zero() && !rd.is_zero() {
                    return Err(EvaluationError::Runtime(format!(
                        "dimension mismatch: cannot {} {} and {}",
                        if matches!(op, BinOp::Add) {
                            "add"
                        } else {
                            "subtract"
                        },
                        ld,
                        rd
                    )));
                }
                let dim = if ld.is_zero() { rd } else { ld };
                // Convert RHS into the LHS unit before operating (scale-aware).
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                let result = if matches!(op, BinOp::Add) {
                    lv + rv
                } else {
                    lv - rv
                };
                // Prefer the unit name from the quantity operand
                let unit = lu.or(ru);
                Ok(Value::Quantity {
                    value: result,
                    dimension: dim,
                    unit,
                })
            }
            BinOp::Multiply => {
                let dim = ld + rd;
                let unit = if rd.is_zero() {
                    lu // scalar * quantity → keep quantity's unit
                } else if ld.is_zero() {
                    ru // quantity * scalar → keep quantity's unit
                } else {
                    None // derived unit — no simple name
                };
                Ok(Value::Quantity {
                    value: lv * rv,
                    dimension: dim,
                    unit,
                })
            }
            BinOp::Divide => {
                if rv == 0.0 {
                    return Err(EvaluationError::DivisionByZero);
                }
                let dim = ld - rd;
                let unit = if rd.is_zero() {
                    lu // quantity / scalar → keep quantity's unit
                } else {
                    None // derived unit
                };
                Ok(Value::Quantity {
                    value: lv / rv,
                    dimension: dim,
                    unit,
                })
            }
            BinOp::Power => {
                // Exponent must be dimensionless
                if !rd.is_zero() {
                    return Err(EvaluationError::Runtime(
                        "exponent must be dimensionless".into(),
                    ));
                }
                let exp_int = rv as i8;
                let dim = DimensionVector::new(
                    ld.length * exp_int,
                    ld.mass * exp_int,
                    ld.time * exp_int,
                    ld.current * exp_int,
                    ld.temperature * exp_int,
                    ld.amount * exp_int,
                    ld.luminosity * exp_int,
                );
                Ok(Value::Quantity {
                    value: lv.powf(rv),
                    dimension: dim,
                    unit: None,
                })
            }
            BinOp::Remainder => {
                if ld != rd && !ld.is_zero() && !rd.is_zero() {
                    return Err(EvaluationError::Runtime(format!(
                        "dimension mismatch: cannot take remainder of {} and {}",
                        ld, rd
                    )));
                }
                let dim = if ld.is_zero() { rd } else { ld };
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                Ok(Value::Quantity {
                    value: lv % rv,
                    dimension: dim,
                    unit: lu.or(ru),
                })
            }
            // Comparison operators — dimensions must match (Q4: hard error on
            // incompatible dimensions; previously a silent bare-magnitude
            // compare). RHS is converted into the LHS unit before comparing.
            BinOp::LessThan | BinOp::LessEqual | BinOp::GreaterThan | BinOp::GreaterEqual => {
                if ld != rd && !ld.is_zero() && !rd.is_zero() {
                    return Err(EvaluationError::Runtime(format!(
                        "dimension mismatch: cannot compare {} and {}",
                        ld, rd
                    )));
                }
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                let result = match op {
                    BinOp::LessThan => lv < rv,
                    BinOp::LessEqual => lv <= rv,
                    BinOp::GreaterThan => lv > rv,
                    _ => lv >= rv,
                };
                Ok(Value::Bool(result))
            }
            BinOp::Equal => {
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                Ok(Value::Bool(lv == rv && ld == rd))
            }
            BinOp::NotEqual => {
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                Ok(Value::Bool(lv != rv || ld != rd))
            }
            BinOp::ReferenceEqual => {
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                Ok(Value::Bool(lv == rv && ld == rd))
            }
            BinOp::ReferenceNotEqual => {
                let rv = Self::align_rhs(rv, &rd, ru.as_deref(), lu.as_deref());
                Ok(Value::Bool(lv != rv || ld != rd))
            }
            // Logical / bitwise — not meaningful on quantities, fall through to f64
            _ => {
                let fl = Value::Float(lv);
                let fr = Value::Float(rv);
                self.eval_binary(op, &fl, &fr)
            }
        }
    }

    fn eval_collection_filter(
        &self,
        source: &ExprIR,
        binding: &str,
        predicate: &ExprIR,
        ctx: &EvalContext,
        keep_when_true: bool,
        depth: usize,
    ) -> EvalResult {
        let source_val = self.eval_inner(source, ctx, depth + 1)?;
        let items = value_to_list(&source_val)?;
        let mut result = Vec::new();
        for item in items {
            let child_ctx = ctx.child_with(binding, item.clone());
            let pred_val = self.eval_inner(predicate, &child_ctx, depth + 1)?;
            let matches = value_to_bool(&pred_val)?;
            if matches == keep_when_true {
                result.push(item.clone());
            }
        }
        Ok(Value::List(result))
    }

    fn eval_collect(
        &self,
        source: &ExprIR,
        binding: &str,
        transform: &ExprIR,
        ctx: &EvalContext,
        depth: usize,
    ) -> EvalResult {
        let source_val = self.eval_inner(source, ctx, depth + 1)?;
        let items = value_to_list(&source_val)?;
        let mut result = Vec::new();
        for item in items {
            let child_ctx = ctx.child_with(binding, item.clone());
            let mapped = self.eval_inner(transform, &child_ctx, depth + 1)?;
            result.push(mapped);
        }
        Ok(Value::List(result))
    }

    fn eval_quantifier(
        &self,
        source: &ExprIR,
        binding: &str,
        predicate: &ExprIR,
        ctx: &EvalContext,
        is_for_all: bool,
        depth: usize,
    ) -> EvalResult {
        let source_val = self.eval_inner(source, ctx, depth + 1)?;
        let items = value_to_list(&source_val)?;
        for item in items {
            let child_ctx = ctx.child_with(binding, item.clone());
            let pred_val = self.eval_inner(predicate, &child_ctx, depth + 1)?;
            let matches = value_to_bool(&pred_val)?;
            if is_for_all && !matches {
                return Ok(Value::Bool(false));
            }
            if !is_for_all && matches {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(is_for_all))
    }
}

impl Default for ExpressionEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Graph-aware helpers for lazy feature chain resolution
// ---------------------------------------------------------------------------

/// Resolve an element to a Value: try literal extraction, then computed eval,
/// then fall back to Ref for further chaining.
fn resolve_element_value(
    element: &Element,
    evaluator: &ExpressionEvaluator,
    ctx: &super::EvalContext,
) -> Value {
    if let Some(val) = extract_element_value(element) {
        return val;
    }
    if let Some(val) = try_eval_unresolved(element, evaluator, ctx) {
        return val;
    }
    Value::Ref(element.id.clone())
}

/// Resolve a `Value::Ref` sentinel to the element's concrete live value.
///
/// Mirrors the on-demand resolution the evaluator does when it encounters
/// a `Value::Ref` via `FeatureRef` at evaluation time: look up the element
/// by id, try literal-property extraction, then fall back to compiling and
/// evaluating the element's expression subtree. Returns `None` when the
/// element isn't present (dangling id), isn't resolvable from literals,
/// or its expression fails to compile/evaluate.
///
/// Callers: snapshot projection (see `snapshot_view::normalize_with`) uses
/// this to surface live values for attributes whose context binding is a
/// lazy Ref sentinel, so the UI doesn't show "—" for attributes that are
/// expression-resolvable. Per-call cost is bounded by expression-AST
/// traversal; hot paths should prefer [`resolve_ref_value_cached`] to
/// share a compile-result cache across ticks.
pub fn resolve_ref_value(id: &ElementId, ctx: &EvalContext) -> Option<Value> {
    let graph = ctx.graph.as_ref()?;
    let element = graph.get_element(id)?;
    if let Some(v) = extract_element_value(element) {
        return Some(v);
    }
    try_eval_unresolved(element, &ExpressionEvaluator::new(), ctx)
}

/// Cached variant of [`resolve_ref_value`].
///
/// Hot-path version for callers that resolve the same Ref ids across many
/// ticks (snapshot projection per-tick). The caller holds an
/// `Arc<Mutex<RefResolveCache>>`; we consult it for a pre-compiled
/// [`ExprIR`] before re-walking the element's AST. Misses populate the
/// cache with `Some(Arc<ExprIR>)` on success or `None` so subsequent
/// lookups for non-compilable elements short-circuit without re-walking
/// the graph.
///
/// The cache keys by `ElementId` alone — the IR only changes when the
/// model is re-parsed. Per ADR-011 §6 (S3.T14) the cache lifetime is
/// snapshot-scoped (tied to the elaborated graph revision via salsa),
/// not session-scoped. Different orchestrators on the same revision
/// share the populated cache.
///
/// The mutex guard is dropped before evaluation so the compile-result
/// lookup doesn't block sibling resolvers behind a per-tick eval walk.
pub fn resolve_ref_value_cached(
    id: &ElementId,
    ctx: &EvalContext,
    cache: &std::sync::Mutex<RefResolveCache>,
) -> Option<Value> {
    let graph = ctx.graph.as_ref()?;
    let element = graph.get_element(id)?;
    if let Some(v) = extract_element_value(element) {
        return Some(v);
    }
    // Cached compile — `None` sentinel means "we tried and it failed".
    // Clone the inner `Option<Arc<ExprIR>>` while the guard is held so
    // we can drop the lock before running the (potentially deep)
    // evaluator on the IR.
    let cached: Option<std::sync::Arc<super::ExprIR>> = {
        let mut guard = cache.lock().ok()?;
        guard
            .entry(id.clone())
            .or_insert_with(|| {
                super::compile_expression(element, graph)
                    .ok()
                    .map(std::sync::Arc::new)
            })
            .clone()
    };
    let ir = cached?;
    ExpressionEvaluator::new().eval(&ir, ctx).ok()
}

/// Shared compile-result cache keyed by element id. Populated lazily by
/// [`resolve_ref_value_cached`]; `None` values mark elements whose
/// expression subtree failed to compile (non-expression attributes,
/// malformed ASTs) so repeated misses short-circuit.
pub type RefResolveCache =
    std::collections::HashMap<ElementId, Option<std::sync::Arc<super::ExprIR>>>;

/// Re-entrant depth cap for [`try_eval_unresolved`]. Legitimate attribute
/// lookups are at most 2-3 levels deep; a `Value::Ref` cycle manifests as
/// dozens of consecutive frames, far above any honest evaluation.
const MAX_REF_RESOLVE_DEPTH: usize = 16;

thread_local! {
    /// Re-entry counter for [`try_eval_unresolved`]. Independent of the
    /// per-evaluation `depth` parameter: it guards re-entry across the
    /// `try_eval_unresolved` → `evaluator.eval` → `compile_expression`
    /// boundary, which otherwise resets per-pass depth to zero and lets
    /// `Value::Ref` cycles (notably ISQ power-factor tuples on stdlib-merged
    /// graphs) loop indefinitely.
    static REF_RESOLVE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Try to evaluate an element's expression subtree (AST-first) as a Value.
///
/// This enables computed attributes like:
///   `attribute minTripMultiple = if breakerType == BreakerType::C ? 5 else 10;`
/// Prefers the parser-emitted AST subtree when the evaluator has a graph in
/// its context.
///
/// Recursion across the `compile_expression` → `eval` boundary is bounded by
/// the thread-local [`REF_RESOLVE_DEPTH`] counter, capped at
/// [`MAX_REF_RESOLVE_DEPTH`]. Without it, `Value::Ref` cycles through a
/// stdlib-merged graph re-enter this function indefinitely. The analysis-ir
/// evaluator previously lacked this guard while sysml-runtime had it —
/// backported here (AUDIT-2026-06-01 WS1 Step 0).
fn try_eval_unresolved(
    element: &Element,
    evaluator: &ExpressionEvaluator,
    ctx: &super::EvalContext,
) -> Option<Value> {
    let entered = REF_RESOLVE_DEPTH.with(|c| {
        let d = c.get();
        if d >= MAX_REF_RESOLVE_DEPTH {
            return false;
        }
        c.set(d + 1);
        true
    });
    if !entered {
        return None;
    }
    let result = (|| {
        let graph = ctx.graph.as_ref()?;
        let ir = super::compile_expression(element, graph).ok()?;
        evaluator.eval(&ir, ctx).ok()
    })();
    REF_RESOLVE_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    result
}

/// Try to extract a literal value from an element's properties.
///
/// Checks `value`, `default`, and falls back to parsing the legacy
/// `unresolved_value` string prop with unit stripping (`"300 [V]"` → 300)
/// and enum reference parsing (`"BreakerType::C"` → `"C"`). The AST path is
/// handled by `try_eval_unresolved`; this function only services literal-
/// typed attributes whose parser emits a non-expression typed value.
/// Project an Element-intrinsic field name (`kind`, `name`, `id`) off a
/// `Value::Ref`. SysML features cannot legally collide with these reserved
/// names because the intrinsic fields belong to the meta-model, not the
/// user-defined namespace — so this short-circuit can run before SysML
/// feature-chaining without ambiguity.
///
/// Returns `None` when `name` is not an intrinsic field; the caller then
/// falls back to feature-chaining resolution.
fn project_element_intrinsic(
    graph: &sysml_core::ModelGraph,
    element_id: &sysml_core::ElementId,
    name: &str,
) -> Option<Value> {
    let element = graph.get_element(element_id)?;
    match name {
        "kind" => Some(Value::String(format!("{:?}", element.kind))),
        "name" => element.name.as_ref().map(|n| Value::String(n.clone())),
        "id" => Some(Value::String(element.id.to_string())),
        _ => None,
    }
}

fn extract_element_value(element: &Element) -> Option<Value> {
    // Check "value" property
    if let Some(val) = element.get_prop("value") {
        match val {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_) => {
                return Some(val.clone());
            }
            _ => {}
        }
    }
    // Check "default" property
    if let Some(val) = element.get_prop("default") {
        match val {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_) => {
                return Some(val.clone());
            }
            _ => {}
        }
    }
    // Note: `unresolved_value` is no longer written by either parser
    // (removed in Phase 6D). The value/default paths above are sufficient
    // for literal extraction.
    None
}

// RSC-2.5: the graph-wide same-name fallback scan
// (`find_value_by_name_excluding`) and its RSC-2.3 telemetry counter
// (`name_scan_hits` / `reset_name_scan_hits`) were DELETED here — the
// counter stayed zero across the corpus since RSC-2.3, which was the
// design doc's deletion gate (§4 row RSC-2.5, risk "dynamic-name
// semantics"). Misses now surface as eval-time `UndefinedVariable`.

/// Find a redefinition value for a named feature on an element.
///
/// Handles the pattern where `attribute :>> featureName default value;` creates
/// a nameless child with a Redefinition pointing to the feature.
fn find_redefinition_value(graph: &ModelGraph, parent_id: &ElementId, name: &str) -> Option<Value> {
    for child in graph.children_of(parent_id) {
        if child.name.is_some() {
            continue; // Only check nameless children
        }
        // Check if this child has a Redefinition pointing to the named feature
        for grandchild in graph.children_of(&child.id) {
            if grandchild.kind == ElementKind::Redefinition {
                if grandchild
                    .get_prop("unresolved_redefinedFeature")
                    .and_then(|v| v.as_str())
                    == Some(name)
                {
                    return extract_element_value(&child);
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::ir::{BinOp, ExprIR, UnaryOp};
    use std::collections::BTreeMap;

    fn evaluator() -> ExpressionEvaluator {
        ExpressionEvaluator::new()
    }

    fn empty_ctx() -> EvalContext {
        EvalContext::new()
    }

    // -- Recursion depth limit ------------------------------------------------

    #[test]
    fn recursion_limit_deeply_nested_unary() {
        // Build 200 levels of nested unary-plus: +(+(+(+( ... 1 ... ))))
        let mut expr = ExprIR::LiteralInt(1);
        for _ in 0..200 {
            expr = ExprIR::UnaryOp {
                op: UnaryOp::Plus,
                operand: Box::new(expr),
            };
        }
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(
            matches!(
                result,
                Err(EvaluationError::RecursionLimit { max_depth: 128 })
            ),
            "expected RecursionLimit, got {:?}",
            result
        );
    }

    #[test]
    fn recursion_limit_nested_conditionals() {
        // Build deeply nested conditionals: if true ? (if true ? ... else 0) else 0
        let mut expr = ExprIR::LiteralInt(1);
        for _ in 0..200 {
            expr = ExprIR::Conditional {
                condition: Box::new(ExprIR::LiteralBool(true)),
                then_expr: Box::new(expr),
                else_expr: Box::new(ExprIR::LiteralInt(0)),
            };
        }
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(
            matches!(result, Err(EvaluationError::RecursionLimit { .. })),
            "expected RecursionLimit, got {:?}",
            result
        );
    }

    // -- Ref-cycle re-entry guard (AUDIT-2026-06-01 WS1 Step 0) ---------------
    //
    // A `Value::Ref` cycle through a stdlib-merged graph would otherwise loop
    // forever: try_eval_unresolved → eval → resolve Ref → try_eval_unresolved.
    // The thread-local REF_RESOLVE_DEPTH counter caps re-entry at
    // MAX_REF_RESOLVE_DEPTH and must stay balanced across every call.

    #[test]
    fn ref_resolve_guard_bails_at_depth_cap() {
        let el = Element::new_with_kind(ElementKind::AttributeUsage);
        let ctx = EvalContext::new();
        // Pretend we are already MAX deep in a Ref-resolution chain.
        REF_RESOLVE_DEPTH.with(|c| c.set(MAX_REF_RESOLVE_DEPTH));
        let out = try_eval_unresolved(&el, &evaluator(), &ctx);
        assert_eq!(out, None, "guard must short-circuit at the depth cap");
        // The bail path must leave the counter untouched (no spurious decrement).
        assert_eq!(
            REF_RESOLVE_DEPTH.with(|c| c.get()),
            MAX_REF_RESOLVE_DEPTH,
            "bail path must not mutate the counter"
        );
        REF_RESOLVE_DEPTH.with(|c| c.set(0)); // restore for sibling tests on this thread
    }

    #[test]
    fn ref_resolve_guard_balances_counter() {
        let el = Element::new_with_kind(ElementKind::AttributeUsage);
        let ctx = EvalContext::new();
        let before = REF_RESOLVE_DEPTH.with(|c| c.get());
        let _ = try_eval_unresolved(&el, &evaluator(), &ctx);
        assert_eq!(
            REF_RESOLVE_DEPTH.with(|c| c.get()),
            before,
            "entry increment must be matched by an exit decrement"
        );
    }

    // -- Feature chain edge cases ---------------------------------------------

    #[test]
    fn feature_chain_empty_returns_null() {
        let expr = ExprIR::FeatureChain(vec![]);
        let result = evaluator().eval(&expr, &empty_ctx()).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn feature_chain_single_element() {
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Int(42));
        let expr = ExprIR::FeatureChain(vec!["x".into()]);
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn feature_chain_nested_map() {
        let mut inner = BTreeMap::new();
        inner.insert("c".to_string(), Value::Int(99));
        let mut outer = BTreeMap::new();
        outer.insert("b".to_string(), Value::Map(inner));
        let mut ctx = EvalContext::new();
        ctx.set("a", Value::Map(outer));
        let expr = ExprIR::FeatureChain(vec!["a".into(), "b".into(), "c".into()]);
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Int(99));
    }

    #[test]
    fn feature_chain_missing_first_name() {
        let expr = ExprIR::FeatureChain(vec!["nonexistent".into()]);
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(
            matches!(result, Err(EvaluationError::UndefinedVariable(ref name)) if name == "nonexistent")
        );
    }

    #[test]
    fn feature_chain_non_map_intermediate() {
        let mut ctx = EvalContext::new();
        ctx.set("a", Value::Int(5));
        let expr = ExprIR::FeatureChain(vec!["a".into(), "b".into()]);
        let result = evaluator().eval(&expr, &ctx);
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    // -- Quantifier edge cases ------------------------------------------------

    #[test]
    fn forall_on_empty_list_returns_true() {
        let mut ctx = EvalContext::new();
        ctx.set("items", Value::List(vec![]));
        let expr = ExprIR::ForAll {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::LiteralBool(false)),
        };
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn exists_on_empty_list_returns_false() {
        let mut ctx = EvalContext::new();
        ctx.set("items", Value::List(vec![]));
        let expr = ExprIR::Exists {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::LiteralBool(true)),
        };
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn forall_mixed_values() {
        // forAll over [true, false, true] with identity predicate → false
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
            ]),
        );
        let expr = ExprIR::ForAll {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::FeatureRef("x".into())),
        };
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn exists_finds_matching_element() {
        // exists over [false, false, true] with identity → true
        let mut ctx = EvalContext::new();
        ctx.set(
            "items",
            Value::List(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
            ]),
        );
        let expr = ExprIR::Exists {
            source: Box::new(ExprIR::FeatureRef("items".into())),
            binding: "x".into(),
            predicate: Box::new(ExprIR::FeatureRef("x".into())),
        };
        let result = evaluator().eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    // -- BinOp type-mismatched operands ---------------------------------------

    #[test]
    fn add_string_and_int_is_type_error() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::Add,
            left: Box::new(ExprIR::LiteralString("hello".into())),
            right: Box::new(ExprIR::LiteralInt(1)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn less_than_string_and_int_is_type_error() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::LessThan,
            left: Box::new(ExprIR::LiteralString("a".into())),
            right: Box::new(ExprIR::LiteralInt(1)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn bitand_with_float_is_type_error() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::BitAnd,
            left: Box::new(ExprIR::LiteralReal(1.0)),
            right: Box::new(ExprIR::LiteralInt(2)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn bitor_with_bool_is_type_error() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::BitOr,
            left: Box::new(ExprIR::LiteralBool(true)),
            right: Box::new(ExprIR::LiteralInt(1)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    // -- UnaryOp on wrong types -----------------------------------------------

    #[test]
    fn negate_string_is_type_error() {
        let expr = ExprIR::UnaryOp {
            op: UnaryOp::Negate,
            operand: Box::new(ExprIR::LiteralString("hello".into())),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn not_on_int_is_type_error() {
        let expr = ExprIR::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(ExprIR::LiteralInt(42)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn bitnot_on_string_is_type_error() {
        let expr = ExprIR::UnaryOp {
            op: UnaryOp::BitNot,
            operand: Box::new(ExprIR::LiteralString("hello".into())),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    #[test]
    fn unary_plus_on_bool_is_type_error() {
        let expr = ExprIR::UnaryOp {
            op: UnaryOp::Plus,
            operand: Box::new(ExprIR::LiteralBool(true)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    // -- Division by zero -----------------------------------------------------

    #[test]
    fn division_by_zero_int() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::Divide,
            left: Box::new(ExprIR::LiteralInt(10)),
            right: Box::new(ExprIR::LiteralInt(0)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::DivisionByZero)));
    }

    #[test]
    fn division_by_zero_float() {
        let expr = ExprIR::BinaryOp {
            op: BinOp::Divide,
            left: Box::new(ExprIR::LiteralReal(10.0)),
            right: Box::new(ExprIR::LiteralReal(0.0)),
        };
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(matches!(result, Err(EvaluationError::DivisionByZero)));
    }

    // -- Null/missing value propagation ---------------------------------------

    #[test]
    fn null_coalescing_propagates_non_null() {
        let expr = ExprIR::NullCoalescing {
            expr: Box::new(ExprIR::LiteralInt(5)),
            default: Box::new(ExprIR::LiteralInt(99)),
        };
        let result = evaluator().eval(&expr, &empty_ctx()).unwrap();
        assert_eq!(result, Value::Int(5));
    }

    #[test]
    fn null_coalescing_falls_through_on_null() {
        let expr = ExprIR::NullCoalescing {
            expr: Box::new(ExprIR::LiteralNull),
            default: Box::new(ExprIR::LiteralInt(99)),
        };
        let result = evaluator().eval(&expr, &empty_ctx()).unwrap();
        assert_eq!(result, Value::Int(99));
    }

    #[test]
    fn undefined_variable_is_error() {
        let expr = ExprIR::FeatureRef("unknown".into());
        let result = evaluator().eval(&expr, &empty_ctx());
        assert!(
            matches!(result, Err(EvaluationError::UndefinedVariable(ref name)) if name == "unknown")
        );
    }

    // -- Index edge cases -----------------------------------------------------

    #[test]
    fn index_out_of_bounds_zero() {
        let mut ctx = EvalContext::new();
        ctx.set("arr", Value::List(vec![Value::Int(1), Value::Int(2)]));
        // SysML is 1-based, so 0 is out of bounds
        let expr = ExprIR::Index {
            sequence: Box::new(ExprIR::FeatureRef("arr".into())),
            index: Box::new(ExprIR::LiteralInt(0)),
        };
        let result = evaluator().eval(&expr, &ctx);
        assert!(matches!(
            result,
            Err(EvaluationError::IndexOutOfBounds { .. })
        ));
    }

    #[test]
    fn index_out_of_bounds_too_large() {
        let mut ctx = EvalContext::new();
        ctx.set("arr", Value::List(vec![Value::Int(1)]));
        let expr = ExprIR::Index {
            sequence: Box::new(ExprIR::FeatureRef("arr".into())),
            index: Box::new(ExprIR::LiteralInt(5)),
        };
        let result = evaluator().eval(&expr, &ctx);
        assert!(matches!(
            result,
            Err(EvaluationError::IndexOutOfBounds { .. })
        ));
    }

    #[test]
    fn index_non_integer_is_type_error() {
        let mut ctx = EvalContext::new();
        ctx.set("arr", Value::List(vec![Value::Int(1)]));
        let expr = ExprIR::Index {
            sequence: Box::new(ExprIR::FeatureRef("arr".into())),
            index: Box::new(ExprIR::LiteralString("one".into())),
        };
        let result = evaluator().eval(&expr, &ctx);
        assert!(matches!(result, Err(EvaluationError::TypeError(_))));
    }

    // -----------------------------------------------------------------------
    // Quantity arithmetic with dimensional analysis
    // -----------------------------------------------------------------------

    use sysml_core::physics::DimensionVector;

    fn length() -> DimensionVector {
        DimensionVector::new(1, 0, 0, 0, 0, 0, 0)
    }
    fn time_d() -> DimensionVector {
        DimensionVector::new(0, 0, 1, 0, 0, 0, 0)
    }
    fn velocity() -> DimensionVector {
        DimensionVector::new(1, 0, -1, 0, 0, 0, 0)
    }

    fn qty(v: f64, dim: DimensionVector, unit: &str) -> Value {
        Value::Quantity {
            value: v,
            dimension: dim,
            unit: Some(unit.to_string()),
        }
    }

    /// Helper: build a binary op expression from context variables.
    fn binop_vars(op: BinOp, lvar: &str, rvar: &str) -> ExprIR {
        ExprIR::BinaryOp {
            op,
            left: Box::new(ExprIR::FeatureRef(lvar.into())),
            right: Box::new(ExprIR::FeatureRef(rvar.into())),
        }
    }

    #[test]
    fn quantity_add_same_dimension() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(3.0, length(), "m"));
        ctx.set("b", qty(2.0, length(), "m"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Add, "a", "b"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value, dimension, ..
            } => {
                assert!((value - 5.0).abs() < 1e-10);
                assert_eq!(*dimension, length());
            }
            _ => panic!("expected Quantity, got {:?}", result),
        }
    }

    #[test]
    fn quantity_subtract_same_dimension() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(10.0, length(), "m"));
        ctx.set("b", qty(3.0, length(), "m"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Subtract, "a", "b"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity { value, .. } => assert!((value - 7.0).abs() < 1e-10),
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_add_dimension_mismatch() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(3.0, length(), "m"));
        ctx.set("b", qty(2.0, time_d(), "s"));
        let result = evaluator().eval(&binop_vars(BinOp::Add, "a", "b"), &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn quantity_multiply_dimensions_add() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(10.0, length(), "m"));
        ctx.set(
            "b",
            Value::Quantity {
                value: 0.5,
                dimension: DimensionVector::new(0, 0, -1, 0, 0, 0, 0),
                unit: Some("1/s".to_string()),
            },
        );
        let result = evaluator()
            .eval(&binop_vars(BinOp::Multiply, "a", "b"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value, dimension, ..
            } => {
                assert!((value - 5.0).abs() < 1e-10);
                assert_eq!(*dimension, velocity());
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_divide_dimensions_subtract() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(100.0, length(), "m"));
        ctx.set("b", qty(10.0, time_d(), "s"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Divide, "a", "b"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value, dimension, ..
            } => {
                assert!((value - 10.0).abs() < 1e-10);
                assert_eq!(*dimension, velocity());
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_scalar_multiply() {
        let mut ctx = EvalContext::new();
        ctx.set("k", Value::Float(2.0));
        ctx.set("x", qty(5.0, length(), "m"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Multiply, "k", "x"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                assert!((value - 10.0).abs() < 1e-10);
                assert_eq!(*dimension, length());
                assert_eq!(unit.as_deref(), Some("m"));
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_negate() {
        let mut ctx = EvalContext::new();
        ctx.set("x", qty(5.0, length(), "m"));
        let expr = ExprIR::UnaryOp {
            op: UnaryOp::Negate,
            operand: Box::new(ExprIR::FeatureRef("x".into())),
        };
        let result = evaluator().eval(&expr, &ctx).unwrap();
        match &result {
            Value::Quantity { value, .. } => assert!((value - (-5.0)).abs() < 1e-10),
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_power() {
        let mut ctx = EvalContext::new();
        ctx.set("x", qty(3.0, length(), "m"));
        ctx.set("n", Value::Float(2.0));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Power, "x", "n"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value, dimension, ..
            } => {
                assert!((value - 9.0).abs() < 1e-10);
                assert_eq!(*dimension, DimensionVector::new(2, 0, 0, 0, 0, 0, 0));
            }
            _ => panic!("expected Quantity"),
        }
    }

    #[test]
    fn quantity_comparison() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(3.0, length(), "m"));
        ctx.set("b", qty(5.0, length(), "m"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::LessThan, "a", "b"), &ctx)
            .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn quantity_equality_same_dimension() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(5.0, length(), "m"));
        ctx.set("b", qty(5.0, length(), "m"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Equal, "a", "b"), &ctx)
            .unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn quantity_equality_different_dimension() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(5.0, length(), "m"));
        ctx.set("b", qty(5.0, time_d(), "s"));
        let result = evaluator()
            .eval(&binop_vars(BinOp::Equal, "a", "b"), &ctx)
            .unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn quantity_add_dimensionless_to_quantity() {
        let mut ctx = EvalContext::new();
        ctx.set("a", qty(5.0, length(), "m"));
        ctx.set(
            "b",
            Value::Quantity {
                value: 1.0,
                dimension: DimensionVector::default(),
                unit: None,
            },
        );
        let result = evaluator()
            .eval(&binop_vars(BinOp::Add, "a", "b"), &ctx)
            .unwrap();
        match &result {
            Value::Quantity {
                value, dimension, ..
            } => {
                assert!((value - 6.0).abs() < 1e-10);
                assert_eq!(*dimension, length());
            }
            _ => panic!("expected Quantity"),
        }
    }

    // -----------------------------------------------------------------------
    // RSC-2.3: SlotRef / SlotChainHead evaluation
    // -----------------------------------------------------------------------

    fn slot_store_with(
        name: &str,
        runtime: &str,
        value: Value,
    ) -> (crate::slots::SharedSlotStore, crate::slots::SlotId) {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        let mut store = SlotStore::new();
        let id = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string(format!("decl:{name}"))),
                Variability::Continuous,
                WriterId::Orchestrator,
                name,
                runtime,
            ),
            value,
        );
        (std::sync::Arc::new(std::sync::RwLock::new(store)), id)
    }

    #[test]
    fn slotref_reads_slot_when_name_absent_from_context() {
        let (handle, id) = slot_store_with("x", "x", Value::Float(5.0));
        let mut ctx = EvalContext::new();
        ctx.slots = Some(handle);
        let expr = ExprIR::SlotRef {
            slot: id,
            name: "x".to_owned(),
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Float(5.0));
    }

    #[test]
    fn set_slot_mirrors_value_into_every_alias_beyond_two() {
        // Task #8 (steward-ruled corrected-B): `set_slot` must mirror the value
        // into the legacy map under EVERY spelling bound to the slot — the two
        // meta names AND any `add_alias` extras (N > 2), e.g. an ODE's
        // qualified `{ode}.duty` observable. Before this fix set_slot mirrored
        // only the two meta names, dropping alias-only spellings; a name-first
        // `SlotRef` read of such a spelling then shadowed the live slot.
        let (handle, id) = slot_store_with("duty", "duty", Value::Float(0.0));
        // Two alias-only spellings (qualified observable + another).
        handle
            .write()
            .unwrap()
            .add_alias("ProtectionCorePhysicsModel.duty", id);
        handle.write().unwrap().add_alias("physics.duty", id);
        let mut ctx = EvalContext::new();
        ctx.slots = Some(handle);
        assert!(ctx.set_slot(id, Value::Float(-0.26)));
        // Every spelling now reads the live value from the legacy map.
        assert_eq!(ctx.get("duty"), Some(&Value::Float(-0.26)));
        assert_eq!(
            ctx.get("ProtectionCorePhysicsModel.duty"),
            Some(&Value::Float(-0.26)),
            "the alias-only qualified observable spelling must be mirrored"
        );
        assert_eq!(ctx.get("physics.duty"), Some(&Value::Float(-0.26)));
    }

    #[test]
    fn slotref_context_name_wins_over_slot_value() {
        // Parity contract: a context binding (scoped view, RK4 stage value,
        // lambda shadow) must win over the slot's stored value.
        let (handle, id) = slot_store_with("x", "x", Value::Float(5.0));
        let mut ctx = EvalContext::new();
        ctx.slots = Some(handle);
        std::sync::Arc::make_mut(&mut ctx.variables).insert("x".to_owned(), Value::Float(7.0));
        let expr = ExprIR::SlotRef {
            slot: id,
            name: "x".to_owned(),
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Float(7.0));
    }

    #[test]
    fn slotref_reads_via_read_only_handle() {
        // RSC-2.3 read propagation: scoped/executor views carry only
        // `slot_reader`; SlotRef must still resolve through it.
        let (handle, id) = slot_store_with("x", "x", Value::Float(5.0));
        let mut ctx = EvalContext::new();
        ctx.slot_reader = Some(handle);
        assert!(ctx.slots.is_none());
        let expr = ExprIR::SlotRef {
            slot: id,
            name: "x".to_owned(),
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Float(5.0));
    }

    #[test]
    fn slotref_without_any_handle_behaves_like_feature_ref() {
        let (_, id) = slot_store_with("x", "x", Value::Float(5.0));
        // Name present in the context → value served, no handle needed.
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(3.0));
        let expr = ExprIR::SlotRef {
            slot: id,
            name: "x".to_owned(),
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Float(3.0));
        // Name absent and no handle → the FeatureRef error, verbatim.
        let empty = EvalContext::new();
        let result = evaluator().eval(&expr, &empty);
        assert!(
            matches!(result, Err(EvaluationError::UndefinedVariable(ref n)) if n == "x"),
            "expected UndefinedVariable(x), got {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // OPT #1 (runtime-hotpath-perf-plan): slot-fast lane
    // -----------------------------------------------------------------------

    #[test]
    fn slotref_fast_lane_hit_matches_name_path_value() {
        // A fast-lane float must read back identically to the float the
        // name path would have produced for the same SlotRef.
        let (_, id) = slot_store_with("B", "B", Value::Float(0.0));

        let mut name_ctx = EvalContext::new();
        name_ctx.set("B", Value::Float(1.25));

        let mut fast_ctx = EvalContext::new();
        fast_ctx.set_slot_fast(id, 1.25);

        let expr = ExprIR::SlotRef {
            slot: id,
            name: "B".to_owned(),
        };
        assert_eq!(
            evaluator().eval(&expr, &name_ctx).unwrap(),
            evaluator().eval(&expr, &fast_ctx).unwrap(),
        );
        assert_eq!(evaluator().eval(&expr, &fast_ctx).unwrap(), Value::Float(1.25));
    }

    #[test]
    fn slotref_fast_lane_wins_over_stale_name_binding() {
        // Correctness contract for the ODE RHS: the scratch base map can carry
        // a stale prior-tick value under the state var's name (left there by
        // `merge_from(ctx)`), which the RHS no longer overwrites for slotted
        // vars. The current RK-stage value lives in the fast lane and MUST win.
        let (_, id) = slot_store_with("B", "B", Value::Float(0.0));
        let mut ctx = EvalContext::new();
        // Stale name binding (previous tick).
        ctx.set("B", Value::Float(99.0));
        // Current stage value via the fast lane.
        ctx.set_slot_fast(id, 2.5);
        let expr = ExprIR::SlotRef {
            slot: id,
            name: "B".to_owned(),
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Float(2.5));
    }

    #[test]
    fn slotref_empty_fast_lane_falls_through_to_name_first() {
        // A SlotRef whose slot has NO fast-lane entry must resolve exactly as
        // before — name-first. A fast-lane entry for a DIFFERENT slot must not
        // leak into this read (regression guard for index cross-talk). Both
        // slots must come from ONE store so their indices are distinct — the
        // real minting invariant (separate stores each restart at index 0).
        let (id_b, id_x) = {
            use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
            let mut store = SlotStore::new();
            let b = store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(ElementId::from_string("decl:B")),
                    Variability::Continuous,
                    WriterId::Orchestrator,
                    "B",
                    "B",
                ),
                Value::Float(0.0),
            );
            let x = store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(ElementId::from_string("decl:x")),
                    Variability::Continuous,
                    WriterId::Orchestrator,
                    "x",
                    "x",
                ),
                Value::Float(0.0),
            );
            (b, x)
        };
        assert_ne!(id_b.index(), id_x.index());

        let mut ctx = EvalContext::new();
        ctx.set("B", Value::Float(7.0)); // name binding for B, no fast lane
        ctx.set_slot_fast(id_x, 42.0); // unrelated fast-lane slot

        let expr_b = ExprIR::SlotRef {
            slot: id_b,
            name: "B".to_owned(),
        };
        // B has no fast-lane entry → name-first wins, x's fast value never leaks.
        assert_eq!(evaluator().eval(&expr_b, &ctx).unwrap(), Value::Float(7.0));
    }

    #[test]
    fn slot_chain_head_prefers_flat_key_then_walks_tail_from_slot() {
        let mut map = BTreeMap::new();
        map.insert("c".to_owned(), Value::Int(9));
        let (handle, id) = slot_store_with("a.b", "a.b", Value::Map(map));
        let expr = ExprIR::SlotChainHead {
            slot: id,
            names: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            bound: 2,
        };

        // Flat-key fast path (FeatureChain parity) wins over the slot.
        let mut ctx = EvalContext::new();
        ctx.slot_reader = Some(std::sync::Arc::clone(&handle));
        std::sync::Arc::make_mut(&mut ctx.variables).insert("a.b.c".to_owned(), Value::Int(1));
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Int(1));

        // Head absent from the context → slot serves the head, tail walks
        // the Map.
        let mut ctx2 = EvalContext::new();
        ctx2.slot_reader = Some(handle);
        assert_eq!(evaluator().eval(&expr, &ctx2).unwrap(), Value::Int(9));

        // No handle, head absent → original FeatureChain error.
        let empty = EvalContext::new();
        let result = evaluator().eval(&expr, &empty);
        assert!(
            matches!(result, Err(EvaluationError::UndefinedVariable(ref n)) if n == "a"),
            "expected UndefinedVariable(a), got {result:?}"
        );
    }

    #[test]
    fn slot_chain_head_context_head_takes_feature_chain_path() {
        // When the head name IS in the context, SlotChainHead must follow
        // the FeatureChain walk (parity), not the slot.
        let mut inner = BTreeMap::new();
        inner.insert("c".to_owned(), Value::Int(42));
        let mut outer = BTreeMap::new();
        outer.insert("b".to_owned(), Value::Map(inner));
        let mut stale = BTreeMap::new();
        stale.insert("c".to_owned(), Value::Int(0));
        let (handle, id) = slot_store_with("a.b", "a.b", Value::Map(stale));
        let mut ctx = EvalContext::new();
        ctx.slot_reader = Some(handle);
        std::sync::Arc::make_mut(&mut ctx.variables).insert("a".to_owned(), Value::Map(outer));
        let expr = ExprIR::SlotChainHead {
            slot: id,
            names: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            bound: 2,
        };
        assert_eq!(evaluator().eval(&expr, &ctx).unwrap(), Value::Int(42));
    }

    #[test]
    fn unbound_ref_errors_instead_of_graph_name_scan() {
        // RSC-2.5: the graph-wide same-name fallback scan is deleted. A
        // Ref to a value-less element now errors as UndefinedVariable even
        // when a same-named element elsewhere in the graph carries a
        // concrete value (pre-2.5 the scan silently served that value).
        use sysml_core::ModelGraph;

        let mut graph = ModelGraph::new();
        let mut def = Element::new_with_kind(ElementKind::AttributeUsage);
        def.name = Some("temp".to_owned());
        let def_id = def.id.clone();
        graph.add_element(def);
        let mut usage = Element::new_with_kind(ElementKind::AttributeUsage);
        usage.name = Some("temp".to_owned());
        usage.set_prop("value", Value::Float(180.0));
        graph.add_element(usage);

        let mut ctx = EvalContext::new();
        ctx.graph = Some(std::sync::Arc::new(graph));
        ctx.set("temp", Value::Ref(def_id));
        let expr = ExprIR::FeatureRef("temp".to_owned());
        let result = evaluator().eval(&expr, &ctx);
        assert!(
            matches!(result, Err(EvaluationError::UndefinedVariable(ref n)) if n.starts_with("temp")),
            "unbound Ref must error (no graph-wide name scan), got {result:?}"
        );
    }
}
