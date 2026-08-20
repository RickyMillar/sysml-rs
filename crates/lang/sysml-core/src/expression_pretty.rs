//! Plain-text pretty-printer for parser-emitted expression element subtrees.
//!
//! Walks a `ModelGraph` starting at an "expression owner" (constraint usage,
//! calculation usage, attribute usage, …) and renders the structured
//! expression children as a source-like string.
//!
//! Used during elaboration to populate the legacy `constraint` / `expr`
//! string properties from the Phase-1 AST subtree, so downstream consumers
//! that still read those strings keep working after the parser stopped
//! writing `unresolved_value`. Mirrors (and is the canonical home of) the
//! helper that previously lived in `sysml-diagram` and `sysml-service`.
//!
//! The precedence table and formatting rules are shared between the
//! graph-based walk (this module) and the projection-based walk used by the
//! service layer (`sysml-service::expression_ast`). Both implementations
//! drive the same `pretty` function via the [`PrettyPrintTarget`] trait —
//! the operator table, parenthesization policy, and rendering conventions
//! live here and nowhere else.

use crate::{Element, ElementKind, ModelGraph, Value};

// ---------------------------------------------------------------------------
// Trait — shared interface for graph walks and AST-node projections
// ---------------------------------------------------------------------------

/// A node the pretty-printer can walk. Implementations expose the minimal
/// surface the formatter needs: kind tag, optional name, properties, and
/// argIndex-sorted expression children.
///
/// The service-layer `ExpressionAstNode` (JSON projection) and the
/// graph-based `(Element, ModelGraph)` pair both implement this trait so
/// they share a single precedence table and paren policy.
pub trait PrettyPrintTarget {
    /// Element-kind discriminator (e.g. `"OperatorExpression"`, `"LiteralInteger"`).
    fn kind(&self) -> &str;

    /// Optional identifier (used for `FeatureReferenceExpression` names and
    /// invocation/constructor callees).
    fn name(&self) -> Option<&str>;

    /// Read a property by key. Returns the underlying `Value` reference so
    /// the formatter can render numeric/boolean literals without allocating
    /// until it chooses a final shape.
    fn prop_value(&self, key: &str) -> Option<&Value>;

    /// Expression-kind children in `argIndex` order. Non-expression children
    /// (e.g. `FeatureTyping`) must be filtered out by implementations.
    /// Each call owns fresh `Box<dyn>` handles to decouple lifetimes.
    fn sorted_children(&self) -> Vec<Box<dyn PrettyPrintTarget + '_>>;
}

/// Render the top-level expression rooted at a target.
pub fn pretty_print(target: &dyn PrettyPrintTarget) -> String {
    pretty(target, Prec::Lowest)
}

// ---------------------------------------------------------------------------
// Graph-based target: (Element, ModelGraph)
// ---------------------------------------------------------------------------

/// View of a `ModelGraph` element as a [`PrettyPrintTarget`]. Holds
/// references, so cloning is cheap and the formatter can descend through
/// children without additional allocation beyond the trait-object boxes.
pub struct ElementView<'a> {
    element: &'a Element,
    graph: &'a ModelGraph,
}

impl<'a> ElementView<'a> {
    pub fn new(element: &'a Element, graph: &'a ModelGraph) -> Self {
        Self { element, graph }
    }
}

impl<'a> PrettyPrintTarget for ElementView<'a> {
    fn kind(&self) -> &str {
        kind_tag(&self.element.kind)
    }

    fn name(&self) -> Option<&str> {
        self.element.name.as_deref()
    }

    fn prop_value(&self, key: &str) -> Option<&Value> {
        self.element.get_prop(key)
    }

    fn sorted_children(&self) -> Vec<Box<dyn PrettyPrintTarget + '_>> {
        let mut kids: Vec<&Element> = self
            .graph
            .children_of(&self.element.id)
            .filter(|c| is_expression_kind(&c.kind))
            .collect();
        kids.sort_by_key(element_arg_index);
        kids.into_iter()
            .map(|child| {
                Box::new(ElementView {
                    element: child,
                    graph: self.graph,
                }) as Box<dyn PrettyPrintTarget + '_>
            })
            .collect()
    }
}

