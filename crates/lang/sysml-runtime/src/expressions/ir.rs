//! Expression IR types (typed AST).
//!
//! This module defines the intermediate representation for SysML/KerML expressions.

use std::collections::HashSet;

use sysml_core::physics::DimensionVector;

/// A typed expression intermediate representation.
///
/// This is the compiled form of a KerML expression, ready for evaluation.
/// Produced by the parser/compiler, consumed by [`crate::ExpressionEvaluator`].
#[derive(Debug, Clone, PartialEq)]
pub enum ExprIR {
    // -- Literals ----------------------------------------------------------
    /// Integer literal.
    LiteralInt(i64),
    /// Real (floating point) literal.
    LiteralReal(f64),
    /// Boolean literal.
    LiteralBool(bool),
    /// String literal.
    LiteralString(String),
    /// Null literal.
    LiteralNull,
    /// Quantity literal: a numeric literal carrying a unit annotation
    /// (`5 [mA]`, `273.15 [K]`). The `unit`'s dimension is resolved at compile
    /// time via the ISQ unit table (RSC-5.1b, D-5.0.5). Evaluates to a
    /// [`Value::Quantity`](sysml_core::Value::Quantity), so scale-aware
    /// arithmetic/comparison applies. A `num [unit]` literal whose unit is not
    /// in the table is compiled to a plain `LiteralInt`/`LiteralReal` instead
    /// (the unit is dropped), so this variant only ever carries a known unit.
    LiteralQuantity {
        /// Magnitude as written (in `unit`, not SI-normalised).
        value: f64,
        /// Dimension of `unit`, resolved at compile time.
        dimension: DimensionVector,
        /// The (bare) unit name, e.g. `"mA"`.
        unit: String,
    },

    // -- References --------------------------------------------------------
    /// Reference to a variable or feature by name.
    FeatureRef(String),

    /// Dot-navigation chain: `a.b.c` represented as `["a", "b", "c"]`.
    FeatureChain(Vec<String>),

    /// RSC-2.3 (design doc D-2.0.4): a [`FeatureRef`](Self::FeatureRef) (or a
    /// fully-bound [`FeatureChain`](Self::FeatureChain)) whose name was bound
    /// to a runtime slot at compile time by the `bind_slots` pass.
    ///
    /// `name` is the original spelling. The evaluator resolves the **context
    /// name first** (scoped views, RK4 stage bindings, and lambda shadowing
    /// must see exactly what `FeatureRef` saw — that is what keeps the
    /// behavioural baselines byte-identical) and falls back to the by-`SlotId`
    /// read when the name is absent from the context. The by-id fallback is
    /// what replaces the eval-time graph-wide same-name scan at RSC-2.5.
    SlotRef {
        /// The compile-bound slot.
        slot: crate::slots::SlotId,
        /// Original name spelling (context-first resolution + diagnostics).
        name: String,
    },

    /// RSC-2.3 (design doc D-2.0.4): a [`FeatureChain`](Self::FeatureChain)
    /// whose leading `bound` segments statically named a slot; the tail
    /// (`names[bound..]`) stays graph/map-navigated exactly like a
    /// `FeatureChain` tail (`Value::Ref` chains stay dynamic).
    ///
    /// The full original chain is kept so the evaluator can preserve the
    /// `FeatureChain` flat-key fast path and per-segment walk byte-for-byte;
    /// the slot is consulted only when the head does not resolve in the
    /// context.
    SlotChainHead {
        /// Slot bound for the chain head (`names[..bound]` joined with `.`).
        slot: crate::slots::SlotId,
        /// The original full chain.
        names: Vec<String>,
        /// Number of leading segments covered by `slot` (`1..names.len()`).
        bound: usize,
    },

    // -- Binary operators --------------------------------------------------
    /// Binary operation: `left op right`.
    BinaryOp {
        op: BinOp,
        left: Box<ExprIR>,
        right: Box<ExprIR>,
    },

    // -- Unary operators ---------------------------------------------------
    /// Unary operation: `op operand`.
    UnaryOp { op: UnaryOp, operand: Box<ExprIR> },

    // -- Conditional -------------------------------------------------------
    /// Ternary conditional: `if condition ? then_expr else else_expr`.
    Conditional {
        condition: Box<ExprIR>,
        then_expr: Box<ExprIR>,
        else_expr: Box<ExprIR>,
    },

    /// Null coalescing: `expr ?? default`.
    NullCoalescing {
        expr: Box<ExprIR>,
        default: Box<ExprIR>,
    },

    // -- Collection operations ---------------------------------------------
    /// `source->select { |x| predicate }`.
    Select {
        source: Box<ExprIR>,
        binding: String,
        predicate: Box<ExprIR>,
    },

