/**
 * Common rules: visibility, usage_prefix, feature specialization,
 * bodies, comments, documentation, and the _definition_or_usage choice.
 */
const keywords = require("../generated/keywords");

// Extract unique operator symbols sorted by length (longest first)
const operators = require("../generated/operators");
const operatorSymbols = Array.from(
  new Set(operators.flatMap((op) => op.symbols))
).sort((a, b) => b.length - a.length);

// Extract unique enum values
const enums = require("../generated/enums");
const enumValues = Array.from(
  new Set(Object.values(enums).flat())
).sort();

// One member list, two hidden choice rules: the full feature-specialization
// set (which adds the symbolic `~` conjugation arm) and the lambda-safe set
// (which omits it) — see the conjugation_clause note below for why the `~`
// arm must be excluded from lambda-parameter position.
const FEATURE_SPECIALIZATION_MEMBERS = ($) => [
  $.typing,
  $.multiplicity,
  $.multiplicity_modifiers,
  $.supertype_list,
  $.redefinition,
  $.subsets_clause,
  $.references_clause,
  $.intersects_clause,
  $.unions_clause,
  $.differences_clause,
  $.chains_clause,
  $.crosses_clause,
  $.disjoint_clause,
  $.conjugation_clause,
  $.inverting,
  $.featuring,
];

