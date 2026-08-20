/**
 * Requirement and constraint body rules.
 */
module.exports = {
  requirement_body: ($) =>
    seq(
      "{",
      repeat(
        choice(
          $.actor_usage,
          $.stakeholder_usage,
          $.assume_constraint,
          $.require_constraint,
          $.assume_referenced,
          $.require_referenced,
          $.frame_constraint,
          $.verify_constraint,
          $.textual_representation,
          $._definition_or_usage,
          $.metadata_usage
        )
      ),
      "}"
    ),

  subject_requirement: ($) =>
    seq("subject", optional("redefines"), optional($.short_name), optional(field("subject", $._name)), repeat($._feature_specialization), optional($.default_value), optional($.usage_body), ";"),

  objective_requirement: ($) =>
    seq("objective", optional($.short_name), optional(field("name", $._name)), repeat($._feature_specialization), optional($.requirement_body), optional(";")),

  // `typing | redefinition` admits the two ConstraintUsageDeclaration
  // specializations the requirement machinery evaluates — in particular
  // REDEFINITION (`require constraint foo :>> Base::foo { … }`,
  // SysML.xtext:2061-2066 via FeatureSpecializationPart). Without it the
  // `:>>` clause shattered into error-recovery debris (a phantom
  // ReferenceUsage carrying the Redefinition, expression body lost) —
  // full-chain ruling §2.1a(b). DELIBERATELY narrower than the full
  // `_feature_specialization` set: admitting all of it re-shapes the GLR
  // automaton enough to shift parse/recovery outcomes on unrelated KerML
  // conformance fixtures (measured 2026-07-17: mixed ±10-path drifts on
  // association/annex-A fixtures); widen only with a conformance sweep.
  assume_constraint: ($) =>
    prec(1, seq("assume", "constraint", optional(field("name", $._name)), repeat(choice($.typing, $.redefinition)), optional($.constraint_body), optional(";"))),

  require_constraint: ($) =>
    prec(1, seq("require", "constraint", optional(field("name", $._name)), repeat(choice($.typing, $.redefinition)), optional($.constraint_body), optional(";"))),

  // Reference form (spec §7.21.2 reference subsetting; the HSUV grouping
  // idiom `require Load;` / `assume Precondition;`): requires/assumes an
  // EXISTING requirement usage by name — owns no new content. Mirrors
  // verify_constraint's target shape. `require constraint …` wins via
  // require_constraint's prec(1).
  require_referenced: ($) =>
    seq("require", field("target", choice($.feature_chain, $.qualified_name)), ";"),

  assume_referenced: ($) =>
    seq("assume", field("target", choice($.feature_chain, $.qualified_name)), ";"),

  // Per spec: `frame concern` (FramedConcernMembership), not `frame constraint`.
  // FramedConcernUsage → ConstraintUsageDeclaration admits a typing and a `;`
  // terminator (SysML.xtext:2068-2082), e.g. `frame concern : SafetyConcern;`.
  frame_constraint: ($) =>
    seq("frame", "concern", optional(field("name", $._name)), optional($.typing), optional($.constraint_body), optional(";")),

  verify_constraint: ($) =>
    seq("verify", field("target", choice($.feature_chain, $.qualified_name)), optional(";")),

  // Per spec: `render <ref>` in view bodies → ViewRenderingMembership
  // ViewRenderingMember: MemberPrefix 'render' ViewRenderingUsage
  // ViewRenderingUsage: OwnedReferenceSubsetting FeatureSpecialization* UsageBody
  //                   | ('rendering' | UsageExtensionKeyword+) Usage
  render_usage: ($) =>
    seq(
      "render",
      choice(
        // Shorthand: render :>> SomeRendering;
        seq(repeat($._feature_specialization), optional($.usage_body), ";"),
        // Inline: render rendering MyRendering { ... }
        seq("rendering", optional($.short_name), optional(field("name", $._name)),
            repeat($._feature_specialization), optional($.usage_body), optional(";")),
      ),
    ),

  assert_constraint_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "assert", optional("not"),
      optional("constraint"),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      choice(
        seq($.function_body, optional(";")),
        ";",
      ),
    )),

  constraint_body: ($) =>
    seq(
      "{",
      repeat(choice(
        $._definition_or_usage,
        $.textual_representation,
        $.metadata_usage
      )),
      optional($.result_expression),
      "}"
    ),

  // The final expression in a constraint/calc body (xtext: ResultExpression = Expression ';')
  result_expression: ($) =>
    seq($._expression, optional(";")),
};
