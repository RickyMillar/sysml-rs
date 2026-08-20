/**
 * Usage rules: all *_usage rules plus feature declarations, satisfy, dependency, etc.
 *
 * OPTIMIZATION: 5 identical standard usages (part, attribute, item, occurrence, ref)
 * are merged into `standard_usage` with a keyword field. This eliminates 4 duplicate
 * rule state sets from the parser, significantly reducing parser.c size.
 * Consumers distinguish the kind via the `keyword` field value.
 *
 * Hidden subrules (_usage_header, _usage_name) were tried but cause >65k states
 * due to keyword overlap + GLR conflict inflation. See TREE_SITTER_STATUS.md.
 */
module.exports = {
  // === Standard usages (merged) ===
  // Replaces: part_usage, attribute_usage, item_usage, occurrence_usage, ref_usage
  // Distinguish by reading the `keyword` field (e.g., "part", "attribute", etc.)

  standard_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      // Per SysML.xtext EventOccurrenceUsage (SysML.xtext:862-866):
      //   OccurrenceUsagePrefix 'event' OccurrenceUsageKeyword UsageDeclaration?
      // i.e. the `event` keyword is an optional specialiser that may precede
      // the (occurrence) keyword to mint EventOccurrenceUsage. Modelling it
      // as a leading optional keeps the merged `standard_usage` rule intact
      // and limits state growth. ast_builder dispatches `event_prefix` +
      // keyword=="occurrence" to EventOccurrenceUsage (G10).
      optional(field("event_prefix", "event")),
      field("keyword", choice("part", "attribute", "item", "occurrence", "ref")),
      optional($.short_name),
      // `frame` is contextual in RequirementBody (`frame concern ...`), not
      // globally reserved. Keep the exception local to usage names rather
      // than adding a grammar keyword to `_name` (which explodes conflicts).
      optional(field("name", choice($._name, alias("frame", $.identifier)))),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.usage_body),
      optional(";"),
    )),

  // Per SysML spec: PortUsage = OccurrenceUsagePrefix 'port' PortUsageDeclaration
  // Direction (in/out/inout) is part of usage_prefix, NOT a port keyword.
  // This ensures `in port data : DataPort;` parses as ONE port_usage with
  // direction=in, not two separate port_usages.
  port_usage: ($) =>
    choice(
      // Full form: [in|out] port name [: Type] [body] ;
      prec(3, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "port",
        optional($.short_name),
        field("name", $._name),
        repeat($._feature_specialization),
        optional($.default_value),
        optional($.usage_body),
        optional(";"),
      )),
      // Bare form: [in|out] port [body] ;
      prec(2, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "port",
        optional($.short_name),
        // no name
        repeat($._feature_specialization),
        optional($.default_value),
        optional($.usage_body),
        optional(";"),
      )),
    ),

  // EnumerationUsage (SysML.xtext EnumerationUsage: `UsagePrefix
  // EnumerationUsageKeyword Usage` — a real metaclass, SysML-vocab.ttl).
  // The standalone `enum e : Color;` usage form. Mirrors port_usage's
  // named/bare split — `port` is the proven precedent for a usage keyword
  // that also starts a `<kw> def` definition (standard_def), with the
  // keyword-extraction lexer picking the `def` keyword after `enum` when the
  // enum_def item makes it valid in-state, and [$.enum_usage] mirroring
  // [$.port_usage] in conflicts.js. Enum MEMBERS inside an `enum def` body
  // are a different rule (enum_member, definitions.js) and are unaffected.
  enum_usage: ($) =>
    choice(
      // Full form: [prefix] enum name [: Type] [= value] [body] ;
      prec(3, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "enum",
        optional($.short_name),
        field("name", $._name),
        repeat($._feature_specialization),
        optional($.default_value),
        optional($.usage_body),
        optional(";"),
      )),
      // Bare form: [prefix] enum [: Type] [= value] [body] ;
      prec(2, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "enum",
        optional($.short_name),
        // no name
        repeat($._feature_specialization),
        optional($.default_value),
        optional($.usage_body),
        optional(";"),
      )),
    ),

  // === Custom-body usages ===

  action_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "action",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      choice(
        seq($.action_body, optional(";")),
        ";",
      ),
    )),

  // G21: split into named/bare forms (mirrors port_usage). The all-optional
  // single-seq form let the LR parser prematurely reduce a bare `state`
  // keyword to a complete (empty) state_usage, then re-parse a following
  // `Name { entry action {...} }` as a sibling feature_declaration — losing
  // the entry action of the FIRST state in a state def body. Splitting forces
  // the parser to shift the identifier as `name` when one follows `state`
  // (higher-prec full form), while the lower-prec bare form keeps the
  // anonymous `state {...}` usage valid.
  state_usage: ($) =>
    choice(
      // Full form: [prefix] state name [parallel] [specs] [body] ;
      prec(3, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "state",
        optional($.short_name),
        field("name", $._name),
        optional("parallel"),
        repeat($._feature_specialization),
        optional($.state_body),
        optional(";"),
      )),
      // Bare form: [prefix] state [parallel] [specs] [body] ;
      prec(2, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "state",
        optional($.short_name),
        // no name
        optional("parallel"),
        repeat($._feature_specialization),
        optional($.state_body),
        optional(";"),
      )),
    ),

  // G23: ExhibitStateUsage (SysML.xtext:1835-1841) —
  //   OccurrenceUsagePrefix 'exhibit'
  //   ( ownedRelationship += OwnedReferenceSubsetting FeatureSpecializationPart?
  //   | StateUsageKeyword UsageDeclaration?
  //   )
  //   ValuePart? StateUsageBody
  //
  // Previously `exhibit` lived only in the generic `usage` fallback (bare
  // keyword, no "state" alternative), so the DOMINANT corpus form
  // `exhibit state <name> : <Type>;` mis-parsed: the LR parser reduced
  // `usage` to an empty node right after "exhibit", then re-parsed the
  // trailing `state <name> : <Type> ;` as an unrelated sibling `state_usage`
  // — two elements (an empty ExhibitStateUsage + a phantom StateUsage) for
  // what the spec defines as ONE ExhibitStateUsage. Peeled into its own
  // rule with three prec'd alternatives:
  //   (a) bare subsetting form `exhibit <name> [specs] ...;` (name required —
  //       OwnedReferenceSubsetting always references a target feature)
  //   (b) full `exhibit state <name> [: Type] ...;` form
  //   (c) bare `exhibit state [: Type] ...;` form (no name)
  // (b)/(c) mirror state_usage's own named/bare split (G21): the
  // `"state" [name] [specs] [body]` tail has the exact all-optional shape
  // that let the parser prematurely reduce `state` to empty before shifting
  // a following name, so the same prec(full) > prec(bare) trick applies.
  // `repeat($._feature_specialization)` already covers every UsageDeclaration
  // specialization form (typing, subsets, references, redefines — see
  // common.js `_feature_specialization`), so this fix is not limited to the
  // bare-typing case.
  exhibit_state_usage: ($) =>
    choice(
      // (a) `exhibit <name> [specs] [= default] [parallel] [{ body }] ;`
      prec(4, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "exhibit",
        field("name", $._name),
        repeat($._feature_specialization),
        optional($.default_value),
        optional("parallel"),
        optional($.state_body),
        optional(";"),
      )),
      // (b) full: `exhibit state <name> [: Type] [= default] [parallel] [{ body }] ;`
      prec(3, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "exhibit",
        "state",
        optional($.short_name),
        field("name", $._name),
        repeat($._feature_specialization),
        optional($.default_value),
        optional("parallel"),
        optional($.state_body),
        optional(";"),
      )),
      // (c) bare: `exhibit state [: Type] [= default] [parallel] [{ body }] ;` (no name)
      prec(2, seq(
        optional($.visibility_indicator),
        optional("abstract"),
        optional($.usage_prefix),
        "exhibit",
        "state",
        optional($.short_name),
        // no name
        repeat($._feature_specialization),
        optional($.default_value),
        optional("parallel"),
        optional($.state_body),
        optional(";"),
      )),
    ),

  // Token-distinct annotated connection form. Keeping the required `#`
  // branch separate avoids adding an empty PrefixMetadataAnnotation repeat
  // to every ordinary connection (which perturbs GLR choices corpus-wide).
  annotated_connection_usage: ($) =>
    prec(3, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      repeat1($.prefix_metadata_annotation),
      choice("connection", "connect"),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional("connect"),
      optional($.connection_ends),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  connection_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      choice("connection", "connect"),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional("connect"),
      optional($.connection_ends),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  // Anonymous connection-end usage with an explicit ReferenceSubsetting.
  // This is the pilot/spec-idiomatic RequirementDerivation shape:
  //   end #original ::> vehicleMassRequirement;
  // The general feature_declaration rule intentionally requires a name, so
  // this token-distinct `end` + metadata + ReferencesKeyword form is scoped
  // to its own rule instead of weakening every anonymous feature context.
  connection_end_usage: ($) =>
    prec(3, seq(
      optional($.visibility_indicator),
      "end",
      repeat1($.prefix_metadata_annotation),
      optional($.short_name),
      optional(field("name", choice($._name, alias("frame", $.identifier)))),
      $.references_clause,
      repeat($._feature_specialization),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  interface_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "interface",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional("connect"),
      optional($.connection_ends),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  requirement_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "requirement",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.requirement_body),
      optional(";"),
    )),

  constraint_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "constraint",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.constraint_body),
      optional(";"),
    )),

  allocation_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      choice("allocate", "allocation"),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional("allocate"),
      optional($.allocation_ends),
      optional(";"),
    )),

  allocation_ends: ($) =>
    seq(
      field("from", $.feature_chain),
      "to",
      field("to", $.feature_chain),
    ),

  flow_connection_usage: ($) =>
    seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      choice("flow", seq("succession", "flow")),
      optional($.short_name),
      optional(field("name", $._name)),
      optional(seq("of", field("flow_type", $.type_ref))),
      repeat($._feature_specialization),
      optional($.flow_ends),
      optional($.usage_body),
      optional(";"),
    ),

  flow_ends: ($) =>
    choice(
      seq("from", field("source", $.feature_chain), "to", field("target", $.feature_chain)),
      // SysML permits the compact anonymous form `flow source to target;`
      // (no `from` keyword). Keep this in the endpoint sub-rule so the CST
      // exposes the same source/target fields as the explicit `from ... to ...`
      // form and ast_builder can continue using one extractor.
      seq(field("source", $.feature_chain), "to", field("target", $.feature_chain)),
      seq("from", field("source", $.feature_chain)),
      seq("to", field("target", $.feature_chain)),
    ),

  // Message (SysML.xtext Message:1240 → FlowUsage; MessageDeclaration
  // :1244-1252): `message <name>? ('of' <payload>)? ('from' <src> 'to'
  // <dst>)?` OR the anonymous alternative `message <src> to <dst>` (spec: the
  // anonymous alternative carries NO UsageDeclaration/ValuePart/payload).
  // Peeled out of the shared kerml_usage keyword bundle so its `of` payload
  // and ends parse.
  //
  // The F3 verbatim conflict `'message' feature_chain 'to' feature_chain` had
  // TWO causes, both fixed here rather than by dropping the anonymous form:
  // (1) the name field admitted $.feature_chain (kerml_usage inheritance —
  //     spec Identification is a plain name), so `message A to B` parsed as
  //     name=A both ways; now name is $._name, mirroring flow.
  // (2) reusing flow_ends brought its `to <target>`-only arm, making
  //     (name, to-ends) vs (no-name, anon-ends) genuinely ambiguous; the
  //     message_ends rule below has exactly the two spec arms.
  // flow_connection_usage is the proven precedent for this exact shape
  // (optional $._name name + an ends rule with an anonymous arm), resolved by
  // its self-conflict entry — [$.message_usage] mirrors it (conflicts.js).
  message_usage: ($) =>
    seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "message",
      optional($.short_name),
      optional(field("name", $._name)),
      optional(seq("of", field("payload", $.type_ref))),
      repeat($._feature_specialization),
      optional($.message_ends),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    ),

  // MessageDeclaration ends (SysML.xtext :1244-1252): the from-led arm and
  // the anonymous `<src> to <dst>` arm. Same source/target fields as
  // flow_ends so ast_builder's endpoint extractor is unchanged (one home).
  // NOTE: deliberately NOT reusing $.flow_ends — its extra from-only/to-only
  // arms are what made the anonymous form ambiguous for message (see above).
  message_ends: ($) =>
    choice(
      seq("from", field("source", $.feature_chain), "to", field("target", $.feature_chain)),
      seq(field("source", $.feature_chain), "to", field("target", $.feature_chain)),
    ),

  // === Succession rules ===

  succession_usage: ($) =>
    prec(-1, seq(
      optional($.visibility_indicator),
      choice("first", "then"),
      field("successor", choice($.feature_chain, $.qualified_name)),
      optional(";"),
    )),

  _succession_connection: ($) => seq(
    optional(seq(
      optional(choice("from", "first")),
      optional($.multiplicity),
      field("source", choice($.feature_chain, $.qualified_name)),
    )),
    "then",
    optional($.multiplicity),
    field("target", choice($.feature_chain, $.qualified_name)),
  ),

  succession_decl: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "succession",
      optional("all"),
      optional($.short_name),
      optional(field("name", $._name)),
      optional($.typing),
      optional($.multiplicity),
      optional($.multiplicity_modifiers),
      optional($.supertype_list),
      optional($.redefinition),
      optional($._succession_connection),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  // === Feature declaration/redefinition ===

  // Per spec (KerML.xtext FeatureDeclaration):
  //   FeatureDeclaration ::= Identification FeatureSpecializationPart?
  //                        | FeatureSpecializationPart
  // Identification carries the name; FeatureSpecializationPart is one or
  // more typings/subsettings/redefinitions/etc. So either the name is
  // present (with optional specs) OR there is no name but at least ONE
  // feature_specialization is required.
  //
  // This admits anonymous typed parameters like `in : Real[1];` and
  // `out : Real;` inside calc/function bodies (gap G04 — KerML library
  // VectorCalculations / TensorCalculations rely on this form).
  feature_declaration: ($) =>
    // NOTE: G04 (commit 2ab4643b) added an anonymous form `[in] : Type ...`
    // here as a `choice(named, anonymous)`, to admit `in : Real[1];` inside
    // calc/function bodies (~43 FeatureTypings in KerML library). That
    // change caused the LR table to silently prefer SPLITTING multi-prefix
    // usages at the keyword boundary: `end ref part p : T [0..*] ordered;`
    // would parse as a prefix-only standard_usage + a NAMED feature_declaration
    // for `p : T [0..*] ordered`. Three rounds of conflict declarations
    // could not bias the GLR runtime correctly. Reverted to the named-only
    // form. The anonymous-typed-parameter gap (G04) is deferred to a future
    // G04b worktree with a cleaner approach: introduce a separate rule
    // scoped to function_body / calc_body, not added to global
    // feature_declaration.
    prec(0, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      optional($.multiplicity),
      optional(seq(optional("member"), "feature")),
      optional($.short_name),
      field("name", $._name),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.usage_body),
      optional(";"),
    )),

  return_feature: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "return",
      optional("feature"),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.usage_body),
      optional(";"),
    )),

  // G12: Invariant. Per KerML.xtext:976 `Invariant: FeaturePrefix 'inv'
  // ('true'|'false')? ExpressionDeclaration FunctionBody` where
  // ExpressionDeclaration carries an optional ValuePart (`= expr`) and FunctionBody
  // may be `;`. So `inv flag = true;` and `inv flag : Boolean = true;` are
  // spec-legal — not only the braced `inv x { ... }` form. `inv` is KerML-only.
  inv_constraint: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "inv",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.constraint_body),
      optional(";"),
    )),

  // G04b: anonymous typed parameter — `in : Real[1];` (no name) inside a calc def
  // body. Per KerML.xtext:549 the FeatureDeclaration name is optional when a
  // FeatureSpecializationPart (typing) is present. Reachable ONLY from calc_body
  // (see common.js), never the shared function_body / _definition_or_usage — so it
  // cannot perturb other brace-bodied defs. MUST lead with `typing` (`: Type`):
  // token-distinct from feature_redefinition (`:>`/`:>>`) and named
  // feature_declaration (name before `:`); the typing rule absorbs the trailing
  // multiplicity, and `default_value` is omitted (the `default` keyword is a
  // usage_prefix → ambiguous). KerML Vector/Tensor params are pure `in : Type[mult]`.
  anonymous_typed_param: ($) =>
    prec(2, seq(
      optional($.usage_prefix),
      $.typing,
      optional(";"),
    )),

  feature_redefinition: ($) =>
    prec(1, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      optional(seq(optional("member"), "feature")),
      choice($.redefinition, $.supertype_list),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.usage_body),
      optional(";"),
    )),

  // === Satisfy/verify & dependency ===

  satisfy_requirement: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      choice(
        // satisfy can use direct reference: satisfy <ref> [by <subject>];
        // Per xtext SatisfyRequirementUsage (SysML.xtext:2112)
        seq("satisfy",
          choice(
            seq("requirement",
                optional($.short_name),
                optional(field("name", $._name)),
                repeat($._feature_specialization)),
            seq(field("ref", choice($.qualified_name, $._name)),
                repeat($._feature_specialization)),
          )),
        // verify always requires "requirement" keyword here;
        // bare "verify <ref>" is handled by verify_constraint
        seq("verify", "requirement",
            optional($.short_name),
            optional(field("name", $._name)),
            repeat($._feature_specialization)),
      ),
      optional(seq("by", field("subject", choice($.feature_chain, $.qualified_name, $._name)))),
      optional($.requirement_body),
      optional(";"),
    )),

  // SysML.xtext:55-60 — prefix metadata, optional identification only when
  // followed by `from`, one-or-more clients, one-or-more suppliers, and a
  // mandatory RelationshipBody terminator. Requiring the endpoint lists and
  // terminator also removes the keyword-only early reduction behind B1a's
  // sl_note GLR-shatter recovery.
  dependency_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      repeat($.prefix_metadata_annotation),
      "dependency",
      choice(
        seq(field("name", $._name), "from"),
        optional("from"),
      ),
      field("client", choice($.feature_chain, $.qualified_name)),
      repeat(seq(",", field("client", choice($.feature_chain, $.qualified_name)))),
      "to",
      field("supplier", choice($.feature_chain, $.qualified_name)),
      repeat(seq(",", field("supplier", choice($.feature_chain, $.qualified_name)))),
      choice(
        seq($.usage_body, optional(";")),
        ";",
      ),
    )),

  // === Use case rules ===

  // use_case_def merged into case_def in definitions.js (Round 4 optimization B4)

  // Per spec (SysML.xtext:2295-2301): include [ref | use case Name] CaseBody
  include_use_case_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "include",
      choice(
        // Inline form: include use case [Name] ...
        seq("use", "case",
            optional($.short_name),
            optional(field("name", $._name))),
        // Reference form: include <ref> ...
        seq(field("name", choice($.feature_chain, $._name)),
            repeat($._feature_specialization)),
      ),
      repeat($._feature_specialization),
      optional($.default_value),
      optional($.case_body),
      optional(";"),
    )),

  // Merged case_usage + use_case_usage (Round 4 optimization B1+B2)
  // Per spec: "case" (SysML.xtext:2209) and "use case" (SysML.xtext:2291)
  // Bare choice() on keyword — NO field() (proven anti-pattern: +540 states)
  case_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      choice("case", seq("use", "case")),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      choice(seq($.case_body, optional(";")), ";"),
    )),

  // Per spec (SysML.xtext:2084-2091): actor [Name] Usage
  // ActorMembership wraps an ActorUsage (returns PartUsage)
  actor_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "actor",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      choice(seq($.usage_body, optional(";")), ";"),
    )),

  // Per spec (SysML.xtext:2098-2099): stakeholder [Name] Usage
  // StakeholderMembership wraps a StakeholderUsage (returns PartUsage).
  // Scoped to requirement_body only (a RequirementBodyItem); mirrors actor_usage. (G08f)
  stakeholder_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      "stakeholder",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      choice(seq($.usage_body, optional(";")), ";"),
    )),

  // Per spec (SysML.xtext:2158-2159): ConcernUsage uses RequirementBody.
  // Peeled out of the generic `usage` fallback. (G08f)
  concern_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "concern",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.requirement_body),
      optional(";"),
    )),

  // Per spec (SysML.xtext:2403-2404): ViewpointUsage uses RequirementBody. (G08f)
  viewpoint_usage: ($) =>
    prec(2, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      "viewpoint",
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.requirement_body),
      optional(";"),
    )),

  // Per spec (SysML.xtext:2178-2191): CaseBody extends CalcBody with subject, actor, objective
  case_body: ($) =>
    seq(
      "{",
      repeat(
        choice(
          $.actor_usage,
          $.assume_constraint,
          $.require_constraint,
          $.frame_constraint,
          $.verify_constraint,
          $.textual_representation,
          $._definition_or_usage,
          $.metadata_usage
        )
      ),
      optional($.result_expression),
      "}"
    ),

  // === Generic usage fallback ===

  usage: ($) =>
    prec(0, seq(
      optional($.visibility_indicator),
      optional("abstract"),
      optional($.usage_prefix),
      // concern/viewpoint peeled to dedicated requirement_body rules (G08f).
      // exhibit peeled to dedicated exhibit_state_usage rule (G23) — the
      // "state"-alternative shape (SysML.xtext:1835-1841) needs its own
      // named/bare split, which a bare keyword-field fallback can't express.
      field("keyword", choice(
        "calc", "analysis", "verification", "view",
        "rendering", "metadata",
      )),
      optional($.short_name),
      optional(field("name", $._name)),
      repeat($._feature_specialization),
      optional($.default_value),
      optional(choice($.function_body, $.usage_body)),
      optional(";"),
    )),
};
