/**
 * Action rules: action_body, action statements, and control-flow nodes.
 */
module.exports = {
  action_body: ($) =>
    seq(
      "{",
      repeat(
        choice(
          $._definition_or_usage,
          $.metadata_usage,
          $.accept_action,
          $.send_action,
          $.assignment_action,
          $.if_action,
          $.while_action,
          $.for_action,
          $.terminate_action,
        )
      ),
      "}",
    ),

  accept_action: ($) =>
    seq(
      "accept",
      optional(field("name", $._name)),
      optional($.typing),
      // SysML.xtext:1447 — `accept <param> via <port>`: the receiving port of
      // an action-node AcceptActionUsage. Mirrors `trigger_accept`'s `via`
      // (states.js) so the runtime lowers it to an Accept node port_source.
      optional(seq("via", field("via_port", $._expression))),
      ";",
    ),

  send_action: ($) =>
    seq("send", $._expression, optional(seq(choice("to", "via"), $._expression)), ";"),

  assignment_action: ($) =>
    seq("assign", $._expression, choice("=", ":="), $._expression, ";"),

  // TerminateNode (SysML.xtext:1636, `TerminateNode returns SysML::
  // TerminateActionUsage`). A terminate action ends the performance of an
  // occurrence; the terminated occurrence may be named as an optional argument
  // (`NodeParameterMember`) and defaults to the immediately containing action.
  // Direct action-node forms: `terminate;` and `terminate <ref>;`. The
  // `action <name> terminate;` declaration-prefix form (ActionNodeUsage-
  // Declaration) and the succession-prefixed inline `then terminate;` form are
  // not yet covered — see dispatch.rs terminate_action handler notes.
  terminate_action: ($) =>
    seq("terminate", optional(field("target", $._expression)), ";"),

  if_action: ($) =>
    seq(
      "if",
      $._expression,
      $.action_body,
      optional(seq("else", choice($.action_body, $.if_action))),
    ),

  while_action: ($) =>
    seq(
      choice("while", "until"),
      $._expression,
      $.action_body,
    ),

  for_action: ($) =>
    seq(
      "for",
      field("var", $._name),
      "in",
      $._expression,
      $.action_body,
    ),

  // Control-flow nodes merged into single rule (like standard_usage pattern).
  // Replaces: merge_node, decision_node, fork_node, join_node, perform_action.
  // Experiment B (Feb 2026): saves 202 states (12,212 -> 12,010).
  control_flow_node: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional($.usage_prefix),
      field("keyword", choice("merge", "decide", "fork", "join", "perform")),
      optional("action"),   // only valid after "perform" per spec
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),  // only valid for perform per spec
      optional($.action_body),
      optional(";"),
    )),
};
