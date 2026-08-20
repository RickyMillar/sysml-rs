/**
 * Definition rules: standard_def (merged 9 defs), custom-body defs, enum, and generic definition.
 *
 * OPTIMIZATION: 9 identical definitions (part, attribute, port, connection, interface,
 * item, allocation, occurrence, flow) are merged into `standard_def` with a keyword choice.
 * This eliminates 8 duplicate rule state sets. No field() wrapper on keyword to avoid
 * adding LR states (field() creates new contexts).
 */
const { defRule } = require("../helpers/patterns");

// These keep custom bodies -- stay as individual rules.
// concern/viewpoint use RequirementBody per SysML.xtext:2151 (ConcernDefinition)
// and :2399 (ViewpointDefinition) — peeled out of the generic `definition`
// fallback (which uses the shared definition_body) so requirement-specific
// members (stakeholder, actor, frame, subject, ...) parse inside them. (G08f)
const CUSTOM_BODY_DEFS = [
  ["action_def",      "action",      "action_body"],
  ["state_def",       "state",       "state_body"],
  ["requirement_def", "requirement", "requirement_body"],
  ["constraint_def",  "constraint",  "constraint_body"],
  ["concern_def",     "concern",     "requirement_body"],
  ["viewpoint_def",   "viewpoint",   "requirement_body"],
];

const customDefs = {};
for (const [name, keyword, body] of CUSTOM_BODY_DEFS) {
  customDefs[name] = defRule(keyword, body);
}

module.exports = {
  ...customDefs,

  // Token-distinct user-defined-keyword form for connection definitions.
  // Kept separate from merged standard_def so ordinary definitions do not
  // inherit an empty prefix-metadata repeat and its GLR state cost.
  annotated_connection_def: ($) =>
    prec(4, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      repeat1($.prefix_metadata_annotation),
      "connection", "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq($.definition_body, optional(";")),
        ";",
      ),
    )),

  // Merged: 9 definitions that all use definition_body (no field() -- adds LR states)
  standard_def: ($) =>
    prec(3, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      choice(
        "part", "attribute", "port", "connection", "interface",
        "item", "allocation", "occurrence", "flow",
      ),
      "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq($.definition_body, optional(";")),
        ";",
      ),
    )),

  // IndividualDefinition (SysML.xtext IndividualDefinition:
  // `BasicDefinitionPrefix? isIndividual?='individual' EmptyMultiplicityMember
  //  DefinitionExtensionKeyword* 'def' Definition` — returns
  // OccurrenceDefinition; there is NO IndividualDefinition metaclass in
  // SysML-vocab.ttl). The bare `individual def X;` form only.
  //
  // LR shape: in body-member position, `individual` already forks between
  // usage_prefix (standard_usage/feature_declaration) and kerml_definition's
  // leading optional(choice("assoc","individual")) — the lexer's keyword
  // extraction picks the following KEYWORD (struct/classifier/… — and now
  // `def`) when an in-state item makes it valid, and an identifier reduces to
  // the usage-prefix path. This rule adds only the `def`-keyword continuation
  // to that existing fork; no usage keyword overlaps it.
  //
  // Deliberately NOT covered (follow-up residual, shared with `variation`):
  // the prefixed `individual <kind> def` family (OccurrenceDefinitionPrefix on
  // part/item/occurrence/… defs, SysML.xtext OccurrenceDefinitionPrefix). After
  // `individual`, a kind keyword like `part` is a shift(this-path)/
  // reduce(usage_prefix) fork that LR(1) cannot split without a
  // [$.usage_prefix, $.standard_def]-style GLR conflict — `individual part x;`
  // (usage) vs `individual part def X;` (def) diverge only at the token after
  // the kind keyword. `variation part def V;` misparses identically today.
  individual_def: ($) =>
    seq(
      optional($.visibility_indicator),
      optional("abstract"),
      "individual",
      "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq($.definition_body, optional(";")),
        ";",
      ),
    ),

  enum_def: ($) =>
    seq(
      optional($.visibility_indicator),
      "enum", "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      optional($.enum_body),
    ),

  enum_body: ($) =>
    seq(
      "{",
      repeat(choice($.enum_member, $.doc_comment, $.comment_element)),
      "}"
    ),

  enum_member: ($) =>
    choice(
      seq(
        optional("enum"),
        field("name", $._name),
        optional($.typing),
        optional($.default_value),
        optional($.usage_body),
        ";"
      ),
      seq(
        field("name", $._name),
        $.usage_body
      ),
      // Unnamed EnumeratedValue value form (SysML.xtext EnumeratedValue →
      // Usage): e.g. `= 60.0;` inside `enum def SizeChoice`. Begins with the
      // default_value separator (`=`/`:=`/`default`), disjoint from the
      // name-first arms above.
      seq(
        optional("enum"),
        $.default_value,
        ";"
      )
    ),

  // Per spec (SysML.xtext:2174-2176, 2287-2289): case def / use case def Name CaseBody
  // OPTIMIZATION: use_case_def merged into case_def with bare keyword choice (no field())
  case_def: ($) =>
    prec(3, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      choice("case", seq("use", "case")),
      "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq($.case_body, optional(";")),
        ";",
      ),
    )),

  // G04b: `calc def` peeled out of the generic `definition` rule into a dedicated
  // rule so its body (`calc_body`) can admit anonymous typed parameters
  // (`in : Real[1];`) WITHOUT touching the shared function_body that every other
  // brace-bodied def uses. Body is just `calc_body` (a superset of definition_body
  // + result_expression), so there is no function_body/definition_body choice to
  // perturb. Dispatches to CalculationDefinition exactly like the old generic path.
  calc_def: ($) =>
    prec(3, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      "calc", "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq($.calc_body, optional(";")),
        ";",
      ),
    )),

  // Generic definition fallback (analysis, view, etc.)
  definition: ($) =>
    prec(1, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      // concern/viewpoint peeled to dedicated requirement_body rules (G08f);
      // `stakeholder` removed entirely — SysML.xtext:2098 defines only
      // StakeholderUsage (a PartUsage), never a StakeholderDefinition, so
      // `stakeholder def` is non-conformant and must fail to parse (G08f).
      field("keyword", choice(
        "analysis", "verification", "view",
        "rendering", "metadata",
      )),
      "def",
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq(choice($.function_body, $.definition_body), optional(";")),
        ";",
      ),
    )),
};
