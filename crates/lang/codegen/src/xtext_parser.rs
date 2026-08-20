//! Xtext grammar parser for extracting operators, keywords, and rules.
//!
//! This module parses Xtext grammar files (`.xtext`) to extract:
//! - Operators with precedence information
//! - Keywords defined in the grammar
//! - Enum definitions
//! - Grammar rules
//!
//! The primary source is `KerMLExpressions.xtext` for operators and `SysML.xtext`
//! for keywords and domain-specific rules.

use std::collections::HashMap;

/// Information about an operator extracted from xtext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorInfo {
    /// The rule name (e.g., "EqualityOperator").
    pub name: String,
    /// The operator symbols (e.g., ["==", "!=", "===", "!=="]).
    pub symbols: Vec<String>,
    /// Category derived from name (e.g., "equality").
    pub category: String,
    /// Precedence level (higher = binds tighter).
    pub precedence: u8,
}

/// Information about an enum defined in xtext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XtextEnumInfo {
    /// The enum name (e.g., "FeatureDirection").
    pub name: String,
    /// The returns type (e.g., "SysML::FeatureDirectionKind").
    pub returns_type: String,
    /// The values as (variant_name, keyword) pairs.
    pub values: Vec<(String, String)>,
}

/// Information about a grammar rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XtextRule {
    /// The rule name.
    pub name: String,
    /// The returns type, if specified.
    pub returns_type: Option<String>,
    /// Whether this is a fragment rule.
    pub is_fragment: bool,
    /// Whether this is a terminal rule.
    pub is_terminal: bool,
}

/// Precedence levels for operators (from lowest to highest binding).
/// These values come from the structure of KerMLExpressions.xtext.
const PRECEDENCE_MAP: &[(&str, u8)] = &[
    ("ConditionalOperator", 1),
    ("NullCoalescingOperator", 2),
    ("ImpliesOperator", 3),
    ("OrOperator", 4),
    ("ConditionalOrOperator", 4),
    ("XorOperator", 5),
    ("AndOperator", 6),
    ("ConditionalAndOperator", 6),
    ("EqualityOperator", 7),
    ("ClassificationTestOperator", 8),
    ("MetaClassificationTestOperator", 9),
    ("CastOperator", 10),
    ("MetaCastOperator", 10),
    ("RelationalOperator", 11),
    ("RangeOperator", 12),
    ("AdditiveOperator", 13),
    ("MultiplicativeOperator", 14),
    ("ExponentiationOperator", 15),
    ("UnaryOperator", 16),
];

/// Parse operators from an xtext grammar file (typically KerMLExpressions.xtext).
///
/// Looks for patterns like:
/// ```text
/// EqualityOperator :
///     '==' | '!=' | '===' | '!=='
/// ;
/// ```
///
/// # Arguments
///
/// * `content` - The xtext file content as a string
///
/// # Returns
///
/// A vector of `OperatorInfo` structs with precedence information.
pub fn parse_xtext_operators(content: &str) -> Vec<OperatorInfo> {
    let mut operators = Vec::new();
    let precedence_lookup: HashMap<&str, u8> = PRECEDENCE_MAP.iter().cloned().collect();

    // Match operator rule definitions
    // Pattern: OperatorName : 'symbol' | 'symbol' ... ;
    let lines: Vec<&str> = content.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();

        // Check if this is an operator rule definition
        if line.ends_with("Operator :") || line.ends_with("Operator:") {
            let name = line
                .trim_end_matches(':')
                .trim_end_matches(" :")
                .trim()
                .to_owned();

            // Collect the operator body (may span multiple lines)
            let mut body = String::new();
            i += 1;
            while i < lines.len() {
                let body_line = lines[i].trim();
                body.push_str(body_line);
                body.push(' ');
                if body_line.ends_with(';') {
                    break;
                }
                i += 1;
            }

            // Extract symbols from the body
            let symbols = extract_symbols(&body);

            if !symbols.is_empty() {
                let category = derive_category(&name);
                let precedence = precedence_lookup.get(name.as_str()).copied().unwrap_or(0);

                operators.push(OperatorInfo {
                    name,
                    symbols,
                    category,
                    precedence,
                });
            }
        }
        i += 1;
    }

    // Sort by precedence
    operators.sort_by_key(|op| op.precedence);
    operators
}

/// Extract all unique quoted keyword-like strings from xtext content.
///
/// This function scans the entire xtext file for quoted strings that look
/// like keywords (alphabetic identifiers) and returns them. This is useful
/// for finding keywords that are used inline rather than in dedicated rules.
///
/// # Arguments
///
/// * `content` - The xtext file content as a string
///
/// # Returns
///
/// A vector of unique keyword strings, sorted alphabetically.
pub fn extract_all_keyword_strings(content: &str) -> Vec<String> {
    let mut keywords = std::collections::HashSet::new();

    // Extract all single-quoted strings that are alphabetic identifiers
    let mut in_quote = false;
    let mut current = String::new();

    for ch in content.chars() {
        if ch == '\'' {
            if in_quote {
                // Check if the extracted string is a valid keyword (alphabetic)
                if !current.is_empty()
                    && current.chars().all(|c| c.is_ascii_alphabetic())
                    && current.len() > 1
                {
                    keywords.insert(current.clone());
                }
                current.clear();
            }
            in_quote = !in_quote;
        } else if in_quote {
            current.push(ch);
        }
    }

    let mut result: Vec<_> = keywords.into_iter().collect();
    result.sort();
    result
}

