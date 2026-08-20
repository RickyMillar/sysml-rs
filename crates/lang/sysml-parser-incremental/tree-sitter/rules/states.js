/**
 * State machine rules: state_body, entry/do/exit actions, transitions.
 */
module.exports = {
  state_body: ($) =>
    seq(
      "{",
      repeat(
        choice(
          $.entry_action,
          $.do_action,
          $.exit_action,
          $.state_transition_chain,
          $._definition_or_usage,
          $.metadata_usage
        )
      ),
      "}"
    ),

  entry_action: ($) =>
    choice(
      seq("entry", ";"),
      seq("entry", optional(field("action", $._name)), optional($.send_inline), optional($.action_body))
    ),

  do_action: ($) =>
    seq("do", optional(field("action", $._name)), optional($.send_inline), optional($.action_body)),

  exit_action: ($) =>
    choice(
      seq("exit", ";"),
      seq("exit", optional(field("action", $._name)), optional($.send_inline), optional($.action_body))
    ),

  send_inline: ($) =>
    seq("send", $._expression, optional(seq("via", $._expression)), ";"),

  // Inline transition chain: accept X via Y if Z do W then T;
  state_transition_chain: ($) =>
    prec.right(seq(
      optional($.trigger_accept),
      optional($.guard_expression),
      optional($.effect_do),
      $.transition_target,
      optional(";")
    )),

  trigger_accept: ($) =>
    prec(1, seq(
      "accept",
      optional(field("trigger_name", $._name)),
      optional(seq(":", field("trigger_type", $.type_ref))),
      optional(choice(
        seq("via", field("via_port", $._expression)),
        seq("when", field("guard", $._expression)),
        seq("after", field("after_expr", $._expression))
      ))
    )),

  effect_do: ($) =>
    seq("do", choice(
      $.send_inline,
      seq("action", field("action", $._name), ";"),
      $.action_body
    )),

  transition_usage: ($) =>
    prec.right(
      seq(
        optional($.visibility_indicator),
        "transition",
        optional(field("name", $._name)),
        optional($.transition_source),
        optional($.trigger_action),
        optional($.guard_expression),
        optional($.effect_action),
        optional($.transition_target),
        optional(";")
      )
    ),

  transition_source: ($) =>
    prec(1, seq("first", field("source", $._name))),

  trigger_action: ($) =>
    seq(
      "accept",
      optional(field("trigger", $._name)),
      optional($.typing),
      optional(choice(
        seq("via", field("via_port", $._expression)),
        seq("when", field("when_guard", $._expression)),
        seq("after", field("after_expr", $._expression))
      ))
    ),

  guard_expression: ($) =>
    seq("if", choice(
      seq("[", $._expression, "]"),
      $._expression,
    )),

  // `transition` keyword form effect (SysML §7.18.3). Accepts the canonical spec
  // form `do action {…}` (e.g. `do action providePower {…}`) in addition to the
  // pre-existing `do {…}` / `do name;`. The `action` keyword is absorbed as the
  // name; the trailing `{` vs `;` disambiguates the name arms (LR-friendly, no
  // new conflict). Previously `do action {…}` fell out of the transition and was
  // mis-parsed as a sibling state subaction (GAP-SM-EFFECT).
  effect_action: ($) =>
    seq("do", choice(
      $.action_body,                                  // do { ... }
      seq(field("action", $._name), $.action_body),   // do action { ... }
      seq(field("action", $._name), ";")              // do name;
    )),

  transition_target: ($) =>
    seq("then", field("target", choice($._name, $.qualified_name))),

  target_transition_usage: ($) =>
    prec(1, seq(
      optional($.visibility_indicator),
      optional($.trigger_action),
      optional($.guard_expression),
      optional(choice($.effect_do, $.effect_action)),
      "then",
      field("target", choice($.qualified_name, $._name)),
      optional($.action_body),
      optional(";"),
    )),
};