/// Render the expression rooted at `owner`'s first expression-element child
/// as a plain-text string. Returns `None` when no structured expression
/// child is present.
pub fn pretty_print_owner(owner: &Element, graph: &ModelGraph) -> Option<String> {
    let root = first_expression_child(owner, graph)?;
    let view = ElementView {
        element: root,
        graph,
    };
    Some(pretty(&view, Prec::Lowest))
}

fn first_expression_child<'g>(owner: &Element, graph: &'g ModelGraph) -> Option<&'g Element> {
    graph
        .children_of(&owner.id)
        .filter(|c| is_expression_kind(&c.kind))
        .min_by_key(element_arg_index)
}

fn element_arg_index(element: &&Element) -> i64 {
    element
        .get_prop("argIndex")
        .and_then(|v| match v {
            Value::Int(i) => Some(*i),
            _ => None,
        })
        .unwrap_or(i64::MAX)
}

/// Stable string tag for an `ElementKind`. Matches the `format!("{:?}", kind)`
/// convention already used by the service projection, so both
/// implementations observe identical kind strings when dispatching inside
/// `pretty`.
fn kind_tag(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::LiteralBoolean => "LiteralBoolean",
        ElementKind::LiteralInteger => "LiteralInteger",
        ElementKind::LiteralRational => "LiteralRational",
        ElementKind::LiteralString => "LiteralString",
        ElementKind::LiteralInfinity => "LiteralInfinity",
        ElementKind::LiteralExpression => "LiteralExpression",
        ElementKind::NullExpression => "NullExpression",
        ElementKind::OperatorExpression => "OperatorExpression",
        ElementKind::InvocationExpression => "InvocationExpression",
        ElementKind::FeatureReferenceExpression => "FeatureReferenceExpression",
        ElementKind::FeatureChainExpression => "FeatureChainExpression",
        ElementKind::SelectExpression => "SelectExpression",
        ElementKind::CollectExpression => "CollectExpression",
        ElementKind::IndexExpression => "IndexExpression",
        ElementKind::MetadataAccessExpression => "MetadataAccessExpression",
        ElementKind::ConstructorExpression => "ConstructorExpression",
        _ => "Other",
    }
}

pub fn is_expression_kind(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::LiteralBoolean
            | ElementKind::LiteralInteger
            | ElementKind::LiteralRational
            | ElementKind::LiteralString
            | ElementKind::LiteralInfinity
            | ElementKind::LiteralExpression
            | ElementKind::NullExpression
            | ElementKind::OperatorExpression
            | ElementKind::InvocationExpression
            | ElementKind::FeatureReferenceExpression
            | ElementKind::FeatureChainExpression
            | ElementKind::SelectExpression
            | ElementKind::CollectExpression
            | ElementKind::IndexExpression
            | ElementKind::MetadataAccessExpression
            | ElementKind::ConstructorExpression
    )
}

// ---------------------------------------------------------------------------
// Precedence / rendering (the canonical formatter)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    Lowest,
    Implies,
    Or,
    And,
    Eq,
    Rel,
    Add,
    Mul,
    Unary,
    Pow,
    Atom,
}

fn prec_of(op: &str) -> Prec {
    match op {
        "implies" => Prec::Implies,
        "or" | "xor" | "||" | "|" | "??" => Prec::Or,
        "and" | "&&" | "&" => Prec::And,
        "==" | "!=" | "===" | "!==" => Prec::Eq,
        "<" | ">" | "<=" | ">=" | "hastype" | "istype" | "@" | "@@" | "as" | "meta" | ".." => {
            Prec::Rel
        }
        "+" | "-" => Prec::Add,
        "*" | "/" | "%" => Prec::Mul,
        "**" | "^" => Prec::Pow,
        "not" | "~" => Prec::Unary,
        _ => Prec::Atom,
    }
}

