/**
 * Expression rules: all binary/unary/primary expressions, arrows, invocations, etc.
 *
 * LAMBDA ARCHITECTURE (Feb 2026):
 * The xtext spec defines lambda bodies as CalculationBody — the same as function/calc
 * bodies. "in x;" inside a lambda is just a DefaultReferenceUsage with direction=in,
 * parsed as a regular body member. This grammar follows the same approach:
 * arrow_body_expression uses function_body, which handles all lambda patterns
 * (doc comments, in/ref parameters with bodies, result expressions) uniformly.
 */
const { PREC, binaryExprLeft, binaryExprRight } = require("../helpers/patterns");

module.exports = {
  _expression: ($) =>
    choice(
      $.conditional_expression,
      $.null_coalesce_expression,
      $.implies_expression,
      $.or_expression,
      $.xor_expression,
      $.and_expression,
      $.equality_expression,
      $.classification_expression,
      $.relational_expression,
      $.range_expression,
      $.additive_expression,
      $.multiplicative_expression,
      $.exponentiation_expression,
      $.unary_expression,
      $.primary_expression
    ),

  conditional_expression: ($) =>
    prec.right(
      PREC.CONDITIONAL,
      seq(
        "if",
        field("condition", $._expression),
        "?",
        field("then", $._expression),
        "else",
        field("else", $._expression)
      )
    ),

  // Table-driven binary expressions
  null_coalesce_expression:   binaryExprLeft(PREC.NULL_COALESCE, ["??"]),
  implies_expression:         binaryExprLeft(PREC.IMPLIES, ["implies"]),
  or_expression:              binaryExprLeft(PREC.OR, ["|", "or"]),
  xor_expression:             binaryExprLeft(PREC.XOR, ["xor"]),
  and_expression:             binaryExprLeft(PREC.AND, ["&", "and"]),
  equality_expression:        binaryExprLeft(PREC.EQUALITY, ["==", "!=", "===", "!=="]),
  relational_expression:      binaryExprLeft(PREC.RELATIONAL, ["<", ">", "<=", ">="]),
  range_expression:           binaryExprLeft(PREC.RANGE, [".."]),
  additive_expression:        binaryExprLeft(PREC.ADDITIVE, ["+", "-"]),
  multiplicative_expression:  binaryExprLeft(PREC.MULTIPLICATIVE, ["*", "/", "%"]),
  exponentiation_expression:  binaryExprRight(PREC.EXPONENTIATION, ["**", "^"]),

  // Classification is special: RHS is type_ref, not _expression
  classification_expression: ($) =>
    prec.left(
      PREC.CLASSIFICATION,
      seq(
        $._expression,
        choice("hastype", "istype", "@", "@@", "as", "meta"),
        $.type_ref
      )
    ),

  unary_expression: ($) =>
    prec(
      PREC.UNARY,
      seq(choice("+", "-", "~", "not"), $._expression)
    ),

  primary_expression: ($) =>
    prec(
      PREC.PRIMARY,
      choice(
        $.literal,
        $.feature_chain,
        $.qualified_name,
        $.invocation_expression,
        $.new_expression,
        $.bracket_expression,
        $.index_expression,
        $.arrow_expression,
        $.arrow_body_expression,
        $.arrow_reduce_expression,
        $.parenthesized_expression,
        $.collect_expression,
        $.select_expression,
        $.member_access
      )
    ),

  new_expression: ($) =>
    seq("new", $.invocation_expression),

  feature_chain: ($) =>
    prec.left(
      PREC.FEATURE_CHAIN,
      seq(
        choice($._name, $._keyword_name),
        repeat(seq(".", choice($._name, $._keyword_name)))
      )
    ),

  // Member access after complex expressions: expr.member
  member_access: ($) =>
    prec.left(
      PREC.FEATURE_CHAIN,
      seq(
        choice($.invocation_expression, $.bracket_expression, $.index_expression, $.arrow_expression, $.arrow_body_expression, $.arrow_reduce_expression, $.parenthesized_expression, $.collect_expression, $.select_expression, $.qualified_name),
        ".",
        $._name,
        repeat(seq(".", $._name))
      )
    ),

  // Arrow expression: expr->name or expr->name(args)
  // The optional (args) is inlined to avoid shift-reduce conflict between
  // bare identifier and invocation_expression in the RHS.
  arrow_expression: ($) =>
    prec.left(
      PREC.ARROW,
      seq(
        choice($.feature_chain, $.qualified_name, $.invocation_expression, $.bracket_expression, $.index_expression, $.member_access, $.parenthesized_expression, $.collect_expression, $.select_expression, $.arrow_body_expression, $.arrow_reduce_expression),
        "->",
        choice($.feature_chain, $.qualified_name, $.member_access),
        optional(seq("(", optional($.argument_list), ")"))
      )
    ),

  // Arrow with body: expr->name { lambda_params* result_expr? }
  // Keep lambda bodies separate from function_body so result expressions that
  // begin with keyword-like names (`item == 0`) don't get greedily reduced as
  // body members such as `item` usages.
  arrow_body_expression: ($) =>
    prec.dynamic(5, prec.left(
      PREC.ARROW + 1,
      seq(
        $.arrow_expression,
        $.lambda_body
      )
    )),

  // Admit inline doc declarations inside lambda/reduction bodies. Per
  // KerML.xtext:1117 FunctionBody → FunctionBodyPart → NonFeatureMember
  // (Comment | Documentation), a `doc /* */` is a valid body member. Without
  // this slot, `alternatives->maximize { doc /* */ in x; eval(x) }` (TradeStudies
  // library) ERRORs, and GLR error-recovery closes the enclosing package early —
  // orphaning later library defs (MaximizeObjective/TradeStudy owner=None).
  lambda_body: ($) =>
    seq("{", repeat(choice($.doc_comment, $.lambda_parameter)), optional($.result_expression), "}"),

  lambda_parameter: ($) =>
    prec(3, seq(
      choice(seq("in", optional("ref")), "out", "inout", "ref"),
      field("name", choice($._name, $._keyword_name)),
      // Lambda-safe specialization set: the optional terminator below makes
      // the symbolic `~` conjugation arm genuinely ambiguous with a unary-`~`
      // result expression here (`{ in x ~Y }`), so this position uses the
      // set WITHOUT that arm (common.js). `conjugates` still parses here.
      repeat($._lambda_feature_specialization),
      optional($.default_value),
      optional($.usage_body),
      optional(";"),
    )),

  // Arrow with reduce pattern: (expr->name) '+' ?? zero
  arrow_reduce_expression: ($) =>
    prec.dynamic(5, prec.left(
      PREC.ARROW + 1,
      seq(
        $.arrow_expression,
        choice($.quoted_name, $.string_literal, $.qualified_name, $.identifier),
        optional(seq("??", $._expression))
      )
    )),

  // Index expression: expr#(args)
  index_expression: ($) =>
    prec.left(
      PREC.PRIMARY,
      seq(
        choice($.feature_chain, $.qualified_name, $.member_access, $.invocation_expression, $.arrow_expression, $.arrow_body_expression, $.parenthesized_expression, $.collect_expression, $.select_expression),
        "#",
        "(",
        optional($.argument_list),
        ")"
      )
    ),

  invocation_expression: ($) =>
    prec(PREC.ARROW + 1, seq(
      choice($.feature_chain, $.qualified_name, $.member_access),
      "(",
      optional($.argument_list),
      ")"
    )),

  argument_list: ($) =>
    seq(
      $._argument,
      repeat(seq(",", $._argument))
    ),

  _argument: ($) =>
    choice(
      $.named_argument,
      $._expression
    ),

  named_argument: ($) =>
    prec(1, seq(
      field("name", $.identifier),
      "=",
      field("value", $._expression)
    )),

  bracket_expression: ($) =>
    prec.left(PREC.PRIMARY, seq(
      choice($.literal, $.feature_chain, $.qualified_name, $.parenthesized_expression, $.index_expression, $.invocation_expression),
      "[",
      $._expression,
      "]"
    )),

  parenthesized_expression: ($) =>
    choice(
      seq("(", ")"),
      seq("(", $._expression, repeat(seq(",", $._expression)), ")"),
    ),

  // Collect/select use function_body for uniform lambda handling
  collect_expression: ($) =>
    seq($.feature_chain, "->", "collect", $.function_body),

  select_expression: ($) =>
    seq($.feature_chain, "->", "select", $.function_body),
};
