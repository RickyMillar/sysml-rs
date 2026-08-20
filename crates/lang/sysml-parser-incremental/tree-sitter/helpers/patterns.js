/**
 * Reusable grammar pattern factories for SysML v2 tree-sitter grammar.
 *
 * These factories reduce duplication by encoding common rule shapes.
 * Each factory returns a rule function ($) => ... suitable for tree-sitter.
 */

// Precedence levels (higher = tighter binding)
const PREC = {
  CONDITIONAL: 1,
  NULL_COALESCE: 2,
  IMPLIES: 3,
  OR: 4,
  XOR: 5,
  AND: 6,
  EQUALITY: 7,
  CLASSIFICATION: 8,
  RELATIONAL: 11,
  RANGE: 12,
  ADDITIVE: 13,
  MULTIPLICATIVE: 14,
  EXPONENTIATION: 15,
  UNARY: 16,
  PRIMARY: 17,
  FEATURE_CHAIN: 18,
  ARROW: 19,
};

// KerML definition keywords (used without 'def' suffix)
const KERML_DEF_KEYWORDS = [
  "classifier", "class", "struct", "datatype", "function", "behavior",
  "interaction", "metaclass", "assoc", "type", "predicate",
];

/**
 * Definition factory: creates a standard *_def rule.
 * 13 of 14 defs follow this exact pattern (enum_def is special).
 *
 * @param {string} keyword - The element keyword (e.g., "part", "action")
 * @param {string} [bodyRule] - Body rule name (default: "definition_body")
 */
function defRule(keyword, bodyRule) {
  return ($) => prec(3, seq(
    optional($.visibility_indicator),
    optional("abstract"),
    keyword, "def",
    optional($.short_name),
    field("name", $._name),
    repeat($._feature_specialization),
    choice(
      seq(bodyRule ? $[bodyRule] : $.definition_body, optional(";")),
      ";",
    ),
  ));
}

/**
 * Left-associative binary expression factory.
 *
 * @param {number} precLevel - Precedence level
 * @param {string[]} ops - Operator symbols
 */
function binaryExprLeft(precLevel, ops) {
  return ($) => prec.left(precLevel, seq($._expression, choice(...ops), $._expression));
}

/**
 * Right-associative binary expression factory.
 *
 * @param {number} precLevel - Precedence level
 * @param {string[]} ops - Operator symbols
 */
function binaryExprRight(precLevel, ops) {
  return ($) => prec.right(precLevel, seq($._expression, choice(...ops), $._expression));
}

/**
 * Comma-separated list (1+).
 */
function commaSep1(rule) {
  return seq(rule, repeat(seq(",", rule)));
}

/**
 * Optional comma-separated list.
 */
function commaSep(rule) {
  return optional(commaSep1(rule));
}

/**
 * Usage factory: creates a standard *_usage rule.
 * Used for usages that share the same structure but differ only in keyword/body.
 *
 * @param {string|string[]} keyword - The element keyword(s) (e.g., "action", ["port", "in", "out", "inout"])
 * @param {string} [bodyRule] - Body rule name (default: "usage_body")
 */
function usageRule(keyword, bodyRule) {
  const kw = Array.isArray(keyword) ? choice(...keyword) : keyword;
  return ($) => prec(2, seq(
    optional($.visibility_indicator),
    optional("abstract"),
    optional($.usage_prefix),
    kw,
    optional($.short_name),
    optional(field("name", $._name)),
    repeat($._feature_specialization),
    optional($.default_value),
    optional(bodyRule ? $[bodyRule] : $.usage_body),
    optional(";"),
  ));
}

module.exports = {
  PREC,
  KERML_DEF_KEYWORDS,
  defRule,
  usageRule,
  binaryExprLeft,
  binaryExprRight,
  commaSep1,
  commaSep,
};