fn step_up(p: Prec) -> Prec {
    match p {
        Prec::Lowest => Prec::Implies,
        Prec::Implies => Prec::Or,
        Prec::Or => Prec::And,
        Prec::And => Prec::Eq,
        Prec::Eq => Prec::Rel,
        Prec::Rel => Prec::Add,
        Prec::Add => Prec::Mul,
        Prec::Mul => Prec::Unary,
        Prec::Unary => Prec::Pow,
        Prec::Pow => Prec::Atom,
        Prec::Atom => Prec::Atom,
    }
}

fn prop_str<'a>(target: &'a dyn PrettyPrintTarget, key: &str) -> Option<&'a str> {
    target.prop_value(key).and_then(|v| match v {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    })
}

fn pretty(target: &dyn PrettyPrintTarget, parent: Prec) -> String {
    match target.kind() {
        "LiteralInteger" | "LiteralRational" => target
            .prop_value("value")
            .map(value_to_plain)
            .unwrap_or_else(|| "?".into()),
        "LiteralBoolean" => target
            .prop_value("value")
            .and_then(|v| match v {
                Value::Bool(b) => Some(*b),
                _ => None,
            })
            .map(|b| if b { "true".into() } else { "false".into() })
            .unwrap_or_else(|| "?".into()),
        "LiteralString" => {
            let s = prop_str(target, "value").unwrap_or("");
            format!("\"{s}\"")
        }
        "LiteralInfinity" => "*".into(),
        "NullExpression" => "null".into(),
        "FeatureReferenceExpression" => target
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| "?".into()),
        "FeatureChainExpression" => target
            .sorted_children()
            .iter()
            .map(|c| pretty(c.as_ref(), Prec::Atom))
            .collect::<Vec<_>>()
            .join("."),
        "OperatorExpression" => pretty_operator(target, parent),
        "InvocationExpression" => pretty_invocation(target),
        "IndexExpression" => {
            let kids = target.sorted_children();
            let src = kids
                .first()
                .map(|c| pretty(c.as_ref(), Prec::Atom))
                .unwrap_or_else(|| "?".into());
            let idx = kids
                .get(1)
                .map(|c| pretty(c.as_ref(), Prec::Lowest))
                .unwrap_or_else(|| "?".into());
            format!("{src}#({idx})")
        }
        "MetadataAccessExpression" => {
            let head = target.name().unwrap_or("?");
            format!("{head}.metadata")
        }
        "ConstructorExpression" => {
            let name = target.name().unwrap_or("?");
            let args = target
                .sorted_children()
                .iter()
                .map(|c| pretty(c.as_ref(), Prec::Lowest))
                .collect::<Vec<_>>()
                .join(", ");
            format!("new {name}({args})")
        }
        _ => target
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| "?".into()),
    }
}

fn pretty_operator(target: &dyn PrettyPrintTarget, parent: Prec) -> String {
    let op = prop_str(target, "operator").unwrap_or("?");
    let kids = target.sorted_children();

    if op == "if" && kids.len() == 3 {
        let c = pretty(kids[0].as_ref(), Prec::Lowest);
        let t = pretty(kids[1].as_ref(), Prec::Lowest);
        let e = pretty(kids[2].as_ref(), Prec::Lowest);
        // A conditional is the lowest-precedence production (KerMLExpressions.xtext
        // §54-62: ConditionalExpression is the root of the expression hierarchy).
        // As an operand of ANY operator it must be parenthesized, or the bare
        // `else` branch greedily swallows the following operator on re-parse — e.g.
        // `(if c ? a else b) - x` would round-trip as `if c ? a else (b - x)`,
        // silently rebinding `- x` to the else branch only.
        let inner = format!("if {c} ? {t} else {e}");
        return paren_if(inner, Prec::Lowest, parent);
    }

    let this = prec_of(op);

    if kids.len() == 1 {
        let operand = pretty(kids[0].as_ref(), Prec::Unary);
        let sep = if op.chars().all(|c| c.is_alphabetic()) {
            " "
        } else {
            ""
        };
        let inner = format!("{op}{sep}{operand}");
        return paren_if(inner, this, parent);
    }

    if kids.len() == 2 {
        let lhs = pretty(kids[0].as_ref(), this);
        let rhs_prec = match op {
            "**" | "^" => this,
            _ => step_up(this),
        };
        let rhs = pretty(kids[1].as_ref(), rhs_prec);
        let inner = format!("{lhs} {op} {rhs}");
        return paren_if(inner, this, parent);
    }

    kids.iter()
        .map(|c| pretty(c.as_ref(), this))
        .collect::<Vec<_>>()
        .join(&format!(" {op} "))
}

