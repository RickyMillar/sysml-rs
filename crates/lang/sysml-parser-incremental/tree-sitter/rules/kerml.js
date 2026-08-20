/**
 * KerML definition and usage rules (without 'def' keyword).
 */
const { KERML_DEF_KEYWORDS } = require("../helpers/patterns");

module.exports = {
  // KerML definitions: [abstract] [assoc] <kind> Name [:> supers] { ... }
  kerml_definition: ($) =>
    prec(1, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional(choice("assoc", "individual")),
      choice(...KERML_DEF_KEYWORDS),
      optional("all"),
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      choice(
        seq(choice($.function_body, $.definition_body), optional(";")),
        ";",
      ),
    )),

  // KerML usages: things that use KerML keywords as usage prefixes
  kerml_usage: ($) =>
    prec(0, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      // G08e: `subset` is NOT a usage — it's a standalone Subsetting relationship
      // (KerML.xtext:679), now handled by `subsetting_decl`. `superset` is a pure
      // invention (absent from all spec sources) and `redefine` is not the real
      // standalone keyword (KerML.xtext:708 uses `redefinition`); both dropped.
      // `message` peeled into its own `message_usage` rule (usages.js) for its
      // `of` payload + from-required ends (SysML.xtext Message:1240).
      choice("step", "expr", "bool"),
      optional($.short_name),
      optional(field("name", choice($.feature_chain, $._name))),
      repeat($._feature_specialization),
      optional($.default_value),
      choice(
        seq(choice($.function_body, $.usage_body), optional(";")),
        ";",
      ),
    )),

  // Standalone Subsetting relationship (KerML.xtext:679-688):
  //   'subset' <subsettingFeature> (':>' | 'subsets') <subsettedFeature> ';'
  // A NonFeatureElement namespace member (NOT a usage). Both endpoints are
  // feature references (or feature chains), resolved by name. (G08e)
  subsetting_decl: ($) =>
    seq(
      "subset",
      field("subsetting", choice($.feature_chain, $.qualified_name)),
      choice(":>", "subsets"),
      field("subsetted", choice($.feature_chain, $.qualified_name)),
      ";",
    ),
};
