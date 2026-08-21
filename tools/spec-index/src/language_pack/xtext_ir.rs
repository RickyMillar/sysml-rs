//! Normalize `sysml-codegen` grammar IR into card-facing shape.
//!
//! Grammar tokenization lives in `sysml-codegen::xtext_parser` (one home for
//! Xtext parsing). This module only *normalizes*: it locates
//! a rule's IR across the three grammars, resolves rule-ref dependencies
//! against the full rule-name universe, and reports dangling references.

use std::collections::BTreeSet;

use sysml_codegen::{parse_grammar_ir, GrammarRuleIr, IrNode};

/// Collect `rule-ref` names (only) from an IR tree, in first-appearance order.
/// Cross-ref targets name metamodel types, not productions, so they are
/// excluded — dangling-rule detection must not flag them.
pub fn rule_ref_names(node: &IrNode, out: &mut Vec<String>) {
    match node {
        IrNode::RuleRef { name } => {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        IrNode::Sequence { items } | IrNode::Choice { items } | IrNode::UnorderedGroup { items } => {
            for it in items {
                rule_ref_names(it, out);
            }
        }
        IrNode::Optional { item }
        | IrNode::ZeroOrMore { item }
        | IrNode::OneOrMore { item }
        | IrNode::Assignment { item, .. } => rule_ref_names(item, out),
        _ => {}
    }
}

/// The three grammar sources, already read from allowlisted paths.
pub struct Grammars {
    pub kerml: String,
    pub sysml: String,
    pub expressions: String,
}

impl Grammars {
    /// Parse every rule in every grammar into IR, tagged by grammar label.
    pub fn all_ir(&self) -> Vec<GrammarRuleIr> {
        let mut out = Vec::new();
        out.extend(parse_grammar_ir("kerml", &self.kerml));
        out.extend(parse_grammar_ir("sysml", &self.sysml));
        out.extend(parse_grammar_ir("expressions", &self.expressions));
        out
    }

    /// Every rule name declared across all three grammars.
    pub fn rule_name_universe(&self) -> BTreeSet<String> {
        self.all_ir().into_iter().map(|r| r.rule).collect()
    }
}

/// Find the IR for `rule` in grammar `grammar` (`kerml`/`sysml`/`expressions`).
pub fn rule_ir(grammars: &Grammars, grammar: &str, rule: &str) -> Option<GrammarRuleIr> {
    let src = match grammar {
        "kerml" => &grammars.kerml,
        "sysml" => &grammars.sysml,
        "expressions" => &grammars.expressions,
        _ => return None,
    };
    parse_grammar_ir(grammar, src)
        .into_iter()
        .find(|r| r.rule == rule)
}

/// Dependency rule-ref/cross-ref names in `ir` that do not resolve to any rule
/// in the grammar universe (a dangling-ref conflict). Cross-ref
/// targets name metamodel types, so they are not expected to resolve as rules
/// and are excluded here.
pub fn dangling_dependencies(ir: &GrammarRuleIr, universe: &BTreeSet<String>) -> Vec<String> {
    let mut refs = Vec::new();
    rule_ref_names(&ir.expression, &mut refs);
    refs.into_iter()
        .filter(|dep| !universe.contains(dep))
        .collect()
}