module.exports = {
  // NOTE: Hidden subrules (_usage_header, _usage_name, _def_header) were tried
  // to reduce LR state explosion (Issue #656 pattern) but all caused >65k states
  // due to GLR conflict inflation across 15+ calling contexts. All optionals
  // (vis, abstract, prefix, short_name, name) are kept inline.
  // See TREE_SITTER_STATUS.md for full analysis and references.

  _definition_or_usage: ($) =>
    choice(
      // Merged standard usages (part, attribute, item, occurrence, ref)
      $.standard_usage,
      // Merged standard definitions (part, attribute, port, connection, interface, item, allocation, occurrence, flow)
      $.annotated_connection_def,
      $.standard_def,
      // Bare `individual def X;` → OccurrenceDefinition isIndividual
      // (definitions.js; the prefixed `individual <kind> def` family is a
      // documented residual — see the rule's note).
      $.individual_def,
      // G04b: calc def (peeled from generic `definition` for anonymous-param body)
      $.calc_def,
      // Custom-body definitions
      $.action_def,
      $.action_usage,
      $.state_def,
      $.state_usage,
      // Peeled from the generic `usage` fallback (G23) — see usages.js.
      $.exhibit_state_usage,
      $.transition_usage,
      $.port_usage,
      $.annotated_connection_usage,
      $.connection_usage,
      // Keyword-annotated, anonymous connection end, e.g.
      // `end #original ::> requirement;` (G24 / SysML §7.27.4).
      $.connection_end_usage,
      $.interface_usage,
      $.requirement_def,
      $.requirement_usage,
      // concern/viewpoint peeled out of generic definition/usage (G08f): both
      // bodies are RequirementBody per SysML.xtext:2151/2159/2399/2403.
      $.concern_def,
      $.concern_usage,
      $.viewpoint_def,
      $.viewpoint_usage,
      $.constraint_def,
      $.constraint_usage,
      $.allocation_usage,
      $.enum_def,
      // Standalone EnumerationUsage `enum e : Color;` (usages.js) — after
      // `enum`, the lexer picks the `def` keyword (enum_def) when present,
      // an identifier lands here.
      $.enum_usage,
      $.flow_connection_usage,  // now includes succession flow
      $.message_usage,
      $.succession_usage,
      $.succession_decl,
      $.connector_usage,
      $.binding_usage,
      $.bind_as_usage,
      $.satisfy_requirement,
      $.objective_requirement,
      $.subject_requirement,
      $.dependency_usage,
      $.assert_constraint_usage,
      $.target_transition_usage,
      $.render_usage,
      $.expose_decl,
      // Use case / actor rules (use_case_def merged into case_def)
      $.include_use_case_usage,
      $.case_def,
      $.case_usage,
      $.feature_redefinition,
      $.feature_declaration,
      $.return_feature,
      $.inv_constraint,
      $.disjoining_decl,
      // Standalone Subsetting relationship member (G08e): `subset X subsets Y;`
      // (KerML.xtext:679 — a NonFeatureElement, NOT a usage).
      $.subsetting_decl,
      $.doc_comment,
      // Action control-flow nodes (merged)
      $.control_flow_node,
      // KerML definitions/usages
      $.kerml_definition,
      $.kerml_usage,
      // Generic fallbacks
      $.definition,
      $.usage
    ),

  // Note: "expose" is NOT a visibility kind per spec — it has its own
  // expose_decl rule (Import subtype). The xtext mapped it to visibility
  // as a grammar convenience but semantically it's distinct.
  visibility_indicator: ($) =>
    choice("public", "private", "protected"),

  usage_prefix: ($) =>
    repeat1(choice("readonly", "derived", "end", "ref", "individual", "variation", "variant", "constant", "default", "in", "out", "inout", "composite", "portion", "snapshot", "timeslice")),

  supertype_list: ($) =>
    seq(
      choice(":>", "specializes"),
      $.type_ref,
      repeat(seq(",", $.type_ref))
    ),

  disjoint_clause: ($) =>
    seq("disjoint", optional($.type_ref), "from", $.type_ref, repeat(seq(",", $.type_ref))),

  disjoining_decl: ($) =>
    prec(1, seq(
      optional($.visibility_indicator),
      "disjoint",
      field("source", $.type_ref),
      "from",
      field("target", $.type_ref),
      repeat(seq(",", $.type_ref)),
      optional(";")
    )),

  typing: ($) =>
    seq(":", field("type", $.type_ref), optional($.multiplicity), repeat(seq(",", $.type_ref, optional($.multiplicity)))),

  redefinition: ($) =>
    seq(
      choice(":>>", "redefines"),
      field("target", choice($.qualified_name, $.feature_chain)),
      repeat(seq(",", choice($.qualified_name, $.feature_chain)))
    ),

  multiplicity: ($) =>
    seq(
      "[",
      $._mult_value,
      optional(seq("..", $._mult_value)),
      "]"
    ),

  _mult_value: ($) =>
    choice($.number, "*", $.identifier, $.qualified_name),

  multiplicity_modifiers: ($) =>
    repeat1(choice("ordered", "nonunique")),

  default_value: ($) =>
    choice(
      seq("=", $._expression),
      seq(":=", $._expression),
      seq("default", optional(choice(":=", "=")), choice(
        prec(2, seq("{", $._expression, "}")),
        $._expression
      ))
    ),

  relationship_body: ($) =>
    repeat1(
      choice(
        $.subsets_clause,
        $.redefines_clause,
        $.references_clause
      )
    ),

  subsets_clause: ($) =>
    seq("subsets", choice($.feature_chain, $.qualified_name), repeat(seq(",", choice($.feature_chain, $.qualified_name)))),

  redefines_clause: ($) =>
    seq("redefines", choice($.feature_chain, $.qualified_name), repeat(seq(",", choice($.feature_chain, $.qualified_name)))),

  // KerML ReferencesKeyword is either the symbolic `::>` form or the
  // long `references` spelling (KerML.xtext:608-614).
  references_clause: ($) =>
    seq(choice("::>", "references"), choice($.feature_chain, $.qualified_name), repeat(seq(",", choice($.feature_chain, $.qualified_name)))),

  crosses_clause: ($) =>
    seq("crosses", $.feature_chain),

  unions_clause: ($) =>
    seq("unions", $.feature_chain, repeat(seq(",", $.feature_chain))),

  differences_clause: ($) =>
    seq("differences", $.feature_chain, repeat(seq(",", $.feature_chain))),

  intersects_clause: ($) =>
    seq("intersects", $.type_ref, repeat(seq(",", $.type_ref))),

  chains_clause: ($) =>
    seq("chains", $.feature_chain),

  // ConjugationPart (KerML.xtext:337): `('~' | 'conjugates') OwnedConjugation`.
  // The declaring type is the implicit conjugatedType; the target is the
  // originalType. Distinct from the `: ~Type` conjugated-typing form (types.js).
  //
  // BOTH spellings are admitted. The symbolic `~` arm lives in a SEPARATE
  // rule (`symbolic_conjugation`, aliased back to `conjugation_clause` so the
  // CST — and ast_builder's node-kind dispatch — see exactly one node kind)
  // because it must be EXCLUDED from lambda-parameter position: a
  // lambda_parameter's terminator is optional, so `{ in x ~Y }` is genuinely
  // ambiguous between a conjugation on `x` and the unary-`~` result
  // expression `~Y` — the F3 verbatim tree-sitter conflict on
  // `lambda_parameter`. Everywhere else the specialization repeat is followed
  // by separator-led continuations only, so `~` is unambiguous. KerML's own
  // FunctionBodyPart members carry terminators (KerML.xtext FunctionBody
  // :925-937), so nothing spec-legal is lost in lambda position — the
  // `conjugates` keyword still works there.
  conjugation_clause: ($) =>
    seq("conjugates", $.type_ref),

  // Symbolic spelling of ConjugationPart / ClassifierConjugationPart /
  // FeatureConjugationPart (KerML.xtext :337-339 / :481-485 / :726-728).
  // Only ever used behind alias($.symbolic_conjugation, $.conjugation_clause).
  symbolic_conjugation: ($) =>
    seq("~", $.type_ref),

  _feature_specialization: ($) => choice(
    ...FEATURE_SPECIALIZATION_MEMBERS($),
    alias($.symbolic_conjugation, $.conjugation_clause),
  ),

  // Lambda-parameter position only (expressions.js `lambda_parameter`):
  // the full set MINUS the symbolic `~` conjugation arm. See the
  // conjugation_clause note above.
  _lambda_feature_specialization: ($) =>
    choice(...FEATURE_SPECIALIZATION_MEMBERS($)),

  inverting: ($) =>
    seq("inverse", "of", choice($.feature_chain, $.qualified_name)),

  featuring: ($) =>
    seq("featured", "by", choice($.feature_chain, $.qualified_name), repeat(seq(",", choice($.feature_chain, $.qualified_name)))),

  // PrefixMetadataAnnotation / PrefixMetadataMember (SysML.xtext:129-138):
  // a user-defined keyword is `#` followed by the metadata definition's
  // qualified name, declared name, or short name.
  prefix_metadata_annotation: ($) =>
    seq("#", field("type", $.type_ref)),

  // Metadata annotation: @TypeRef { body } or @TypeRef ;
  metadata_usage: ($) =>
    seq(
      "@",
      field("type", $.type_ref),
      choice(
        seq("{", repeat($._body_member), "}"),
        ";"
      )
    ),

  _body_member: ($) =>
    choice(
      $._definition_or_usage,
      $.import_decl,
      $.alias_decl,
      $.comment_element,
      $.package_decl,
      $.metadata_usage,
      // ElementFilterMembership is a valid Namespace member (SysML.xtext:229-232).
      // Definition/usage bodies are Namespaces too, so `filter ...;` must parse
      // inside e.g. `view def V { filter @SysML::PartUsage; }`. Shared by
      // definition_body / usage_body / function_body / calc_body.
      $.filter_decl
    ),

  definition_body: ($) =>
    seq("{", repeat($._body_member), "}"),

  usage_body: ($) =>
    seq("{", repeat($._body_member), "}"),

  // Function/calc/predicate body: like definition_body but with trailing result expression
  function_body: ($) =>
    seq("{", repeat($._body_member), optional($.result_expression), "}"),

  // G04b: calc-def body. A peer of function_body that ALSO admits anonymous typed
  // parameters (`in : Real[1];`, no name — KerML.xtext:549, used throughout the
  // KerML Vector/Tensor `calc def` library). Kept SEPARATE from the shared
  // function_body (used by analysis/view/kerml-defs/calc-usage) and reachable ONLY
  // from calc_def — so adding the anonymous form here does NOT perturb the
  // function_body/definition_body disambiguation of every other brace-bodied def.
  calc_body: ($) =>
    seq("{", repeat(choice($._body_member, $.anonymous_typed_param)), optional($.result_expression), "}"),

  // Comments & documentation
  comment_element: ($) =>
    seq(
      "comment",
      optional(field("name", $._name)),
      optional(seq("about", choice($.qualified_name, $.identifier), repeat(seq(",", choice($.qualified_name, $.identifier))))),
      $.doc_string
    ),

  doc_comment: ($) =>
    seq("doc", optional(field("name", $._name)), optional(seq("locale", $.string_literal)), $.doc_string),

  textual_representation: ($) =>
    seq(
      optional(seq("rep", optional(field("name", $._name)))),
      "language",
      field("language", $.string_literal),
      optional($.doc_string)
    ),

  doc_string: ($) =>
    choice(
      /\/\*([^*]|\*[^\/])*\*\//,
      seq("/*", /[^*]*/, "*/")
    ),

  // Spec: KerMLExpressions.xtext:570-577 — three distinct comment/note types
  // REGULAR_COMMENT: /* ... */ (block comment, not starting with //*)
  // ML_NOTE: //* ... */ (multi-line note)
  // SL_NOTE: // ... (single-line note)

  // Multi-line note: //* ... */ (higher priority than sl_note when both match)
  ml_note: ($) =>
    token(prec(2, seq("//*", /[^*]*\*+([^/*][^*]*\*+)*/, "/"))),

  // Single-line note: // ... (rest of line)
  sl_note: ($) =>
    token(prec(1, seq("//", /.*/))),

  // Block comment: /* ... */ (regular comment, not a note)
  comment: ($) =>
    token(seq("/*", /[^*]*\*+([^/*][^*]*\*+)*/, "/")),

  // Keywords & operators (for queries)
  keyword: ($) => choice(...keywords),
  operator: ($) => choice(...operatorSymbols),
  enum_value: ($) => choice(...enumValues),
};
