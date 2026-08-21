//! Machine-readable known-gap registry. Each record documents a real,
//! reviewed implementation limitation — a place where the tree-sitter lowering
//! under-serves a normative concept. The registry is the single home referenced
//! by both the `tooling.implementation.*` cards it drives (their `known_gaps`
//! array) and, later, by evidence records that `justifies` an `unsupported`
//! axis (`known_gap_ref`, evidence-record schema). Cards are minted FROM this
//! list, so the registry and the cards can never drift.
//!
//! Honesty rule: a gap is never phrased as a language restriction — it is an
//! implementation limitation against a normative concept that DOES exist in
//! the grammar. Every record names the normative card(s) it qualifies and the
//! grammar authority it is grounded in.

use serde::Serialize;

/// One reviewed implementation limitation.
#[derive(Debug, Clone, Serialize)]
pub struct KnownGap {
    /// Stable registry id (referenced by card `known_gaps` / evidence `known_gap_ref`).
    pub id: String,
    pub title: String,
    /// Coarse kind (e.g. `parser-lowering-gap`).
    pub kind: String,
    /// Original paraphrase of the limitation (never reproduced spec prose).
    pub summary: String,
    /// The primary grammar concept the limitation is about (drives the tooling
    /// card's keyword slug + alias).
    pub concept: String,
    /// Every grammar rule name this gap documents (includes `concept`). Each is a
    /// user-facing xtext concept that stays `uncarded` (coverage-lowering) with
    /// its denominator record annotated by this gap id — the honest home for
    /// concepts that parse but whose lowering does not materialize them as a
    /// distinct kind, so no card can describe them faithfully.
    pub covers: Vec<String>,
    /// `kerml` or `sysml` — which grammar declares the concept (grounds provenance).
    pub authority: String,
    /// The `tooling.implementation.*` card minted for this gap.
    pub tooling_card: String,
    /// Normative card(s) this gap qualifies (the tooling card links to these).
    pub related_cards: Vec<String>,
    /// `open` | `closed` (a closed gap stays recorded for audit).
    pub status: String,
    /// How the limitation was observed.
    pub evidence: String,
}

/// The known-gap annotation for an uncarded grammar concept, if any: the gap
/// registry id, its tooling card, and its status. Used by the denominator
/// closure to enrich an `uncarded` record's rationale so the honest coverage gap
/// carries an auditable reason instead of the generic "no card yet".
pub fn uncarded_gap_note(name: &str) -> Option<(String, String, String)> {
    registry()
        .into_iter()
        .find(|g| g.covers.iter().any(|c| c == name))
        .map(|g| (g.id, g.tooling_card, g.status))
}

