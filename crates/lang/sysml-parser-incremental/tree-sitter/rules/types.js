/**
 * Type reference, name, literal, and identifier rules.
 */
module.exports = {
  type_ref: ($) =>
    choice(
      seq("~", choice($.qualified_name, $._name)),
      $.qualified_name,
      $.feature_chain,
      $._name
    ),

  // Spec: QualifiedName = GlobalQualification? Qualification? Name
  // GlobalQualification = '$' '::'
  // Qualification = (Name '::')+
  qualified_name: ($) =>
    prec.left(
      seq(
        optional($.global_qualification),
        $._name,
        repeat1(seq("::", $._name))
      )
    ),

  // Spec: KerMLExpressions.xtext:541-542 — GlobalQualification = '$' '::'
  global_qualification: ($) =>
    seq("$", "::"),

  _name: ($) =>
    choice(
      $.identifier,
      $.quoted_name,
      // Keywords that are also used as feature names in the standard library
      "type", "start",
      "self", "this", "that",
      // usage_prefix keyword that may appear as a feature name
      // (e.g., `port composite : CompositePort;` where composite is the port name).
      // alias() ensures it appears as an identifier node in the tree, so
      // child_by_field_name("name") works in the Rust AST builder.
      alias("composite", $.identifier),
    ),

  // Keywords used as feature names in the standard library (for feature_chain only)
  _keyword_name: ($) => choice("entry", "exit", "do", "accept"),

  short_name: ($) =>
    seq("<", $._name, ">"),

  // Spec: KerMLExpressions.xtext:564 — UNRESTRICTED_NAME with escape sequences
  // '\'' ('\\' ('b'|'t'|'n'|'f'|'r'|'"'|"'"|'\\') | !('\\'|'\''))* '\''
  quoted_name: ($) =>
    seq("'", /([^'\\]|\\[btnfr"'\\])*/, "'"),

  identifier: ($) =>
    token(/[A-Za-z_][A-Za-z0-9_]*/),

  // Literals
  literal: ($) =>
    choice(
      $.string_literal,
      $.integer_literal,
      $.real_literal,
      $.boolean_literal,
      $.null_literal,
      $.infinity_literal
    ),

  string_literal: ($) =>
    /"([^"\\]|\\.)*"/,

  integer_literal: ($) =>
    /[0-9]+/,

  // Spec: KerMLExpressions.xtext:527-528 — RealValue = DECIMAL_VALUE? '.' (DECIMAL_VALUE | EXP_VALUE) | EXP_VALUE
  // The DECIMAL_VALUE before the dot is optional, allowing leading-dot forms like .5, .5e3
  real_literal: ($) =>
    choice(
      /[0-9]*\.[0-9]+([eE][+-]?[0-9]+)?/,
      /[0-9]+[eE][+-]?[0-9]+/,
    ),

  number: ($) =>
    choice($.integer_literal, $.real_literal),

  boolean_literal: ($) =>
    choice("true", "false"),

  null_literal: ($) =>
    "null",

  // Spec: KerMLExpressions.xtext:531-532 — LiteralInfinity = '*'
  infinity_literal: ($) =>
    "*",
};
