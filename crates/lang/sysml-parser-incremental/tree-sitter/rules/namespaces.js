/**
 * Namespace and package rules: packages, namespaces, imports, aliases.
 */
module.exports = {
  source_file: ($) => repeat($._root_member),

  _root_member: ($) =>
    choice(
      $.package_decl,
      $.namespace_decl,
      $.library_package,
      $._definition_or_usage,
      $.import_decl,
      $.alias_decl,
      $.filter_decl,
      $.comment_element,
      $.metadata_usage,
    ),

  package_decl: ($) =>
    seq(
      optional($.visibility_indicator),
      "package",
      optional($.short_name),
      field("name", $._name),
      optional($.package_body)
    ),

  namespace_decl: ($) =>
    seq(
      optional($.visibility_indicator),
      "namespace",
      field("name", $._name),
      optional($.namespace_body)
    ),

  library_package: ($) =>
    seq(
      optional($.visibility_indicator),
      optional("standard"),
      "library",
      "package",
      optional($.short_name),
      field("name", $._name),
      optional($.package_body)
    ),

  package_body: ($) => seq("{", repeat($._package_member), "}"),
  namespace_body: ($) => seq("{", repeat($._namespace_member), "}"),

  // Nested namespace declarations ARE legal package/namespace members
  // (SysML.xtext PackageBodyElement -> PackageMember -> Package). Their
  // omission made `package Inner {}` inside a body error-recover into
  // two feature_declarations (one literally named "package") — the
  // S001 false-duplicate source found in the coffee-machine triage.
  _package_member: ($) =>
    choice(
      $.package_decl,
      $.namespace_decl,
      $.library_package,
      $.import_decl,
      $.alias_decl,
      $.filter_decl,
      $._definition_or_usage,
      $.comment_element,
      $.metadata_usage
    ),

  _namespace_member: ($) =>
    choice(
      $.package_decl,
      $.namespace_decl,
      $.library_package,
      $._definition_or_usage,
      $.import_decl,
      $.alias_decl,
      $.filter_decl,
      $.comment_element,
      $.metadata_usage
    ),

  import_decl: ($) =>
    prec(4, seq(
      optional($.visibility_indicator),
      "import",
      optional("all"),
      field("target", $.import_target),
      optional(seq("::", choice("*", "**"))),
      optional($.filter_package),
      ";"
    )),

  import_target: ($) =>
    choice(
      $.import_qualified_name,
      $.import_single_name
    ),

  import_qualified_name: ($) =>
    prec.left(seq(
      optional($.global_qualification),
      $._name,
      repeat1(seq("::", $._name))
    )),

  import_single_name: ($) =>
    seq(optional($.global_qualification), $._name),

  // Per spec: `expose <qname>[::*] ;` — Import subtype for ViewUsage bodies
  // Expose: MembershipExpose | NamespaceExpose
  // Unlike import, uses 'expose' keyword instead of visibility + 'import'
  expose_decl: ($) =>
    prec(4, seq(
      "expose",
      field("target", $.import_target),
      optional(seq("::", choice("*", "**"))),
      optional($.filter_package),
      ";"
    )),

  alias_decl: ($) =>
    seq(
      optional($.visibility_indicator),
      "alias",
      optional($.short_name),
      optional(field("name", $._name)),
      "for",
      field("target", choice($.qualified_name, $._name)),
      repeat($._feature_specialization),
      optional(choice(
        seq("{", repeat($._definition_or_usage), "}"),
        ";"
      ))
    ),

  filter_package: ($) => seq("[", $._expression, "]"),

  // Per spec SysML.xtext:229-232 — ElementFilterMember:
  //   MemberPrefix 'filter' ownedRelatedElement += OwnedExpression ';'
  // Routed via PackageBody / PackageBodyElement (xtext lines 203, 213).
  // The expression body covers both the plain form (`filter someExpr;`,
  // `filter Safety::isMandatory;`) and the metadata-classifier form
  // (`filter @Safety;`, `filter @Safety and Safety::isMandatory;`), where
  // `@Safety` is a leading ClassificationExpression with no left operand
  // (SysML.pest line 77: `ClassificationTestOperator ~ TypeReferenceMember`).
  filter_decl: ($) =>
    prec(4, seq(
      optional($.visibility_indicator),
      "filter",
      field("expression", $._filter_expression),
      ";"
    )),

  // Operand-less ClassificationExpression as a primary, then any binary tail
  // built on top of `_expression` via choice. The leading-`@` form would
  // otherwise be ambiguous with `metadata_usage` at top level, so we keep it
  // gated behind the `filter` keyword.
  _filter_expression: ($) =>
    choice(
      $._expression,
      $.filter_classification_lead
    ),

  // Hidden helper: an operand-less ClassificationExpression — `@TypeRef`
  // (or `hastype`/`istype TypeRef`) with the LEFT operand omitted (implicit
  // `self`), per KerML ClassificationExpression BNF where the left
  // ArgumentMember is optional. Gated within the filter context only (a bare
  // prefix-`@` is deliberately NOT a global `_expression` — it would be
  // ambiguous with `metadata_usage`).
  _lead_classification: ($) =>
    seq(choice("hastype", "istype", "@"), $.type_ref),

  // `@TypeRef` (or `hastype TypeRef`, `istype TypeRef`) with no left operand,
  // optionally followed by a binary-expression tail. The tail operand may
  // itself be a leading-`@` classification, so `@A or @B` chains parse — a
  // bare `@B` is NOT an `_expression` (the infix classification_expression
  // requires a left operand). We re-implement the right side as a flat repeat
  // to avoid bringing the full binary precedence stack into play here.
  filter_classification_lead: ($) =>
    prec.left(seq(
      $._lead_classification,
      repeat(seq(
        choice("and", "or", "xor", "&", "|", "implies"),
        choice($._expression, $._lead_classification)
      ))
    )),
};