/// Parse enum definitions from an xtext grammar file.
///
/// Looks for patterns like:
/// ```text
/// enum FeatureDirection returns SysML::FeatureDirectionKind :
///     in | out | inout | ...
/// ;
/// ```
///
/// # Arguments
///
/// * `content` - The xtext file content as a string
///
/// # Returns
///
/// A vector of `XtextEnumInfo` structs.
pub fn parse_xtext_enums(content: &str) -> Vec<XtextEnumInfo> {
    let mut enums = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();

        // Check if this is an enum definition
        if line.starts_with("enum ") {
            // Parse: enum Name returns Type :
            // Find the colon that separates the header from the body (not ::)
            if let Some(colon_pos) = find_rule_colon(line) {
                let header = &line[5..colon_pos].trim(); // skip "enum "
                let parts: Vec<&str> = header.split_whitespace().collect();

                if parts.len() >= 3 && parts[1] == "returns" {
                    let name = parts[0].to_owned();
                    let returns_type = parts[2..].join(" ");

                    // Get anything after the colon on the same line
                    let mut body = line[colon_pos + 1..].to_string();

                    // Collect the rest of the enum body
                    i += 1;
                    while i < lines.len() {
                        let body_line = lines[i].trim();
                        body.push_str(body_line);
                        body.push(' ');
                        if body_line.ends_with(';') {
                            break;
                        }
                        i += 1;
                    }

                    // Parse enum values
                    let values = parse_enum_values(&body);

                    enums.push(XtextEnumInfo {
                        name,
                        returns_type,
                        values,
                    });
                }
            }
        }
        i += 1;
    }

    enums
}

/// Parse grammar rules from an xtext grammar file.
///
/// Extracts rule names, return types, and whether they are fragments or terminals.
///
/// # Arguments
///
/// * `content` - The xtext file content as a string
///
/// # Returns
///
/// A vector of `XtextRule` structs.
#[allow(clippy::expect_used)] // Invariant: strip_prefix after starts_with check
pub fn parse_xtext_rules(content: &str) -> Vec<XtextRule> {
    let mut rules = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    for line in lines {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        // Terminal rule: terminal NAME ...
        if trimmed.starts_with("terminal ") {
            let name = trimmed
                .strip_prefix("terminal ")
                .expect("invariant: starts_with checked above")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            if !name.is_empty() {
                rules.push(XtextRule {
                    name,
                    returns_type: None,
                    is_fragment: false,
                    is_terminal: true,
                });
            }
            continue;
        }

        // Fragment rule: fragment NAME returns TYPE :
        if trimmed.starts_with("fragment ") {
            if let Some((name, returns_type)) = parse_rule_header(
                trimmed
                    .strip_prefix("fragment ")
                    .expect("invariant: starts_with checked above"),
            ) {
                rules.push(XtextRule {
                    name,
                    returns_type,
                    is_fragment: true,
                    is_terminal: false,
                });
            }
            continue;
        }

        // Regular rule: NAME returns TYPE : or NAME :
        if let Some((name, returns_type)) = parse_rule_header(trimmed) {
            // Skip operator and keyword rules (they're handled separately)
            if name.ends_with("Operator") || name.ends_with("Keyword") {
                continue;
            }
            rules.push(XtextRule {
                name,
                returns_type,
                is_fragment: false,
                is_terminal: false,
            });
        }
    }

    rules
}

/// Extract quoted symbols from a rule body.
fn extract_symbols(body: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();

    for ch in body.chars() {
        if ch == '\'' {
            if in_quote {
                if !current.is_empty() {
                    symbols.push(current.clone());
                }
                current.clear();
            }
            in_quote = !in_quote;
        } else if in_quote {
            current.push(ch);
        }
    }

    symbols
}

/// Extract the first quoted keyword from a rule body.
fn extract_first_keyword(body: &str) -> Option<String> {
    let symbols = extract_symbols(body);
    symbols.into_iter().next()
}

/// Derive category from operator name.
fn derive_category(name: &str) -> String {
    // Remove "Operator" suffix and convert to snake_case
    let base = name.trim_end_matches("Operator");
    let mut category = String::new();
    for (i, ch) in base.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            category.push('_');
        }
        category.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    category
}

/// Parse enum values from the body of an enum definition.
fn parse_enum_values(body: &str) -> Vec<(String, String)> {
    let mut values = Vec::new();

    // Remove semicolon (may have spaces around it) and split by |
    let cleaned = body.trim().trim_end_matches(';').trim();
    for part in cleaned.split('|') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Parse: variant = 'keyword' or just 'keyword' or just identifier
        if part.contains('=') {
            let parts: Vec<&str> = part.split('=').collect();
            if parts.len() == 2 {
                let variant = parts[0].trim().to_owned();
                // Try quoted keyword first, then fall back to identifier
                let value =
                    extract_first_keyword(parts[1]).unwrap_or_else(|| parts[1].trim().to_owned());
                if !value.is_empty() {
                    values.push((variant, value));
                }
            }
        } else if let Some(keyword) = extract_first_keyword(part) {
            // Quoted keyword: use as both variant and value
            values.push((keyword.clone(), keyword));
        } else {
            // Bare identifier: use as both variant and value
            let ident = part.trim().to_owned();
            if !ident.is_empty() && ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
                values.push((ident.clone(), ident));
            }
        }
    }

    values
}

