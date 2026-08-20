/**
 * Connector rules: connector_usage, binding_usage, connection/connector/binding ends.
 */
module.exports = {
  // Binary: [from] source to target  |  N-ary: (end, end, ...)
  connection_ends: ($) =>
    choice(
      // Binary form: [from] source to target
      seq(
        optional("from"),
        optional($.multiplicity),
        field("source", $.feature_chain),
        "to",
        optional($.multiplicity),
        field("target", $.feature_chain),
      ),
      // N-ary form: (end, end, ...)
      seq(
        "(",
        $.feature_chain,
        repeat1(seq(",", $.feature_chain)),
        ")",
      ),
    ),

  connector_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "connector",
      optional("all"),
      optional($.multiplicity),
      optional($.short_name),
      optional(field("name", $._name)),
      optional($.typing),
      optional($.multiplicity),
      optional($.multiplicity_modifiers),
      optional($.supertype_list),
      optional($.redefinition),
      optional($.connector_ends),
      // Mandatory terminator: body or semicolon (per SysML/KerML TypeBody rule).
      // This prevents the parser from reducing connector_usage early before
      // consuming name/connector_ends in patterns like:
      //   connector [0..1] transitionLink to [1..*] trigger;
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  // Binding: multiple patterns unified with "=" as the decision point
  binding_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "binding", optional("all"),
      optional($.multiplicity),
      choice(
        // "of"/"bind" pattern (merged — structurally identical)
        seq(choice("of", "bind"), optional($.multiplicity), field("source", $._binding_end_ref),
            "=", optional($.multiplicity), field("target", $._binding_end_ref),
            choice(seq($.usage_body, optional(";")), ";")),
        // Regular pattern: parse name[mult], then branch on "=" or continuation
        seq(
          optional($.short_name),
          field("name", choice($.feature_chain, $._name)),
          optional($.multiplicity),
          choice(
            // "=" means unnamed binding ends
            seq("=", optional($.multiplicity), field("target", $._binding_end_ref),
                choice(seq($.usage_body, optional(";")), ";")),
            // No "=" means named binding, regular continuation
            seq(optional($.typing),
                optional($.multiplicity_modifiers), optional($.supertype_list),
                optional($.redefinition), optional($.binding_ends),
                choice(seq($.usage_body, optional(";")), ";")),
          ),
        ),
        // Bare binding: binding { ... } or binding ;
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  connector_ends: ($) =>
    seq(
      optional(seq(
        optional("from"),
        optional($.multiplicity),
        field("source", $.feature_chain),
        optional("references"),
        optional($.feature_chain),
      )),
      "to",
      optional($.multiplicity),
      field("target", $.feature_chain),
      optional("references"),
      optional($.feature_chain),
    ),

  // Standalone bind shorthand: bind source = target ;
  // SysML BindingConnectorAsUsage (no "binding" keyword prefix)
  bind_as_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "bind",
      optional($.multiplicity),
      field("source", $._binding_end_ref),
      "=",
      optional($.multiplicity),
      field("target", $._binding_end_ref),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  _binding_end_ref: ($) => choice($.feature_chain, $.qualified_name),

  binding_ends: ($) =>
    choice(
      // With "of" keyword
      seq("of", optional($.multiplicity), field("source", $._binding_end_ref),
          "=", optional($.multiplicity), field("target", $._binding_end_ref)),
      // Direct: [mult] source = [mult] target
      seq(optional($.multiplicity), field("source", $._binding_end_ref),
          "=", optional($.multiplicity), field("target", $._binding_end_ref)),
    ),
};