fn pretty_invocation(target: &dyn PrettyPrintTarget) -> String {
    let name = target.name().unwrap_or("?");
    let arrow_fn = matches!(name, "select" | "collect" | "forAll" | "exists" | "reject");
    let mut rendered: Vec<String> = target
        .sorted_children()
        .iter()
        .map(|c| pretty(c.as_ref(), Prec::Lowest))
        .collect();

    if arrow_fn && !rendered.is_empty() {
        let source = rendered.remove(0);
        let body = rendered.join(", ");
        if body.is_empty() {
            return format!("{source}->{name}{{}}");
        }
        return format!("{source}->{name}{{{body}}}");
    }

    let args = rendered.join(", ");
    format!("{name}({args})")
}

fn paren_if(inner: String, this: Prec, parent: Prec) -> String {
    if (this as u8) < (parent as u8) {
        format!("({inner})")
    } else {
        inner
    }
}

fn value_to_plain(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            let s = format!("{f}");
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{s}.0")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        _ => "?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory [`PrettyPrintTarget`] for exercising the formatter's
    /// precedence/paren policy without standing up a ModelGraph.
    struct Node {
        kind: &'static str,
        name: Option<&'static str>,
        op_value: Option<Value>,
        children: Vec<Node>,
    }

    impl Node {
        fn op(operator: &'static str, children: Vec<Node>) -> Self {
            Node {
                kind: "OperatorExpression",
                name: None,
                op_value: Some(Value::String(operator.to_owned())),
                children,
            }
        }
        fn feat(name: &'static str) -> Self {
            Node {
                kind: "FeatureReferenceExpression",
                name: Some(name),
                op_value: None,
                children: vec![],
            }
        }
    }

    impl PrettyPrintTarget for Node {
        fn kind(&self) -> &str {
            self.kind
        }
        fn name(&self) -> Option<&str> {
            self.name
        }
        fn prop_value(&self, key: &str) -> Option<&Value> {
            if key == "operator" {
                self.op_value.as_ref()
            } else {
                None
            }
        }
        fn sorted_children(&self) -> Vec<Box<dyn PrettyPrintTarget + '_>> {
            self.children
                .iter()
                .map(|c| Box::new(c) as Box<dyn PrettyPrintTarget + '_>)
                .collect()
        }
    }

    impl PrettyPrintTarget for &Node {
        fn kind(&self) -> &str {
            (*self).kind()
        }
        fn name(&self) -> Option<&str> {
            (*self).name()
        }
        fn prop_value(&self, key: &str) -> Option<&Value> {
            (*self).prop_value(key)
        }
        fn sorted_children(&self) -> Vec<Box<dyn PrettyPrintTarget + '_>> {
            (*self).sorted_children()
        }
    }

    /// A conditional that is an operand of a binary operator must be wrapped in
    /// parens, or `- H_dc` re-parses into the else branch only. Regression test
    /// for the DC-bias asymmetry bug in the original hybrid-simulation fixture
    /// (since replaced by the espresso fixtures).
    #[test]
    fn conditional_operand_of_subtraction_is_parenthesized() {
        // (if cond ? a else b) - x
        let cond = Node::op(
            "if",
            vec![Node::feat("cond"), Node::feat("a"), Node::feat("b")],
        );
        let sub = Node::op("-", vec![cond, Node::feat("x")]);
        assert_eq!(pretty_print(&sub), "(if cond ? a else b) - x");
    }

    /// At top level (no enclosing operator) a conditional stays bare.
    #[test]
    fn top_level_conditional_is_not_parenthesized() {
        let cond = Node::op(
            "if",
            vec![Node::feat("cond"), Node::feat("a"), Node::feat("b")],
        );
        assert_eq!(pretty_print(&cond), "if cond ? a else b");
    }
}