/// Parse a rule header to extract name and optional returns type.
fn parse_rule_header(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim();

    // Must contain : to be a rule definition (but not ::)
    // Find the first standalone : (not part of ::)
    let colon_pos = find_rule_colon(trimmed)?;
    let before_colon = &trimmed[..colon_pos];

    // Check for "returns" clause
    if before_colon.contains(" returns ") {
        let parts: Vec<&str> = before_colon.split(" returns ").collect();
        if parts.len() == 2 {
            let name = parts[0].trim().to_owned();
            let returns_type = Some(parts[1].trim().to_owned());
            if is_valid_rule_name(&name) {
                return Some((name, returns_type));
            }
        }
    } else {
        // Simple rule without returns clause
        let name = before_colon.split_whitespace().next()?.to_owned();
        if is_valid_rule_name(&name) {
            return Some((name, None));
        }
    }

    None
}

/// Find the position of the rule definition colon (not part of ::).
fn find_rule_colon(s: &str) -> Option<usize> {
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == ':' {
            // Check if this is part of :: (namespace separator)
            let is_double_colon =
                (i > 0 && chars.get(i - 1) == Some(&':')) || chars.get(i + 1) == Some(&':');
            if !is_double_colon {
                return Some(i);
            }
        }
    }
    None
}

/// Check if a string is a valid rule name.
fn is_valid_rule_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_uppercase())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

// ===========================================================================
// Normalized grammar IR (rule body -> expression tree) — LANG-05.
//
// This is a NEW, purely additive parser: `parse_xtext_rules` above only
// captures rule *metadata* (name/returns/flags) and is left byte-untouched.
// `parse_grammar_ir` lowers each rule *body* into a serializable expression
// tree that `tools/spec-index` normalizes into language-pack cards. It is
// never reached from `build.rs` or the coverage panics in `spec_validation`,
// so it cannot change any generated `.generated.rs` output.
//
// The serialized shape matches the `irNode` definition in
// the removed language-pack schema
// (op-tagged, kebab-cased). See language-pack-design.md section 5.
// ===========================================================================

use serde::{Deserialize, Serialize};

/// One normalized grammar-IR node. `#[serde(tag = "op")]` + kebab-case renders
/// exactly the schema `irNode` variants (`rule-ref`, `zero-or-more`, ...).
///
/// Honesty gate (design section 5.3): any construct the body parser cannot
/// classify becomes an [`IrNode::Unknown`] carrying the verbatim fragment —
/// never dropped. `unknown` nodes are counted, not silently swallowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum IrNode {
    /// Ordered concatenation of terms.
    Sequence { items: Vec<IrNode> },
    /// `|`-separated alternatives.
    Choice { items: Vec<IrNode> },
    /// Xtext `&` unordered group.
    UnorderedGroup { items: Vec<IrNode> },
    /// `?` cardinality over one item.
    Optional { item: Box<IrNode> },
    /// `*` cardinality over one item.
    ZeroOrMore { item: Box<IrNode> },
    /// `+` cardinality over one item.
    OneOrMore { item: Box<IrNode> },
    /// A `'...'` literal token; also feeds card `keywords`.
    Keyword { value: String },
    /// Reference to a terminal/lexer rule (e.g. `ID`, `STRING_VALUE`).
    TerminalRef { name: String },
    /// Reference to a parser rule; contributes a `rule` dependency edge.
    RuleRef { name: String },
    /// Xtext `[Type]` cross-reference; contributes a `crossref` dependency edge.
    CrossRef { name: String },
    /// Xtext `feature=` / `feature+=` / `feature?=` assignment.
    Assignment {
        feature: String,
        operator: String,
        item: Box<IrNode>,
    },
    /// Xtext `{Action}` metadata, preserved verbatim.
    Action { raw: String },
    /// Xtext `=>` / `->` syntactic-predicate metadata, preserved verbatim.
    Predicate { raw: String },
    /// A construct the parser could not classify — kept verbatim.
    Unknown { raw: String, reason: String },
}

impl IrNode {
    /// Accumulate rule/cross-ref dependency names (first-appearance order,
    /// deduped) and count `unknown` nodes. Terminal-refs are lexical and are
    /// deliberately excluded from `dependencies` (design section 5.4).
    fn walk(
        &self,
        deps: &mut Vec<String>,
        seen: &mut std::collections::HashSet<String>,
        unknown: &mut usize,
    ) {
        match self {
            IrNode::RuleRef { name } | IrNode::CrossRef { name } => {
                if seen.insert(name.clone()) {
                    deps.push(name.clone());
                }
            }
            IrNode::Unknown { .. } => *unknown += 1,
            IrNode::Sequence { items }
            | IrNode::Choice { items }
            | IrNode::UnorderedGroup { items } => {
                for it in items {
                    it.walk(deps, seen, unknown);
                }
            }
            IrNode::Optional { item }
            | IrNode::ZeroOrMore { item }
            | IrNode::OneOrMore { item }
            | IrNode::Assignment { item, .. } => item.walk(deps, seen, unknown),
            IrNode::Keyword { .. }
            | IrNode::TerminalRef { .. }
            | IrNode::Action { .. }
            | IrNode::Predicate { .. } => {}
        }
    }
}