    /// `source->collect { |x| transform }`.
    Collect {
        source: Box<ExprIR>,
        binding: String,
        transform: Box<ExprIR>,
    },

    /// `source->reject { |x| predicate }`.
    Reject {
        source: Box<ExprIR>,
        binding: String,
        predicate: Box<ExprIR>,
    },

    /// `source->forAll { |x| predicate }`.
    ForAll {
        source: Box<ExprIR>,
        binding: String,
        predicate: Box<ExprIR>,
    },

    /// `source->exists { |x| predicate }`.
    Exists {
        source: Box<ExprIR>,
        binding: String,
        predicate: Box<ExprIR>,
    },

    // -- Indexing -----------------------------------------------------------
    /// Collection indexing: `sequence#(index)`.
    Index {
        sequence: Box<ExprIR>,
        index: Box<ExprIR>,
    },

    // -- Function calls ----------------------------------------------------
    /// Named function call: `name(arg1, arg2, ...)`.
    FunctionCall { name: String, args: Vec<ExprIR> },

    // -- Sequence ----------------------------------------------------------
    /// Sequence / tuple construction: `(a, b, c)`.
    Sequence(Vec<ExprIR>),

    /// Range expression: `lower..upper`.
    Range {
        lower: Box<ExprIR>,
        upper: Box<ExprIR>,
    },

    // -- Classification / meta (Tier 2: compile to IR, eval returns diagnostic) --
    /// Metadata access: `@element` or `@@element`.
    /// `is_double` distinguishes `@@` (true) from `@` (false).
    MetaAccess {
        operand: Box<ExprIR>,
        is_double: bool,
    },

    /// Constructor expression: `new TypeName(a = expr1, b = expr2, ...)`.
    ConstructorCall {
        type_name: String,
        named_args: Vec<(String, Box<ExprIR>)>,
    },
}

impl ExprIR {
    /// Collect all free variable names referenced by this expression.
    ///
    /// Recursively traverses all sub-expressions, collecting `FeatureRef` names
    /// and the first element of `FeatureChain`s. Bound variables from collection
    /// operations (Select, Collect, etc.) are excluded.
    pub fn free_variables(&self) -> HashSet<String> {
        let mut vars = HashSet::new();
        self.collect_free_variables(&mut vars, &HashSet::new());
        vars
    }