/// The curated known-gap registry. Two real tree-sitter lowering gaps found by
/// probing the lowering with targeted sources: TextualRepresentation and EnumerationUsage.
pub fn registry() -> Vec<KnownGap> {
    vec![
        KnownGap {
            id: "tree-sitter.textual-representation-generic-lowering".to_owned(),
            title: "TextualRepresentation lowers to a generic ReferenceUsage".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "A KerML TextualRepresentation annotating element (a `rep` with a `language` \
                      string and a body) parses cleanly, but the tree-sitter lowering produces a \
                      generic ReferenceUsage rather than a distinct TextualRepresentation element, \
                      so the representation's language and body are not modelled as such. This is \
                      an implementation limitation, not a language restriction — the concept \
                      exists in the grammar."
                .to_owned(),
            concept: "TextualRepresentation".to_owned(),
            covers: vec!["TextualRepresentation".to_owned()],
            authority: "kerml".to_owned(),
            tooling_card: "tooling.implementation.textual-representation-generic-lowering".to_owned(),
            related_cards: vec!["sysml.structure.reference-usage".to_owned()],
            status: "closed".to_owned(),
            evidence: "Originally observed by parsing a `rep` annotating element and inspecting \
                       the lowered element kinds: TextualRepresentation lowered to ReferenceUsage \
                       with no distinct element kind. CLOSED: it now lowers to a distinct \
                       TextualRepresentation element carrying its `language` + body, gated by \
                       spec_property_conformance test arms."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.enumeration-usage-not-distinct".to_owned(),
            title: "enum members do not lower to a distinct EnumerationUsage".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "Members declared with `enum` inside an `enum def` parse cleanly, but the \
                      lowering produces no model element for them at all — not merely a generic \
                      usage in place of a distinct EnumerationUsage, but zero elements — so \
                      enumerated-value membership is not modelled in any form. This is an \
                      implementation limitation, not a language restriction — the concept exists \
                      in the grammar."
                .to_owned(),
            concept: "EnumerationUsage".to_owned(),
            covers: vec!["EnumerationUsage".to_owned(), "EnumeratedValue".to_owned()],
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.enumeration-usage-not-distinct".to_owned(),
            related_cards: vec!["sysml.structure.enumeration-definition".to_owned()],
            status: "closed".to_owned(),
            evidence: "Originally observed by parsing an `enum def` with members and inspecting \
                       the lowered element kinds: `enum` members produced zero model elements. \
                       CLOSED in stages: each enumerated value now lowers to a distinct \
                       EnumerationUsage wrapped in a VariantMembership and typed by the enum def; \
                       the enum def stamps isAbstract (isVariation implies abstract), clearing a \
                       spurious S080; the unnamed value form `= 60.0;` lowers to an anonymous \
                       EnumerationUsage; and the standalone `enum e : Color;` usage form parses to \
                       a dedicated `enum_usage` node (named/bare split, same shape as port_usage) \
                       and lowers to an ordinary-membership EnumerationUsage riding the standard \
                       usage path (typing/subsetting/usage-prefix flags). Pilot-corpus parity for \
                       the enumeration test model improved on both axes. Remaining related \
                       misparse: `ref enum` splits (the pre-existing `ref <keyword-usage>` split, \
                       see tree-sitter.prefixed-def-and-ref-keyword-usage-misparse)."
                .to_owned(),
        },
        // --- Uncarded-tail gaps: user-facing concepts that PARSE cleanly but
        // whose tree-sitter lowering does not materialize the distinct
        // element kind the grammar declares (confirmed by parsing probe
        // sources and inspecting the lowered element-kind statistics).
        // They stay `uncarded` (honestly coverage-lowering); this registry is
        // their auditable home and mints one tooling.implementation.* card each.
        KnownGap {
            id: "tree-sitter.type-relationship-fragment-generic-lowering".to_owned(),
            title: "type-relationship operators do not materialize a distinct relationship kind".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "The type-relationship operators unioning (`unions`), intersecting \
                      (`intersects`), differencing (`differences`), disjoining (`disjoint from`), \
                      feature-inversion (`inverse of`), type-featuring (`featured by`) and \
                      conjugation (`conjugate`/`~` on a type) parse cleanly, but the lowering does \
                      not produce their distinct Unioning/Intersecting/Differencing/Disjoining/ \
                      FeatureInverting/TypeFeaturing/Conjugation relationship elements — the \
                      participating types are recorded without the specialization relationship \
                      being modelled as such. An implementation limitation, not a language \
                      restriction; the concepts exist in the grammar. (The `~` port-conjugation \
                      form on ports IS modelled — see the conjugated-port-definition card.)"
                .to_owned(),
            concept: "Unioning".to_owned(),
            covers: vec![
                "Unioning".to_owned(),
                "Intersecting".to_owned(),
                "Differencing".to_owned(),
                "Disjoining".to_owned(),
                "FeatureInverting".to_owned(),
                "TypeFeaturing".to_owned(),
                "Conjugation".to_owned(),
                "ClassifierConjugation".to_owned(),
                "FeatureConjugation".to_owned(),
                "PortConjugation".to_owned(),
            ],
            authority: "kerml".to_owned(),
            tooling_card: "tooling.implementation.type-relationship-fragment-generic-lowering".to_owned(),
            related_cards: vec!["kerml.structure.specialization".to_owned()],
            status: "closed".to_owned(),
            evidence: "Originally observed by parsing `classifier C unions T1, T2` and sibling \
                       forms and inspecting the lowered element kinds: only Type/Classifier \
                       elements were produced — no relationship kind. CLOSED: unions/intersects/\
                       differences/disjoint-from/featured-by/inverse-of now mint their Unioning/\
                       Intersecting/Differencing/Disjoining/TypeFeaturing/FeatureInverting \
                       relationship elements (the existing pass-2 resolvers already consumed \
                       them), and Conjugation is minted via the `conjugates` keyword. The \
                       symbolic `~` spelling also parses everywhere except lambda-parameter \
                       position — a separate `symbolic_conjugation` rule aliased back to \
                       `conjugation_clause` (one node kind, unchanged lowering), excluded only \
                       from `lambda_parameter` where the optional terminator makes `{ in x ~Y }` \
                       genuinely ambiguous with a unary-`~` result expression. Spec-moot there: \
                       KerML FunctionBody members carry terminators, and the `conjugates` keyword \
                       still parses in lambda position (regression-pinned)."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.occurrence-prefix-generic-lowering".to_owned(),
            title: "individual / portion occurrence prefixes do not lower to distinct kinds".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "The occurrence prefixes individual (`individual def`/`individual`), \
                      portion (`portion`), snapshot and timeslice parse but do not lower to their \
                      distinct occurrence element kinds — `individual def` in particular is not \
                      recognised as an OccurrenceDefinition marked individual, and the usage \
                      prefixes drop to a generic ReferenceUsage/OccurrenceUsage without their \
                      individual/portion role. An implementation limitation, not a language \
                      restriction; the concepts exist in the grammar."
                .to_owned(),
            concept: "IndividualDefinition".to_owned(),
            covers: vec![
                "IndividualDefinition".to_owned(),
                "IndividualUsage".to_owned(),
                "PortionUsage".to_owned(),
            ],
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.occurrence-prefix-generic-lowering".to_owned(),
            related_cards: vec![
                "sysml.structure.occurrence-definition".to_owned(),
                "sysml.structure.occurrence-usage".to_owned(),
            ],
            status: "closed".to_owned(),
            evidence: "MIS-FRAMING CORRECTION (the original filing misread xtext RULE names as \
                       metaclasses): IndividualDefinition / IndividualUsage / PortionUsage are NOT \
                       metaclasses — they are xtext rules that RETURN OccurrenceDefinition / \
                       OccurrenceUsage with isIndividual / portionKind set (SysML-vocab.ttl has no \
                       such classes). The spec-faithful fix is flags on the occurrence kinds, not \
                       new kinds. The USAGE forms `individual`/`snapshot`/`timeslice` now promote \
                       from the generic ReferenceUsage default to OccurrenceUsage with \
                       isIndividual/portionKind (guarded so a specific occurrence subtype keeps \
                       its kind and just gains the flag). CLOSED: the bare `individual def X;` \
                       DEFINITION form now parses to a dedicated `individual_def` node and lowers \
                       to OccurrenceDefinition + isIndividual, composing with the `abstract` \
                       prefix (SysML.xtext IndividualDefinition:813-817; the \
                       EmptyMultiplicityMember :819-825 is consciously simplified flags-first, \
                       documented at the dispatch arm). The PREFIXED-definition family \
                       (`individual part def` / `individual occurrence def` / `variation part \
                       def`) still misparses — filed separately as \
                       tree-sitter.prefixed-def-and-ref-keyword-usage-misparse."
                .to_owned(),
        },
        // FILED (not fixed): honest grammar misparses that survive the current
        // grammar. Two mechanically-related LR(1) shapes,
        // one record: (a) `individual|variation <kind> def` prefixed-def forms
        // (`individual part def P;`, `individual occurrence def O;`,
        // `variation part def V;`) split into a standard_usage + a
        // feature_declaration — the parser must choose usage-vs-definition on
        // the same `individual|variation <kw>` lookahead and the paths diverge
        // only at the later `def` token (LR(1) shift/reduce, not resolvable by
        // a conflict declaration without a GLR fork); (b) the pre-existing
        // `ref <keyword-usage>` split (`ref enum e;`, `ref port p;`) into an
        // EMPTY standard_usage + the keyword usage — same one-token-later
        // divergence, a pre-existing split. `covers` is empty: the affected
        // concepts (OccurrenceDefinition/PartDefinition/EnumerationUsage/
        // PortUsage and their prefixes) are carded; this record documents the
        // specific surface-form misparses for audit.
        KnownGap {
            id: "tree-sitter.prefixed-def-and-ref-keyword-usage-misparse".to_owned(),
            title: "prefixed `individual|variation <kind> def` and `ref <keyword-usage>` forms misparse"
                .to_owned(),
            kind: "parser-grammar-gap".to_owned(),
            summary: "Two LR(1) lookahead splits: (a) `individual part def` / `individual \
                      occurrence def` / `variation part def` misparse into a standard_usage plus a \
                      feature_declaration — the usage-vs-definition choice must be made on the \
                      prefix+keyword lookahead but the forms diverge one token later at `def`; \
                      (b) `ref enum` / `ref port` (any `ref <keyword-usage>`) splits into an empty \
                      standard_usage plus the keyword usage. Implementation limitations, not \
                      language restrictions — both forms are normative (SysML.xtext \
                      OccurrenceDefinitionPrefix:800-806 / IndividualDefinition:813-817 / \
                      DefinitionPrefix variation, and RefPrefix on usages). The bare forms \
                      (`individual def X;`, standalone `enum e;`, `port p;`) all parse and lower."
                .to_owned(),
            concept: "OccurrenceDefinitionPrefix".to_owned(),
            covers: Vec::new(),
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.prefixed-def-and-ref-keyword-usage-misparse".to_owned(),
            related_cards: vec![
                "sysml.structure.occurrence-definition".to_owned(),
                "sysml.structure.part-definition".to_owned(),
                "sysml.structure.enumeration-definition".to_owned(),
                "sysml.structure.port-usage".to_owned(),
            ],
            status: "open".to_owned(),
            evidence: "Observed by parsing the affected forms against the current generated \
                       parser and inspecting the syntax tree: `individual part def IP;` → \
                       standard_usage(prefix=individual, NAME=`def`) + feature_declaration(IP) — \
                       the grammar eats `def` as the usage name; same shape for `individual \
                       occurrence def` and `variation part def`. `ref enum e2;` / `ref port p2;` \
                       → EMPTY standard_usage(`ref`) + the enum_usage/port_usage. The bare \
                       `individual def` / standalone `enum` forms parse correctly under the same \
                       grammar, isolating the residual to the prefixed/ref spellings."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.control-node-if-terminate-generic-lowering".to_owned(),
            title: "if / terminate action nodes do not lower to distinct node kinds".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "The `if` control node and the `terminate` action node parse cleanly, but \
                      the lowering does not materialize their distinct IfActionUsage / \
                      TerminateActionUsage kinds — an `if` conditional succession lowers to a \
                      generic TransitionUsage and `terminate` does not surface a terminate node. \
                      An implementation limitation, not a language restriction; the grammar \
                      declares both node kinds."
                .to_owned(),
            concept: "IfNode".to_owned(),
            covers: vec!["IfNode".to_owned(), "TerminateNode".to_owned()],
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.control-node-if-terminate-generic-lowering".to_owned(),
            related_cards: vec![
                "sysml.behavior.transition-usage".to_owned(),
                "sysml.behavior.action-usage".to_owned(),
            ],
            status: "closed".to_owned(),
            evidence: "MIS-FRAMING CORRECTION (the original filing misread the construct): IfNode \
                       was NEVER broken — the filing example `if true then done` is a \
                       GuardedTargetSuccession, which is spec-correctly a TransitionUsage (xtext \
                       :1703), NOT an IfActionUsage; the real `if` action node already lowered to \
                       IfActionUsage. TerminateNode was a real gap, CLOSED: `terminate` now \
                       materializes TerminateActionUsage, and the NodeParameterMember target \
                       shape is materialized — an unnamed ReferenceUsage slot child owning the \
                       FeatureBinding expression subtree per SysML.xtext NodeParameterMember/\
                       NodeParameter/FeatureBinding (ParameterMembership/ReferenceUsage/\
                       FeatureValue are the real metaclasses; vocab terminatedOccurrenceArgument) \
                       — and elaborate::actions stamps an ADDITIVE resolvedTerminatedOccurrence \
                       Value::Ref from the retained unresolved_target string; dotted-chain \
                       targets stay honestly unstamped pending chain-aware resolution. Grammar \
                       residuals (filed): the `action <name> terminate;` prefix form ERRORs and \
                       inline `then terminate;` is not materialized."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.message-generic-lowering".to_owned(),
            title: "message / message-event do not lower to distinct kinds".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "A `message` (and its message event) parses cleanly but lowers to a generic \
                      FlowUsage / EventOccurrenceUsage rather than a distinct Message / \
                      MessageEvent, so the message's payload-transfer semantics are not modelled \
                      as such. An implementation limitation, not a language restriction; the \
                      concepts exist in the grammar."
                .to_owned(),
            concept: "Message".to_owned(),
            covers: vec!["Message".to_owned(), "MessageEvent".to_owned()],
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.message-generic-lowering".to_owned(),
            related_cards: vec![
                "sysml.structure.flow-connection".to_owned(),
                "sysml.structure.event-occurrence-usage".to_owned(),
            ],
            status: "closed".to_owned(),
            evidence: "MIS-FRAMING CORRECTION (the original filing expected non-existent \
                       metaclasses): there is NO Message / MessageEvent metaclass — Message / \
                       MessageEvent are xtext rules that RETURN FlowUsage / EventOccurrenceUsage \
                       (§7.16: 'a message is modeled as a flow usage'). So the bare `message \
                       msg;` → FlowUsage+isMessage lowering was ALREADY spec-faithful \
                       (regression-pinned), not a gap. CLOSED: the `from a to b` ends now parse \
                       (from-required) and lower as source/target on the FlowUsage; the anonymous \
                       `message A to B;` end arm is restored (dedicated message_ends rule with \
                       exactly the two spec arms, name narrowed so the earlier LR ambiguity is \
                       fixed rather than the form dropped) and the field-based endpoint extractor \
                       picks it up unchanged; and the lowering materializes the full ends/payload \
                       nesting for FLOWS and messages alike, ADDITIVELY to the flat source/target: \
                       message end = ParameterMembership > EventOccurrenceUsage > \
                       ReferenceSubsetting (SysML.xtext :1254-1262); flow end = \
                       EndFeatureMembership > FlowEnd > [chain-prefix ReferenceSubsetting] + \
                       FeatureMembership > ReferenceUsage > Redefinition (:1309-1339); payload \
                       `of X` = FeatureMembership > PayloadFeature > FeatureTyping (:1289-1300), \
                       from which elaborate::flows derives payloadType (one home). Pilot-corpus \
                       parity baselines re-blessed with per-row justification."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.constructor-expression-generic-lowering".to_owned(),
            title: "constructor expression does not lower to a distinct kind".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "A constructor expression (`Type(args)`) parses cleanly but lowers to a \
                      generic InvocationExpression rather than a distinct ConstructorExpression, \
                      so it is not distinguished from an ordinary invocation. An implementation \
                      limitation, not a language restriction; the concept exists in the grammar."
                .to_owned(),
            concept: "ConstructorExpression".to_owned(),
            covers: vec!["ConstructorExpression".to_owned()],
            authority: "kerml".to_owned(),
            tooling_card: "tooling.implementation.constructor-expression-generic-lowering".to_owned(),
            related_cards: vec!["kerml.expression.invocation".to_owned()],
            status: "closed".to_owned(),
            evidence: "Originally observed by parsing `Pt(1, 2)` and inspecting the lowered \
                       element kinds: it lowered to InvocationExpression. CLOSED: the `new` \
                       keyword discriminates the distinct ConstructorExpression metaclass (which is \
                       real, Kerml-Vocab.ttl) from a plain InvocationExpression; consumers rerouted \
                       and implicit-generalization base constructorEvaluations wired. A plain \
                       `Widget(...)` still lowers to InvocationExpression, as it should."
                .to_owned(),
        },
        KnownGap {
            id: "tree-sitter.state-subaction-generic-lowering".to_owned(),
            title: "state effect / state action / trigger subactions do not lower to distinct kinds".to_owned(),
            kind: "parser-lowering-gap".to_owned(),
            summary: "A state effect behavior, a state action usage and a transition trigger \
                      action parse cleanly but lower to a generic ActionUsage / AcceptActionUsage \
                      rather than their distinct EffectBehaviorUsage / StateActionUsage / \
                      TriggerAction kinds — the state-subaction and trigger roles are not modelled \
                      as distinct usages. An implementation limitation, not a language \
                      restriction; the concepts exist in the grammar."
                .to_owned(),
            concept: "EffectBehaviorUsage".to_owned(),
            covers: vec![
                "EffectBehaviorUsage".to_owned(),
                "StateActionUsage".to_owned(),
                "TriggerAction".to_owned(),
            ],
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.state-subaction-generic-lowering".to_owned(),
            related_cards: vec![
                "sysml.behavior.exhibit-state".to_owned(),
                "sysml.behavior.accept-node".to_owned(),
            ],
            status: "closed".to_owned(),
            evidence: "MIS-FRAMING CORRECTION (the original filing misread xtext RULE names as \
                       metaclasses): EffectBehaviorUsage / StateActionUsage / TriggerAction are NOT \
                       metaclasses — they are xtext rules returning ActionUsage / AcceptActionUsage; \
                       the distinct structure the spec prescribes is the MEMBERSHIP kind wrapping \
                       them, not a distinct usage kind. CLOSED in two halves: \
                       StateSubactionMembership is materialized for state entry/do/exit, and \
                       trigger/guard/effect are real AcceptActionUsage/Expression/ActionUsage \
                       children wrapped in a TransitionFeatureMembership carrying kind \
                       (SysML.xtext:1884-1914, spec §8.3.18.8); the former \
                       trigger/guard/effect/accept_param string props are REMOVED (children carry \
                       the textual form as `text`; port-trigger accept param = real \
                       ReferenceUsage payload child). Residual: the guard child is text-only (no \
                       structured expression subtree) and the effect body is not lowered into \
                       child statements."
                .to_owned(),
        },
        // FILED (not fixed) as a runtime/resolution issue: a resolution
        // NONDETERMINISM, not a lowering gap. `covers` is empty (the affected
        // concepts ARE carded) — this record exists to document the flakiness and
        // carries no uncarded annotation.
        KnownGap {
            id: "resolution.membership-wrapped-role-usage-underresolves".to_owned(),
            title: "membership-wrapped role usages under-resolve their type refs nondeterministically".to_owned(),
            kind: "resolution-nondeterminism".to_owned(),
            summary: "Membership-wrapped role usages once stamped their typing/reference target as a \
                      string prop on the membership (no lowered intermediate usage), so the resolver's \
                      pass-1 loop hit a `_ => {}` no-op: the target was never resolved and a missing one \
                      never counted as unresolved — a silent drop (`subject s : Missing` gave no \
                      diagnostic), with nondeterminism from a runtime fallback scanning a HashMap in \
                      hash order. FIXED. Subject/objective: pass 1 routes them through \
                      `resolve_feature_typing` (resolves in the membership's owning scope, stamps a \
                      `type` Ref, fail-hards a dangling target). assume/require constraint and framed \
                      concern reference forms: the parser mints the membership's owned \
                      ConstraintUsage and hangs the grammar's relationship on it — a ReferenceSubsetting \
                      for the bare-name form (SysML.xtext:448), a FeatureTyping for the `: Def` form — \
                      and `referencedConstraint` derives from that (SysML-vocab.ttl:2576), the resolver \
                      fail-harding both through the standard path. actor/stakeholder were always \
                      unaffected (they lower a real FeatureTyping)."
                .to_owned(),
            concept: "SubjectMembership".to_owned(),
            covers: Vec::new(),
            authority: "sysml".to_owned(),
            tooling_card: "tooling.implementation.role-usage-resolution-nondeterminism".to_owned(),
            related_cards: vec![
                "sysml.requirements.subject".to_owned(),
                "sysml.requirements.actor".to_owned(),
                "sysml.requirements.stakeholder".to_owned(),
                "sysml.requirements.framed-concern".to_owned(),
                "sysml.cases.objective".to_owned(),
            ],
            status: "fixed".to_owned(),
            evidence: "Originally observed by resolving a model whose subject was typed `: Missing` \
                       repeatedly: the run passed then failed across repetitions, and \
                       objective/framed-concern typing was silently dropped inside the role body \
                       with no unresolved count. FIXED in two parts. First, subject + objective \
                       `: Missing` now emit a stable E200 across repeated runs (`no definition \
                       '..' found in scope of subject/objective membership`), and a resolvable \
                       type stamps `type` deterministically — verified across the core, parser, \
                       runtime and identity test suites, with the service command baselines \
                       re-blessed (a model_digest-only shift from the newly-resolved \
                       subject/objective type Refs). Second, framed-concern and assume/require \
                       reference forms now mint a real ReferenceSubsetting (bare-name) or \
                       FeatureTyping (`: Def`) on the owned ConstraintUsage and derive \
                       `referencedConstraint` from it — verified by the parser's \
                       requirement-constraint reference-form tests and the runtime, query and \
                       service contract suites. Gap fully closed."
                .to_owned(),
        },
    ]
}