/// One rule lowered to grammar IR: metadata plus the body expression tree,
/// its flattened rule/cross-ref dependencies, and its `unknown` node count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarRuleIr {
    /// Grammar section: `kerml` | `sysml` | `expressions`.
    pub grammar: String,
    /// Rule name.
    pub rule: String,
    /// `parser-rule` | `fragment-rule` | `terminal-rule` | `enum-rule`.
    pub kind: String,
    /// The `returns` type if declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns_type: Option<String>,
    /// Root of the body expression tree.
    pub expression: IrNode,
    /// Flattened rule/cross-ref dependency names (deduped, first-appearance).
    pub dependencies: Vec<String>,
    /// Count of `unknown` nodes in `expression` (must be 0 for a `validated`
    /// parse claim, design section 5.3).
    pub unknown_count: usize,
}

/// Lex tokens for a grammar rule body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GTok {
    Keyword(String),
    CrossRef(String),
    Action(String),
    Ident(String),
    LParen,
    RParen,
    Pipe,
    Question,
    Star,
    Plus,
    Amp,
    Assign,
    PlusAssign,
    QuestionAssign,
    Predicate(String),
    Semicolon,
    Unknown(String),
}

/// A terminal/lexer-rule name has no lowercase letters (e.g. `ID`,
/// `STRING_VALUE`, `REGULAR_COMMENT`). Parser rules like `Name`,
/// `Identification`, `ActionBody` carry lowercase and are `rule-ref`s.
fn is_terminal_ref_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && !name.chars().any(|c| c.is_ascii_lowercase())
}

/// Tokenize a rule body up to (and including) the first bare `;`. Keywords,
/// cross-refs, and actions are captured as whole units so a `';'` keyword can
/// never be mistaken for the rule terminator.
fn lex_grammar_body(src: &str) -> Vec<GTok> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Whitespace.
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Line comment.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Keyword literal '...'.
        if c == '\'' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                s.push(chars[i]);
                i += 1;
            }
            i += 1; // closing quote
            toks.push(GTok::Keyword(s));
            continue;
        }
        // Cross-reference [Type | QualifiedName].
        if c == '[' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != ']' {
                s.push(chars[i]);
                i += 1;
            }
            i += 1; // closing bracket
            // First type name before a '|', simple name after last '::'.
            let first = s.split('|').next().unwrap_or("").trim();
            let simple = first.rsplit("::").next().unwrap_or(first).trim();
            toks.push(GTok::CrossRef(simple.to_owned()));
            continue;
        }
        // Action {Type ...}.
        if c == '{' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '}' {
                s.push(chars[i]);
                i += 1;
            }
            i += 1; // closing brace
            toks.push(GTok::Action(s.trim().to_owned()));
            continue;
        }
        match c {
            '(' => {
                toks.push(GTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(GTok::RParen);
                i += 1;
            }
            '|' => {
                toks.push(GTok::Pipe);
                i += 1;
            }
            '&' => {
                toks.push(GTok::Amp);
                i += 1;
            }
            '?' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(GTok::QuestionAssign);
                    i += 2;
                } else {
                    toks.push(GTok::Question);
                    i += 1;
                }
            }
            '+' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(GTok::PlusAssign);
                    i += 2;
                } else {
                    toks.push(GTok::Plus);
                    i += 1;
                }
            }
            '*' => {
                toks.push(GTok::Star);
                i += 1;
            }
            '=' => {
                if chars.get(i + 1) == Some(&'>') {
                    toks.push(GTok::Predicate("=>".to_owned()));
                    i += 2;
                } else {
                    toks.push(GTok::Assign);
                    i += 1;
                }
            }
            '-' if chars.get(i + 1) == Some(&'>') => {
                toks.push(GTok::Predicate("->".to_owned()));
                i += 2;
            }
            ';' => {
                toks.push(GTok::Semicolon);
                break; // body ends at the first bare semicolon
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = String::new();
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_')
                {
                    s.push(chars[i]);
                    i += 1;
                }
                toks.push(GTok::Ident(s));
            }
            _ => {
                toks.push(GTok::Unknown(c.to_string()));
                i += 1;
            }
        }
    }
    toks
}

/// Recursive-descent parser over [`GTok`] producing an [`IrNode`].
struct GrammarBodyParser<'a> {
    toks: &'a [GTok],
    pos: usize,
}

impl<'a> GrammarBodyParser<'a> {
    fn peek(&self) -> Option<&GTok> {
        self.toks.get(self.pos)
    }

    fn advance(&mut self) -> Option<&GTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// `choice = sequence ('|' sequence)*`.
    fn parse_choice(&mut self) -> IrNode {
        let mut branches = vec![self.parse_sequence()];
        while matches!(self.peek(), Some(GTok::Pipe)) {
            self.pos += 1;
            branches.push(self.parse_sequence());
        }
        if branches.len() == 1 {
            branches.pop().unwrap_or(IrNode::Sequence { items: Vec::new() })
        } else {
            IrNode::Choice { items: branches }
        }
    }