    fn collect_free_variables(&self, vars: &mut HashSet<String>, bound: &HashSet<String>) {
        match self {
            ExprIR::LiteralInt(_)
            | ExprIR::LiteralReal(_)
            | ExprIR::LiteralBool(_)
            | ExprIR::LiteralString(_)
            | ExprIR::LiteralQuantity { .. }
            | ExprIR::LiteralNull => {}

            ExprIR::FeatureRef(name) | ExprIR::SlotRef { name, .. } => {
                if !bound.contains(name) {
                    vars.insert(name.clone());
                }
            }
            ExprIR::FeatureChain(chain) | ExprIR::SlotChainHead { names: chain, .. } => {
                if let Some(first) = chain.first() {
                    if !bound.contains(first) {
                        vars.insert(first.clone());
                    }
                }
            }
            ExprIR::BinaryOp { left, right, .. } => {
                left.collect_free_variables(vars, bound);
                right.collect_free_variables(vars, bound);
            }
            ExprIR::UnaryOp { operand, .. } => {
                operand.collect_free_variables(vars, bound);
            }
            ExprIR::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                condition.collect_free_variables(vars, bound);
                then_expr.collect_free_variables(vars, bound);
                else_expr.collect_free_variables(vars, bound);
            }
            ExprIR::NullCoalescing { expr, default } => {
                expr.collect_free_variables(vars, bound);
                default.collect_free_variables(vars, bound);
            }
            // Collection ops: the binding variable is bound inside the body
            ExprIR::Select {
                source,
                binding,
                predicate,
            }
            | ExprIR::Reject {
                source,
                binding,
                predicate,
            }
            | ExprIR::ForAll {
                source,
                binding,
                predicate,
            }
            | ExprIR::Exists {
                source,
                binding,
                predicate,
            } => {
                source.collect_free_variables(vars, bound);
                let mut inner_bound = bound.clone();
                inner_bound.insert(binding.clone());
                predicate.collect_free_variables(vars, &inner_bound);
            }
            ExprIR::Collect {
                source,
                binding,
                transform,
            } => {
                source.collect_free_variables(vars, bound);
                let mut inner_bound = bound.clone();
                inner_bound.insert(binding.clone());
                transform.collect_free_variables(vars, &inner_bound);
            }
            ExprIR::Index { sequence, index } => {
                sequence.collect_free_variables(vars, bound);
                index.collect_free_variables(vars, bound);
            }
            ExprIR::FunctionCall { args, .. } => {
                for arg in args {
                    arg.collect_free_variables(vars, bound);
                }
            }
            ExprIR::Sequence(items) => {
                for item in items {
                    item.collect_free_variables(vars, bound);
                }
            }
            ExprIR::Range { lower, upper } => {
                lower.collect_free_variables(vars, bound);
                upper.collect_free_variables(vars, bound);
            }
            ExprIR::MetaAccess { operand, .. } => {
                operand.collect_free_variables(vars, bound);
            }
            ExprIR::ConstructorCall { named_args, .. } => {
                for (_, arg_expr) in named_args {
                    arg_expr.collect_free_variables(vars, bound);
                }
            }
        }
    }

    /// Collect the **slot-bindable read keys** of this expression: every
    /// `FeatureRef` name and every `FeatureChain` joined into its full dotted
    /// key (`a.b.c`) — exactly the keys [`crate::expressions::bind_slots`]
    /// tries to `resolve` (full join first, then longest head). Bound
    /// collection variables (`select`/`collect`/…) are excluded. Already-bound
    /// `SlotRef` / `SlotChainHead` nodes contribute their stored key so the
    /// pass is idempotent.
    ///
    /// Unlike [`free_variables`](Self::free_variables) (which collapses a
    /// `FeatureChain` to its head), this yields the WHOLE chain — the spelling
    /// a per-instance slot must carry for the chain to bind FULLY to a
    /// `SlotRef` (RSC-3.6: minting the slots a prefixed executor's guard /
    /// trigger expressions read so they resolve instance-local).
    pub fn slot_bindable_reads(&self) -> HashSet<String> {
        let mut reads = HashSet::new();
        self.collect_slot_bindable_reads(&mut reads, &HashSet::new());
        reads
    }

    fn collect_slot_bindable_reads(&self, reads: &mut HashSet<String>, bound: &HashSet<String>) {
        match self {
            ExprIR::LiteralInt(_)
            | ExprIR::LiteralReal(_)
            | ExprIR::LiteralBool(_)
            | ExprIR::LiteralString(_)
            | ExprIR::LiteralQuantity { .. }
            | ExprIR::LiteralNull => {}

            ExprIR::FeatureRef(name) | ExprIR::SlotRef { name, .. } => {
                if !bound.contains(name) {
                    reads.insert(name.clone());
                }
            }
            ExprIR::FeatureChain(chain) | ExprIR::SlotChainHead { names: chain, .. } => {
                if chain.first().is_some_and(|f| !bound.contains(f)) {
                    reads.insert(chain.join("."));
                }
            }
            ExprIR::BinaryOp { left, right, .. } => {
                left.collect_slot_bindable_reads(reads, bound);
                right.collect_slot_bindable_reads(reads, bound);
            }
            ExprIR::UnaryOp { operand, .. } => operand.collect_slot_bindable_reads(reads, bound),
            ExprIR::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                condition.collect_slot_bindable_reads(reads, bound);
                then_expr.collect_slot_bindable_reads(reads, bound);
                else_expr.collect_slot_bindable_reads(reads, bound);
            }
            ExprIR::NullCoalescing { expr, default } => {
                expr.collect_slot_bindable_reads(reads, bound);
                default.collect_slot_bindable_reads(reads, bound);
            }
            ExprIR::Select {
                source,
                binding,
                predicate,
            }
            | ExprIR::Reject {
                source,
                binding,
                predicate,
            }
            | ExprIR::ForAll {
                source,
                binding,
                predicate,
            }
            | ExprIR::Exists {
                source,
                binding,
                predicate,
            } => {
                source.collect_slot_bindable_reads(reads, bound);
                let mut inner_bound = bound.clone();
                inner_bound.insert(binding.clone());
                predicate.collect_slot_bindable_reads(reads, &inner_bound);
            }
            ExprIR::Collect {
                source,
                binding,
                transform,
            } => {
                source.collect_slot_bindable_reads(reads, bound);
                let mut inner_bound = bound.clone();
                inner_bound.insert(binding.clone());
                transform.collect_slot_bindable_reads(reads, &inner_bound);
            }
            ExprIR::Index { sequence, index } => {
                sequence.collect_slot_bindable_reads(reads, bound);
                index.collect_slot_bindable_reads(reads, bound);
            }
            ExprIR::FunctionCall { args, .. } => {
                for arg in args {
                    arg.collect_slot_bindable_reads(reads, bound);
                }
            }
            ExprIR::Sequence(items) => {
                for item in items {
                    item.collect_slot_bindable_reads(reads, bound);
                }
            }
            ExprIR::Range { lower, upper } => {
                lower.collect_slot_bindable_reads(reads, bound);
                upper.collect_slot_bindable_reads(reads, bound);
            }
            ExprIR::MetaAccess { operand, .. } => operand.collect_slot_bindable_reads(reads, bound),
            ExprIR::ConstructorCall { named_args, .. } => {
                for (_, arg_expr) in named_args {
                    arg_expr.collect_slot_bindable_reads(reads, bound);
                }
            }
        }
    }

    /// RSC-4.1 (read-set accessor): collect the **compiler-resolved
    /// `SlotId`s** this expression reads — every [`SlotRef`](Self::SlotRef)
    /// and [`SlotChainHead`](Self::SlotChainHead) node the
    /// [`crate::expressions::bind_slots`] pass produced.
    ///
    /// This is the bound-IR counterpart of
    /// [`slot_bindable_reads`](Self::slot_bindable_reads): where that returns
    /// the candidate read *names* before binding, this returns the resolved
    /// slot identities **after** binding (§9 Q2 of the RSC-4.0 scheduler doc:
    /// read-set = compiler-resolved slot-ids). Unbound `FeatureRef` /
    /// `FeatureChain` nodes contribute nothing — they never resolved to a
    /// slot, so they read no peer's write-set deterministically. The result is
    /// raw (insertion-ordered, may contain duplicates); callers sort+dedup.
    pub fn slot_reads(&self) -> Vec<crate::slots::SlotId> {
        let mut out = Vec::new();
        self.collect_slot_reads(&mut out);
        out
    }

    fn collect_slot_reads(&self, out: &mut Vec<crate::slots::SlotId>) {
        match self {
            ExprIR::LiteralInt(_)
            | ExprIR::LiteralReal(_)
            | ExprIR::LiteralBool(_)
            | ExprIR::LiteralString(_)
            | ExprIR::LiteralQuantity { .. }
            | ExprIR::LiteralNull
            | ExprIR::FeatureRef(_)
            | ExprIR::FeatureChain(_) => {}

            ExprIR::SlotRef { slot, .. } | ExprIR::SlotChainHead { slot, .. } => out.push(*slot),

            ExprIR::BinaryOp { left, right, .. } => {
                left.collect_slot_reads(out);
                right.collect_slot_reads(out);
            }
            ExprIR::UnaryOp { operand, .. } => operand.collect_slot_reads(out),
            ExprIR::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                condition.collect_slot_reads(out);
                then_expr.collect_slot_reads(out);
                else_expr.collect_slot_reads(out);
            }
            ExprIR::NullCoalescing { expr, default } => {
                expr.collect_slot_reads(out);
                default.collect_slot_reads(out);
            }
            ExprIR::Select {
                source, predicate, ..
            }
            | ExprIR::Reject {
                source, predicate, ..
            }
            | ExprIR::ForAll {
                source, predicate, ..
            }
            | ExprIR::Exists {
                source, predicate, ..
            } => {
                source.collect_slot_reads(out);
                predicate.collect_slot_reads(out);
            }
            ExprIR::Collect {
                source, transform, ..
            } => {
                source.collect_slot_reads(out);
                transform.collect_slot_reads(out);
            }
            ExprIR::Index { sequence, index } => {
                sequence.collect_slot_reads(out);
                index.collect_slot_reads(out);
            }
            ExprIR::FunctionCall { args, .. } => {
                for arg in args {
                    arg.collect_slot_reads(out);
                }
            }
            ExprIR::Sequence(items) => {
                for item in items {
                    item.collect_slot_reads(out);
                }
            }
            ExprIR::Range { lower, upper } => {
                lower.collect_slot_reads(out);
                upper.collect_slot_reads(out);
            }
            ExprIR::MetaAccess { operand, .. } => operand.collect_slot_reads(out),
            ExprIR::ConstructorCall { named_args, .. } => {
                for (_, arg_expr) in named_args {
                    arg_expr.collect_slot_reads(out);
                }
            }
        }
    }
}

/// Binary operators with KerML precedence semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,

    // Comparison
    Equal,
    NotEqual,
    ReferenceEqual,
    ReferenceNotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,

    // Logical
    And,
    Or,
    Xor,
    Implies,

    // Bitwise
    BitAnd,
    BitOr,

    // Classification (Tier 2: compile succeeds, eval returns UnsupportedOperator)
    /// `expr hastype Type` — test if value has specific type
    HasType,
    /// `expr istype Type` — test if value is exactly a type
    IsType,
    /// `expr as Type` — cast expression to a type
    As,
    /// `expr meta Type` — access metaclass of expression
    Meta,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// Arithmetic negation: `-x`.
    Negate,
    /// Unary plus (identity): `+x`.
    Plus,
    /// Logical not: `not x`.
    Not,
    /// Bitwise complement: `~x`.
    BitNot,
}
