/**
 * Conflict declarations for the SysML v2 tree-sitter grammar.
 *
 * Combined optimization (Feb 2026):
 *   - def-merge-nofield: 9 defs merged into standard_def, only 5 individual defs remain
 *   - flow-merge-nofield: succession_flow_usage merged into flow_connection_usage
 *   - conflict-cleanup: 19 unnecessary self-conflicts removed
 *     (binding_usage, assert_constraint_usage, kerml_usage, kerml_definition,
 *      succession_decl, definition, plus 13 old *_def conflicts replaced by standard_def)
 */

module.exports = function($) {
  return [
    // standard_def self-conflict removed (unnecessary per generator)

    // === Usage self-conflicts (inline repeat(_feature_specialization)) ===
    [$.standard_usage],
    [$.port_usage],
    [$.usage_prefix],
    [$.requirement_usage],
    // concern/viewpoint usages share requirement_usage's optional-body shape (G08f)
    [$.concern_usage],
    [$.viewpoint_usage],
    [$.allocation_usage],
    [$.constraint_usage],
    [$.flow_connection_usage],
    // message_usage mirrors flow_connection_usage's shape but needs NO
    // self-conflict: its ends rule requires either `from` or an explicit
    // source-to-target arm, so the generator resolves it per-lookahead
    // (generate flagged a [$.message_usage] entry as unnecessary).
    [$.state_usage],
    // G23: exhibit_state_usage mirrors state_usage's shape exactly (optional
    // name/typing/body after a keyword), so it needs the same self-conflict
    // — after "exhibit state" the parser can't tell if `[` starts another
    // _feature_specialization (multiplicity) or ends the repeat.
    [$.exhibit_state_usage],
    [$.satisfy_requirement],
    // dependency_usage now has mandatory endpoint lists + terminator (G24),
    // so its former early-reduction self-conflict is unnecessary.
    [$.usage],
    // use_case_usage removed: merged into case_usage (Round 4 B1+B2)
    [$.include_use_case_usage],
    // case_usage self-conflict removed: mandatory terminator eliminates ambiguity (Round 4)
    // actor_usage self-conflict removed: mandatory terminator eliminates ambiguity
    // use_case_def removed: merged into case_def (Round 4 B4)
    // case_def self-conflict unnecessary (generator warning)
    // succession_flow_usage removed (merged into flow_connection_usage)
    // assert_constraint_usage removed (unnecessary)
    [$.feature_declaration],
    [$.return_feature],
    [$.feature_redefinition],
    [$.inv_constraint],
    // succession_decl removed (unnecessary)
    [$._succession_connection],
    // connector_usage self-conflict removed: mandatory terminator eliminates ambiguity
    // binding_usage removed (unnecessary)
    // kerml_definition removed (unnecessary)
    // kerml_usage removed (unnecessary)

    // === Cross-rule conflicts ===
    [$.standard_usage, $.usage_prefix],
    // port_usage no longer conflicts with usage_prefix since only "port"
    // is the keyword (in/out/inout are handled by usage_prefix).
    // "composite" appears in both _name and usage_prefix
    [$._name, $.usage_prefix],
    // use_case_usage/case_usage/include_use_case_usage cross-conflicts with usage_prefix
    // are unnecessary (generator warning) — removed
    [$.do_action, $.effect_do],
    [$.do_action, $.effect_action],
    [$.do_action, $.effect_do, $.effect_action],
    [$.effect_do, $.effect_action],
    [$.succession_usage, $.transition_target, $.target_transition_usage],
    [$.multiplicity_modifiers],

    // Only enum_def self-conflict is required (multiplicity after name)
    [$.enum_def],
    // enum_usage mirrors port_usage's named/bare keyword-usage shape and
    // needs the same self-conflict (inline repeat(_feature_specialization)).
    [$.enum_usage],
    [$.alias_decl],
    [$.import_single_name, $.import_qualified_name],
    [$.connector_ends],
    [$._feature_specialization, $.connection_ends],
    [$.binding_ends],

    // === Expression disambiguation ===
    [$.primary_expression, $.arrow_expression, $.collect_expression, $.select_expression],
    [$.primary_expression, $.bracket_expression],
    // `#` begins both PrefixMetadataAnnotation and the postfix index operator.
    [$.primary_expression, $.index_expression],

    // === State/transition keyword conflicts ===
    [$.entry_action],
    // do_action self-conflict is unnecessary (generator warning)
    [$.exit_action],
    [$.trigger_action],

    // === Requirement constraint conflicts ===
    [$.assume_constraint],
    [$.require_constraint],
    [$.frame_constraint],
    [$.objective_requirement],
    [$.render_usage],

    // === Typing multiplicity ambiguity ===
    [$.typing],

    // lambda_parameter_list conflict removed -- rule was unified into function_body (Feb 2026)

    // === function_body conflicts ===
    [$.function_body, $.definition_body],
    [$.function_body, $.usage_body],
    // function_body/requirement_body conflict is unnecessary (generator warning)

    // === Control-flow node self-conflict (merged from 5 individual rules) ===
    [$.control_flow_node],
  ];
};