    /// `sequence = term*` until a `|`, `)`, `;`, or end. A `&` separator
    /// promotes the run to an unordered group. A single-term sequence is
    /// unwrapped so groups collapse to their content.
    fn parse_sequence(&mut self) -> IrNode {
        let mut items = Vec::new();
        let mut unordered = false;
        loop {
            match self.peek() {
                None | Some(GTok::Pipe) | Some(GTok::RParen) | Some(GTok::Semicolon) => break,
                Some(GTok::Amp) => {
                    unordered = true;
                    self.pos += 1;
                }
                _ => items.push(self.parse_term()),
            }
        }
        if unordered {
            IrNode::UnorderedGroup { items }
        } else if items.len() == 1 {
            items.pop().unwrap_or(IrNode::Sequence { items: Vec::new() })
        } else {
            IrNode::Sequence { items }
        }
    }

    /// `term = [ident ('='|'+='|'?=')] postfix(primary)`.
    fn parse_term(&mut self) -> IrNode {
        if let Some(GTok::Ident(feature)) = self.peek() {
            if let Some(op) = self.toks.get(self.pos + 1).and_then(assign_op_str) {
                let feature = feature.clone();
                self.pos += 2; // ident + operator
                let primary = self.parse_primary();
                let item = self.parse_postfix(primary);
                return IrNode::Assignment {
                    feature,
                    operator: op.to_owned(),
                    item: Box::new(item),
                };
            }
        }
        let primary = self.parse_primary();
        self.parse_postfix(primary)
    }

    /// Apply trailing `?`/`*`/`+` cardinality wrappers.
    fn parse_postfix(&mut self, mut node: IrNode) -> IrNode {
        loop {
            node = match self.peek() {
                Some(GTok::Question) => IrNode::Optional {
                    item: Box::new(node),
                },
                Some(GTok::Star) => IrNode::ZeroOrMore {
                    item: Box::new(node),
                },
                Some(GTok::Plus) => IrNode::OneOrMore {
                    item: Box::new(node),
                },
                _ => return node,
            };
            self.pos += 1;
        }
    }

    fn parse_primary(&mut self) -> IrNode {
        match self.advance() {
            Some(GTok::Keyword(v)) => IrNode::Keyword { value: v.clone() },
            Some(GTok::CrossRef(n)) => IrNode::CrossRef { name: n.clone() },
            Some(GTok::Action(r)) => IrNode::Action { raw: r.clone() },
            Some(GTok::Predicate(r)) => IrNode::Predicate { raw: r.clone() },
            Some(GTok::Ident(n)) => {
                if is_terminal_ref_name(n) {
                    IrNode::TerminalRef { name: n.clone() }
                } else {
                    IrNode::RuleRef { name: n.clone() }
                }
            }
            Some(GTok::LParen) => {
                let inner = self.parse_choice();
                // Consume the matching ')' if present.
                if matches!(self.peek(), Some(GTok::RParen)) {
                    self.pos += 1;
                }
                inner
            }
            Some(GTok::Unknown(r)) => IrNode::Unknown {
                raw: r.clone(),
                reason: "unrecognized grammar token".to_owned(),
            },
            other => IrNode::Unknown {
                raw: other.map(|t| format!("{t:?}")).unwrap_or_default(),
                reason: "unexpected token in primary position".to_owned(),
            },
        }
    }
}

fn assign_op_str(t: &GTok) -> Option<&'static str> {
    match t {
        GTok::Assign => Some("="),
        GTok::PlusAssign => Some("+="),
        GTok::QuestionAssign => Some("?="),
        _ => None,
    }
}

/// Parse a single rule body (the text after the rule-defining colon) into IR.
/// `grammar`/`name`/`kind`/`returns_type` are the already-extracted metadata.
pub fn parse_grammar_ir_body(
    grammar: &str,
    name: &str,
    kind: &str,
    returns_type: Option<String>,
    body_src: &str,
) -> GrammarRuleIr {
    let toks = lex_grammar_body(body_src);
    let mut parser = GrammarBodyParser {
        toks: &toks,
        pos: 0,
    };
    let expression = parser.parse_choice();
    let mut dependencies = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unknown_count = 0;
    expression.walk(&mut dependencies, &mut seen, &mut unknown_count);
    GrammarRuleIr {
        grammar: grammar.to_owned(),
        rule: name.to_owned(),
        kind: kind.to_owned(),
        returns_type,
        expression,
        dependencies,
        unknown_count,
    }
}

/// Header of a grammar rule: `(name, returns_type, kind)`. Headers are the
/// non-indented lines carrying a rule-defining `:`; bodies are indented.
fn grammar_rule_header(line: &str) -> Option<(String, Option<String>, &'static str)> {
    if !line.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let (kind, rest) = if let Some(r) = line.strip_prefix("terminal ") {
        ("terminal-rule", r)
    } else if let Some(r) = line.strip_prefix("fragment ") {
        ("fragment-rule", r)
    } else if let Some(r) = line.strip_prefix("enum ") {
        ("enum-rule", r)
    } else {
        ("parser-rule", line)
    };
    let colon = find_rule_colon(rest)?;
    let before = rest.get(..colon)?;
    if before.starts_with("grammar") || before.starts_with("import") {
        return None;
    }
    let (name, returns_type) = if let Some((n, ret)) = before.split_once(" returns ") {
        (n.trim().to_owned(), Some(ret.trim().to_owned()))
    } else {
        (before.split_whitespace().next()?.to_owned(), None)
    };
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((name, returns_type, kind))
}

/// Parse every rule in a grammar file into normalized grammar IR. `grammar`
/// is the section label (`kerml` | `sysml` | `expressions`). Reuses the same
/// non-indented-header discipline as [`crate::xtext_parser`]'s metadata parse;
/// each rule body runs through [`parse_grammar_ir_body`].
pub fn parse_grammar_ir(grammar: &str, src: &str) -> Vec<GrammarRuleIr> {
    let lines: Vec<&str> = src.lines().collect();
    // Locate every rule header line.
    let mut headers: Vec<(usize, String, Option<String>, &'static str)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if let Some((name, ret, kind)) = grammar_rule_header(line) {
            headers.push((idx, name, ret, kind));
        }
    }
    let mut out = Vec::with_capacity(headers.len());
    for (h, (start, name, ret, kind)) in headers.iter().enumerate() {
        let next = headers.get(h + 1).map_or(lines.len(), |(n, ..)| *n);
        // Body text = everything after the rule colon on the header line, plus
        // subsequent lines up to the next header. The lexer stops at the first
        // bare ';'.
        let header_line = lines[*start];
        let mut body = String::new();
        if let Some(colon) = find_rule_colon(strip_rule_prefix(header_line)) {
            // Recompute colon offset in the ORIGINAL header line (prefix kept).
            let prefix_len = header_line.len() - strip_rule_prefix(header_line).len();
            if let Some(after) = header_line.get(prefix_len + colon + 1..) {
                body.push_str(after);
                body.push('\n');
            }
        }
        for line in lines.get(start + 1..next).unwrap_or_default() {
            body.push_str(line);
            body.push('\n');
        }
        out.push(parse_grammar_ir_body(grammar, name, kind, ret.clone(), &body));
    }
    out
}

/// Strip a leading `terminal `/`fragment `/`enum ` keyword so `find_rule_colon`
/// sees the same slice `grammar_rule_header` used.
fn strip_rule_prefix(line: &str) -> &str {
    for p in ["terminal ", "fragment ", "enum "] {
        if let Some(r) = line.strip_prefix(p) {
            return r;
        }
    }
    line
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_symbols() {
        let body = "'==' | '!=' | '===' | '!=='";
        let symbols = extract_symbols(body);
        assert_eq!(symbols, vec!["==", "!=", "===", "!=="]);
    }

    #[test]
    fn test_extract_symbols_single() {
        let body = "'if'";
        let symbols = extract_symbols(body);
        assert_eq!(symbols, vec!["if"]);
    }

    #[test]
    fn test_derive_category() {
        assert_eq!(derive_category("EqualityOperator"), "equality");
        assert_eq!(derive_category("NullCoalescingOperator"), "null_coalescing");
        assert_eq!(derive_category("ConditionalAndOperator"), "conditional_and");
    }

    #[test]
    fn test_parse_operators() {
        let content = r#"
ConditionalOperator :
    'if'
;

EqualityOperator :
    '==' | '!=' | '===' | '!=='
;
"#;
        let operators = parse_xtext_operators(content);
        assert_eq!(operators.len(), 2);

        // Sorted by precedence
        assert_eq!(operators[0].name, "ConditionalOperator");
        assert_eq!(operators[0].symbols, vec!["if"]);
        assert_eq!(operators[0].category, "conditional");
        assert_eq!(operators[0].precedence, 1);

        assert_eq!(operators[1].name, "EqualityOperator");
        assert_eq!(operators[1].symbols, vec!["==", "!=", "===", "!=="]);
        assert_eq!(operators[1].category, "equality");
        assert_eq!(operators[1].precedence, 7);
    }

    #[test]
    fn test_parse_enums() {
        let content = r#"
enum FeatureDirection returns SysML::FeatureDirectionKind :
    in | out | inout
;
"#;
        let enums = parse_xtext_enums(content);
        assert_eq!(enums.len(), 1);

        assert_eq!(enums[0].name, "FeatureDirection");
        assert_eq!(enums[0].returns_type, "SysML::FeatureDirectionKind");
        assert_eq!(
            enums[0].values,
            vec![
                ("in".to_owned(), "in".to_owned()),
                ("out".to_owned(), "out".to_owned()),
                ("inout".to_owned(), "inout".to_owned()),
            ]
        );
    }

    #[test]
    fn test_parse_rules() {
        let content = r#"
Package returns SysML::Package :
    PackageDeclaration PackageBody
;

terminal DECIMAL_VALUE returns Ecore::EInt:
    '0'..'9' ('0'..'9')*;

fragment Identification returns SysML::Element :
    '<' declaredShortName = Name '>'
;
"#;
        let rules = parse_xtext_rules(content);

        // Find the rules by name
        let package = rules.iter().find(|r| r.name == "Package");
        assert!(package.is_some());
        let package = package.unwrap();
        assert_eq!(package.returns_type, Some("SysML::Package".to_owned()));
        assert!(!package.is_fragment);
        assert!(!package.is_terminal);

        let decimal = rules.iter().find(|r| r.name == "DECIMAL_VALUE");
        assert!(decimal.is_some());
        let decimal = decimal.unwrap();
        assert!(decimal.is_terminal);

        let ident = rules.iter().find(|r| r.name == "Identification");
        assert!(ident.is_some());
        let ident = ident.unwrap();
        assert!(ident.is_fragment);
    }

    #[test]
    fn test_parse_rule_header() {
        let (name, returns) = parse_rule_header("Package returns SysML::Package :").unwrap();
        assert_eq!(name, "Package");
        assert_eq!(returns, Some("SysML::Package".to_owned()));

        let (name, returns) = parse_rule_header("SimpleRule :").unwrap();
        assert_eq!(name, "SimpleRule");
        assert_eq!(returns, None);

        assert!(parse_rule_header("// comment").is_none());
        assert!(parse_rule_header("no colon here").is_none());
    }

    #[test]
    fn parse_rule_header_edge_cases() {
        assert!(parse_rule_header("").is_none());
        assert!(parse_rule_header(":").is_none()); // colon but no name
        assert!(parse_rule_header("a :").is_none()); // lowercase name
        assert!(parse_rule_header("A :").is_some()); // single uppercase letter is valid
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod grammar_ir_tests {
    use super::*;

    /// Count keyword / optional / assignment / unknown nodes anywhere in a tree.
    fn tally(node: &IrNode, kw: &mut usize, opt: &mut usize, asn: &mut usize, unk: &mut usize) {
        match node {
            IrNode::Keyword { .. } => *kw += 1,
            IrNode::Optional { item } => {
                *opt += 1;
                tally(item, kw, opt, asn, unk);
            }
            IrNode::Assignment { item, .. } => {
                *asn += 1;
                tally(item, kw, opt, asn, unk);
            }
            IrNode::Unknown { .. } => *unk += 1,
            IrNode::Sequence { items }
            | IrNode::Choice { items }
            | IrNode::UnorderedGroup { items } => {
                for it in items {
                    tally(it, kw, opt, asn, unk);
                }
            }
            IrNode::ZeroOrMore { item } | IrNode::OneOrMore { item } => {
                tally(item, kw, opt, asn, unk);
            }
            IrNode::TerminalRef { .. }
            | IrNode::RuleRef { .. }
            | IrNode::CrossRef { .. }
            | IrNode::Action { .. }
            | IrNode::Predicate { .. } => {}
        }
    }

    // Real SysML.xtext TransitionUsage body (design's canonical example).
    const TRANSITION_USAGE_BODY: &str = r#"
	TransitionUsageKeyword ( UsageDeclaration? 'first' )?
	ownedRelationship += TransitionSourceMember
	ownedRelationship += EmptyParameterMember
	( ownedRelationship += EmptyParameterMember
	  ownedRelationship += TriggerActionMember )?
	( ownedRelationship += GuardExpressionMember )?
	( ownedRelationship += EffectBehaviorMember )?
	'then' ownedRelationship += TransitionSuccessionMember
	ActionBody
;
"#;

    #[test]
    fn transition_usage_lowers_to_sequence_with_no_unknowns() {
        let ir = parse_grammar_ir_body(
            "sysml",
            "TransitionUsage",
            "parser-rule",
            Some("SysML::TransitionUsage".to_owned()),
            TRANSITION_USAGE_BODY,
        );
        // AC3: sequence root, >=1 keyword, >=1 optional, deps non-empty & all resolve, zero unknowns.
        assert!(
            matches!(ir.expression, IrNode::Sequence { .. }),
            "root should be a sequence, got {:?}",
            ir.expression
        );
        assert_eq!(ir.unknown_count, 0, "TransitionUsage must have zero unknown nodes");
        let (mut kw, mut opt, mut asn, mut unk) = (0, 0, 0, 0);
        tally(&ir.expression, &mut kw, &mut opt, &mut asn, &mut unk);
        assert!(kw >= 1, "expected >=1 keyword ('first'/'then')");
        assert!(opt >= 1, "expected >=1 optional");
        assert!(asn >= 1, "expected assignments (ownedRelationship +=)");
        assert_eq!(unk, 0);
        assert!(ir.dependencies.contains(&"TransitionSourceMember".to_owned()));
        assert!(ir.dependencies.contains(&"ActionBody".to_owned()));
        assert!(ir.dependencies.contains(&"TransitionUsageKeyword".to_owned()));
    }

    #[test]
    fn choice_heavy_rule_lowers_to_choice() {
        // Real KerML.xtext AnnotatingElement.
        let body = "  Comment | Documentation | TextualRepresentation | MetadataFeature\n;";
        let ir = parse_grammar_ir_body("kerml", "AnnotatingElement", "parser-rule", None, body);
        match &ir.expression {
            IrNode::Choice { items } => {
                assert_eq!(items.len(), 4);
                assert!(items.iter().all(|n| matches!(n, IrNode::RuleRef { .. })));
            }
            other => panic!("expected choice, got {other:?}"),
        }
        assert_eq!(ir.unknown_count, 0);
        assert_eq!(ir.dependencies.len(), 4);
    }

    #[test]
    fn assignment_and_crossref_are_captured() {
        // Real KerML.xtext FeatureTyping (full concrete syntax).
        let body = r#"
	( 'specialization' Identification? )?
    'typing' typedFeature = [SysML::Feature | QualifiedName]
    (':' | 'typed' 'by') FeatureType
    RelationshipBody
;
"#;
        let ir = parse_grammar_ir_body("kerml", "FeatureTyping", "parser-rule", None, body);
        assert_eq!(ir.unknown_count, 0);
        // The cross-ref target simple name is a dependency.
        assert!(ir.dependencies.contains(&"Feature".to_owned()));
        // Find the typedFeature assignment and confirm its RHS is a cross-ref.
        fn find_assignment<'a>(n: &'a IrNode, feat: &str) -> Option<&'a IrNode> {
            match n {
                IrNode::Assignment { feature, item, .. } if feature == feat => Some(item),
                IrNode::Sequence { items } | IrNode::Choice { items } | IrNode::UnorderedGroup { items } => {
                    items.iter().find_map(|c| find_assignment(c, feat))
                }
                IrNode::Optional { item } | IrNode::ZeroOrMore { item } | IrNode::OneOrMore { item } | IrNode::Assignment { item, .. } => {
                    find_assignment(item, feat)
                }
                _ => None,
            }
        }
        let rhs = find_assignment(&ir.expression, "typedFeature").expect("typedFeature assignment");
        assert!(matches!(rhs, IrNode::CrossRef { name } if name == "Feature"));
    }

    #[test]
    fn feature_typing_diverges_kerml_vs_sysml_but_comment_is_equal() {
        // Split-set (design section 6.2): FeatureTyping bodies differ.
        let kerml_ft = r#"
	( 'specialization' Identification? )?
    'typing' typedFeature = [SysML::Feature | QualifiedName]
    (':' | 'typed' 'by') FeatureType
    RelationshipBody
;
"#;
        let sysml_ft = "\tOwnedFeatureTyping | ConjugatedPortTyping\n;";
        let k = parse_grammar_ir_body("kerml", "FeatureTyping", "parser-rule", None, kerml_ft);
        let s = parse_grammar_ir_body("sysml", "FeatureTyping", "parser-rule", None, sysml_ft);
        assert_ne!(
            k.expression, s.expression,
            "FeatureTyping must lower to DIFFERENT IR across grammars (split-set)"
        );

        // Merge-set: Comment is byte-identical in both grammars -> IR-equal.
        let comment_body = r#"
	( 'comment' Identification?
	  ('about' ownedRelationship += Annotation
	     ( ',' ownedRelationship += Annotation )* )?
	)?
	( 'locale' locale = STRING_VALUE )?
	body = REGULAR_COMMENT
;
"#;
        let ck = parse_grammar_ir_body("kerml", "Comment", "parser-rule", None, comment_body);
        let cs = parse_grammar_ir_body("sysml", "Comment", "parser-rule", None, comment_body);
        assert_eq!(
            ck.expression, cs.expression,
            "Comment must lower to EQUAL IR across grammars (merge-set)"
        );
        assert_eq!(ck.unknown_count, 0);
        // STRING_VALUE / REGULAR_COMMENT are terminal-refs, not dependencies.
        assert!(!ck.dependencies.contains(&"STRING_VALUE".to_owned()));
        assert!(!ck.dependencies.contains(&"REGULAR_COMMENT".to_owned()));
        assert!(ck.dependencies.contains(&"Annotation".to_owned()));
    }

    #[test]
    fn parse_grammar_ir_extracts_rule_bodies_from_a_mini_grammar() {
        let grammar = r#"
grammar org.example with org.example.Base

Package returns SysML::Package :
    'package' declaredName = Name PackageBody
;

terminal ID :
    ('a'..'z')+
;
"#;
        let rules = parse_grammar_ir("sysml", grammar);
        let pkg = rules.iter().find(|r| r.rule == "Package").expect("Package rule");
        assert_eq!(pkg.kind, "parser-rule");
        assert_eq!(pkg.returns_type.as_deref(), Some("SysML::Package"));
        assert!(pkg.dependencies.contains(&"Name".to_owned()));
        assert!(pkg.dependencies.contains(&"PackageBody".to_owned()));
        let id = rules.iter().find(|r| r.rule == "ID").expect("ID terminal");
        assert_eq!(id.kind, "terminal-rule");
        // The 'grammar' declaration line is not a rule.
        assert!(rules.iter().all(|r| r.rule != "grammar"));
    }

    #[test]
    fn ir_serializes_to_schema_op_shape() {
        let node = IrNode::Sequence {
            items: vec![
                IrNode::Keyword { value: "transition".to_owned() },
                IrNode::Optional { item: Box::new(IrNode::RuleRef { name: "Identification".to_owned() }) },
                IrNode::Assignment {
                    feature: "body".to_owned(),
                    operator: "+=".to_owned(),
                    item: Box::new(IrNode::TerminalRef { name: "ID".to_owned() }),
                },
            ],
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""op":"sequence""#));
        assert!(json.contains(r#""op":"keyword""#));
        assert!(json.contains(r#""op":"optional""#));
        assert!(json.contains(r#""op":"rule-ref""#));
        assert!(json.contains(r#""op":"assignment""#));
        assert!(json.contains(r#""op":"terminal-ref""#));
        assert!(json.contains(r#""operator":"+=""#));
    }
}
