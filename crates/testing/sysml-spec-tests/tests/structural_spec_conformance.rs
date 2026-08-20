//! SSC — Structural well-formedness spec-conformance harness.
//!
//! Companion to the *runtime-semantic* sweep (`constraint_spec_conformance.rs`,
//! `requirement_spec_conformance.rs`, `calculation_spec_conformance.rs`, …).
//! Those gates ask "do we MEAN the right thing when we RUN it?"; this file asks
//! the orthogonal question the spec-obligation roll-up flagged as a distinct,
//! unowned sweep:
//!
//!   > "a large class of obligations is STRUCTURAL well-formedness
//!   >  (specialization chains, binding connectors, parameter positions) with
//!   >  no conformance gate anywhere." — `spec-obligations/README.md`
//!
//! i.e. "does the implementation REJECT (or at least FLAG) a model that is
//! structurally ill-formed per the spec?".
//!
//! ## What this measures
//!
//! The implementation surfaces model well-formedness through three callable
//! APIs, all reachable from this crate's dev-deps (`sysml-core`):
//!
//!   - `sysml_core::validate_semantic(&ModelGraph) -> Vec<SemanticError>`
//!     — the generated dispatcher over `semantic_rules.toml` (rule ids S0xx,
//!       e.g. S060 at-most-one-subject, S125 send-payload). Hand-written checks
//!       live in `sysml-core/src/semantic_checks/`.
//!   - `ModelGraph::validate_structure() -> Vec<StructuralError>`
//!     — graph integrity (orphans E001, cycles E002, dangling refs E003-E008).
//!   - `ModelGraph::validate_relationship_types() -> Vec<StructuralError>`
//!     — relationship source/target kind constraints.
//!
//! This is the exact stack `sysml-ide-db::analysis::validate_graph` runs for
//! every editor diagnostic, so a fired/not-fired result here is the behavior a
//! user actually sees. Each test parses a VIOLATING and (where meaningful) a
//! CONFORMING fixture, elaborates it (the additive pass the IDE also runs), and
//! collects diagnostics from all three APIs.
//!
//! ## Verdict convention (matches the sibling suites)
//!
//! - `// VERDICT: CONFORMS` — a validator emits a diagnostic for the violation
//!   AND the conforming fixture is clean (of that rule). The obligation is
//!   gated.
//! - `// VERDICT: UNIMPLEMENTED — <obligation>` — NO validator flags the
//!   violation. The test pins the current no-diagnostic behavior so the gap is
//!   recorded and a future fix that closes it will trip this test (forcing the
//!   verdict to be flipped to CONFORMS). **The absence IS the finding.**
//!
//! Every test carries an `// OBL:` line whose id matches the obligation tracker
//! in `crates/testing/sysml-spec-tests/spec-obligations/*.md`. `STRUCTURAL`-tier
//! rows there are the authority for spec citations; this file is the gate.
//!
//! NO LSP, NO SysmlService, NO production-code changes — this file measures.
//! The self-scanning `ssc_matrix_summary` prints the CONFORMS/UNIMPLEMENTED
//! counts.

use sysml_core::elaborate::elaborate;
use sysml_core::{ModelGraph, SemanticError};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Diagnostics from all three well-formedness APIs for a piece of source.
struct Wellformedness {
    /// Semantic-rule errors (S0xx), as `(rule_id, message)`.
    semantic: Vec<(&'static str, String)>,
    /// Structural-integrity errors (E0xx), rendered.
    structural: Vec<String>,
}

impl Wellformedness {
    fn has_rule(&self, rule_id: &str) -> bool {
        self.semantic.iter().any(|(id, _)| *id == rule_id)
    }
    /// Total diagnostics across the semantic + structural surfaces.
    fn total(&self) -> usize {
        self.semantic.len() + self.structural.len()
    }
}

/// Parse → elaborate → run every well-formedness validator.
///
/// This mirrors `sysml-ide-db::analysis::validate_graph`, the single home the
/// IDE/CLI both bottom out in (`validate_semantic` + `validate_structure` +
/// `validate_relationship_types`). Parse errors fail the test immediately —
/// each fixture must be syntax the tree-sitter grammar accepts, otherwise the
/// case measures the parser instead of the validators.
fn check(source: &str) -> Wellformedness {
    let parser = TreeSitterParser::new();
    let mut result =
        parser.parse(&[SysmlFile::new("structural_spec_conformance.sysml", source)]);
    let parse_errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(
        parse_errors.is_empty(),
        "fixture source must parse cleanly, got: {parse_errors:?}"
    );

    // Additive elaboration — the same pass the IDE runs before validating.
    let _ = elaborate(&mut result.graph);

    let graph: &ModelGraph = &result.graph;
    let semantic: Vec<(&'static str, String)> = sysml_core::validate_semantic(graph)
        .into_iter()
        .map(|e: SemanticError| (e.rule_id, e.to_string()))
        .collect();
    let mut structural: Vec<String> = graph
        .validate_structure()
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    structural.extend(
        graph
            .validate_relationship_types()
            .into_iter()
            .map(|e| e.to_string()),
    );

    Wellformedness {
        semantic,
        structural,
    }
}

// ===========================================================================
// CONFORMS — the validator stack flags the violation and clears the conforming
// model. These are the structural obligations that DO have a working gate.
// ===========================================================================

// ---------------------------------------------------------------------------
// OBL: at-most-one-subject (requirements.md / S060,S061)
// "A requirement has at most one subject." (§8.3.21; `cardinality::at_most_one_subject`)
// ---------------------------------------------------------------------------

#[test]
fn req_two_subjects_is_flagged() {
    // OBL: at-most-one-subject
    // VERDICT: CONFORMS
    let v = check("package P { requirement def R { subject s1; subject s2; } }");
    assert!(
        v.has_rule("S060"),
        "two subjects on a requirement def must raise S060, got {:?}",
        v.semantic
    );
}

#[test]
fn req_one_subject_is_clean() {
    // OBL: at-most-one-subject
    // VERDICT: CONFORMS
    let v = check("package P { requirement def R { subject s1; } }");
    assert!(
        !v.has_rule("S060"),
        "a single subject is well-formed; S060 must not fire, got {:?}",
        v.semantic
    );
}

// ---------------------------------------------------------------------------
// OBL: model-element-must-be-owned (structural well-formedness / E001)
// Every non-root element lives inside a namespace. A bare top-level usage/def
// (not a package) is an orphan. (`structural_validation::OrphanElement`)
// This is THE working structural gate; it anchors that the structural surface
// is live (not silently a no-op) for the UNIMPLEMENTED cases below.
// ---------------------------------------------------------------------------

#[test]
fn orphan_top_level_definition_is_flagged() {
    // OBL: model-element-must-be-owned
    // VERDICT: CONFORMS
    let v = check("part def Free;");
    assert!(
        v.structural.iter().any(|m| m.contains("must be inside a namespace")),
        "a bare top-level part def is an orphan (E001), got {:?}",
        v.structural
    );
}

#[test]
fn definition_inside_package_is_not_orphan() {
    // OBL: model-element-must-be-owned
    // VERDICT: CONFORMS
    let v = check("package P { part def Inside; }");
    assert!(
        !v.structural.iter().any(|m| m.contains("must be inside a namespace")),
        "a package-owned def is well-formed; no orphan error expected, got {:?}",
        v.structural
    );
}

// ---------------------------------------------------------------------------
// OBL: send-action-has-payload (actions.md / S125)
// §8.3.17.15 `validateSendActionParameters` — "A SendActionUsage must have at
// least three owned input parameters, corresponding to its payload, sender and
// receiver" (`inputParameters()->size() >= 3`); the payload is the first
// argument (`deriveSendActionUsagePayloadArgument: payloadArgument =
// argument(1)`). A send with no payload argument is ill-formed.
//
// The obligation has three faces, gated by the three tests below:
//   1. SYNTAX — the tree-sitter grammar makes the payload mandatory
//      (`send_action: seq("send", _expression, …)`), so a payload-less send is
//      a parse error: it is NOT grammatically representable.
//   2. CONFORMING — a well-formed `send <payload> …` carries its payload as an
//      Expression subtree child, so S125 does NOT fire.
//   3. VALIDATOR (defensive) — a `SendActionUsage` assembled without any
//      payload (reachable only via a hand-built graph, since syntax forbids it)
//      DOES raise S125 (`actions::send_action_has_payload`).
//
// (The prior fixture `send sig to tgt;` expected S125 for a VALID send — it
// contained the payload `sig` — and was corrected here on 2026-07-30.)
// ---------------------------------------------------------------------------

#[test]
fn payload_less_send_is_a_parse_error() {
    // OBL: send-action-has-payload
    // VERDICT: CONFORMS (syntax face)
    // `send to tgt;` omits the payload; the grammar cannot represent it, so the
    // parser must report an error rather than yield a payload-less send.
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new(
        "structural_spec_conformance.sysml",
        "package P { action def A { send to tgt; } }",
    )]);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.severity == sysml_span::Severity::Error),
        "a send with no payload is not grammatically representable and must be \
         rejected by the parser, got diagnostics {:?}",
        result.diagnostics
    );
}

#[test]
fn send_with_payload_is_clean() {
    // OBL: send-action-has-payload
    // VERDICT: CONFORMS (conforming face)
    let v = check("package P { action def A { send sig to tgt; } }");
    assert!(
        !v.has_rule("S125"),
        "a send carrying its payload (`sig`) is well-formed; S125 must not fire, got {:?}",
        v.semantic
    );
}

#[test]
fn send_action_without_payload_parameter_is_flagged() {
    // OBL: send-action-has-payload
    // VERDICT: CONFORMS (validator face)
    // Payload-less send is a parse error (see `payload_less_send_is_a_parse_error`),
    // so the only way to reach an unpayloaded SendActionUsage is a hand-built
    // graph. The validator must still catch it per validateSendActionParameters —
    // no arbitrary child is added; the send genuinely lacks a payload.
    use sysml_core::{Element, ElementId, ElementKind};

    let mut graph = ModelGraph::new();
    let send =
        Element::new(ElementId::new_v4(), ElementKind::SendActionUsage).with_name("snd");
    graph.add_element(send);
    let _ = elaborate(&mut graph);

    let diagnostics = sysml_core::validate_semantic(&graph);
    assert!(
        diagnostics.iter().any(|e: &SemanticError| e.rule_id == "S125"),
        "a SendActionUsage with no payload parameter must raise S125, got {:?}",
        diagnostics
            .iter()
            .map(|e| (e.rule_id, e.to_string()))
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
// UNIMPLEMENTED — no validator flags the violation. Each test pins the current
// no-diagnostic behavior; the absence of a check is the recorded finding.
// ===========================================================================

// ---------------------------------------------------------------------------
// RECLASSIFIED 2026-06-21 — SATISFIED-BY-CONSTRUCTION (core-steward ruling).
// Three base-specialization obligations previously gated here as #[ignore]d
// UNIMPLEMENTED tests were REMOVED (not real validator obligations):
//   - calc-def-must-specialize-Calculation        (§8.3.19.2)
//   - constraint-def-specializes-check            (§7.20)
//   - verification-case-specializes-library-base  (§8.3.24.3/4)
// SysML derives the library-base specialization for every conforming model via
// implicit generalization; this engine faithfully ports that pass
// (`sysml-core::elaborate::implicit_generalization`, mapping table entries for
// CalculationDefinition → Calculations::Calculation, ConstraintDefinition →
// Constraints::ConstraintCheck, VerificationCaseDefinition →
// VerificationCases::VerificationCase). After elaboration-with-library the base
// specialization is PRESENT by construction, so there is nothing for a validator
// to flag. The minting mechanism is gated by `implicit_generalization.rs` unit
// tests. The removed fixtures used the no-library `check()` path, where
// `resolve_base` can never observe the implicit base — the gates were mis-framed.
// See calculations.md / constraints-expressions.md / verification-analysis-cases.md.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// OBL: requirement-subject-must-be-first-parameter (requirements.md, STRUCTURAL)
// §8.3.21 validateRequirementDefinitionSubjectParameterPosition:
//   `input->notEmpty() and input->first() = subjectParameter`.
// KerML derives `/input` as the features whose direction is `in`/`inout`; a plain
// `attribute` is directionless and so is NOT an input. A genuine violation
// therefore requires a *directed* parameter (`in earlier`) declared BEFORE the
// subject — here it is, so the subject is not `input->first()`. (S141)
// ---------------------------------------------------------------------------

#[test]
fn requirement_subject_not_first_parameter_is_flagged() {
    // OBL: requirement-subject-must-be-first-parameter
    // VERDICT: CONFORMS — S141 flags a directed input declared before the subject.
    let v = check("package P { requirement def R { in earlier; subject s; } }");
    // The single declared subject must NOT trip the cardinality rule — the only
    // violation here is ordering, which a correct validator MUST flag.
    assert!(
        !v.has_rule("S060") && !v.has_rule("S061"),
        "single subject must not raise a cardinality error, got {:?}",
        v.semantic
    );
    // SPEC-CORRECT expectation: a validator MUST flag the subject declared after
    // another input (it is not the first parameter), producing ≥1 diagnostic.
    assert!(
        v.total() > 0,
        "a subject that is not the requirement's first input must be flagged \
         (§8.3.21 validateRequirementDefinitionSubjectParameterPosition), got {:?} / {:?}",
        v.semantic,
        v.structural
    );
}

// ---------------------------------------------------------------------------
// OBL: port-usage-referential (flows-ports.md, STRUCTURAL)
// §8.3.x `validatePortUsageNestedUsagesNotComposite` (S145) +
// `validatePortDefinitionOwnedUsagesNotComposite` (S146): a port's non-port
// nested/owned usages must be referential (non-composite). Compositeness follows
// the SysML occurrence-default (see `semantic_checks::composite`). (CONFORMS S145/S146)
// ---------------------------------------------------------------------------

#[test]
fn port_usage_with_composite_nested_part_is_flagged() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — S145 flags a composite (default) part nested in a port usage.
    let v = check("package P { port def Pt { } part def A { port p : Pt { part nested; } } }");
    assert!(
        v.total() > 0,
        "a composite part nested in a PortUsage must be flagged \
         (validatePortUsageNestedUsagesNotComposite), got {:?} / {:?}",
        v.semantic,
        v.structural
    );
}

#[test]
fn port_definition_with_composite_owned_part_is_flagged() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — S146 flags a composite (default) part owned by a port def.
    let v = check("package P { port def Pt { part owned; } }");
    assert!(
        v.total() > 0,
        "a composite part owned by a PortDefinition must be flagged \
         (validatePortDefinitionOwnedUsagesNotComposite), got {:?} / {:?}",
        v.semantic,
        v.structural
    );
}

#[test]
fn port_usage_with_referential_nested_usage_is_clean() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — an attribute is referential by nature (not an
    // OccurrenceUsage), so a nested attribute in a port must NOT be flagged.
    let v = check("package P { port def Pt { } part def A { port p : Pt { attribute a; } } }");
    assert!(
        v.total() == 0,
        "a referential nested usage in a port is well-formed; nothing must flag, \
         got {:?} / {:?}",
        v.semantic,
        v.structural
    );
}

#[test]
fn metadata_annotation_in_port_definition_is_not_an_owned_usage() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — S146 quantifies over `ownedUsage`, which subsets
    // `ownedFeature` ("The Usages that are ownedFeatures of this Definition"),
    // and KerML derives `ownedFeature = ownedFeatureMembership.ownedMemberFeature`
    // with `ownedFeatureMembership = ownedRelationship->selectByKind(FeatureMembership)`.
    // A `@Signal;` annotation is owned via `DefinitionMember`/`AnnotatingMember`,
    // both of which `return SysML::OwningMembership` — NOT a FeatureMembership —
    // so it is an ownedMember but never an ownedUsage. It must not be flagged,
    // even though `MetadataUsage` is also an `ItemUsage` (→ OccurrenceUsage,
    // composite by default).
    let v = check(
        "package P { metadata def Signal; item def R; \
         port def Pt { @Signal; out item reading : R; } }",
    );
    assert!(
        !v.has_rule("S146"),
        "a metadata annotation inside a port def is an annotating member, not an \
         ownedUsage — S146 must not fire, got {:?}",
        v.semantic
    );
}

#[test]
fn metadata_annotation_in_port_usage_is_not_a_nested_usage() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — the S145 twin of the case above. `nestedUsage =
    // ownedFeature->selectByKind(Usage)` (deriveUsageNestedUsage), so an
    // annotating member is outside the quantifier here too.
    let v = check(
        "package P { metadata def Signal; port def Pt { } \
         part def A { port p : Pt { @Signal; } } }",
    );
    assert!(
        !v.has_rule("S145"),
        "a metadata annotation nested in a port usage is an annotating member, not a \
         nestedUsage — S145 must not fire, got {:?}",
        v.semantic
    );
}

#[test]
fn composite_owned_part_still_flags_alongside_a_metadata_annotation() {
    // OBL: port-usage-referential
    // VERDICT: CONFORMS — negative control for the two cases above: exempting
    // annotating members must NOT blunt the rule. A genuine composite (default,
    // undirected) part owned by the same port def still raises S146.
    let v = check(
        "package P { metadata def Signal; port def Pt { @Signal; part owned; } }",
    );
    assert!(
        v.has_rule("S146"),
        "a composite part owned by a PortDefinition must still be flagged when the \
         port also carries an annotation, got {:?} / {:?}",
        v.semantic,
        v.structural
    );
}

// ---------------------------------------------------------------------------
// OBL: at-most-one-each-subaction-kind (state-machines.md / S068,S069, STRUCTURAL)
// §8.3.18.5/6: ≤1 entry, ≤1 do, ≤1 exit per state. A check FUNCTION exists
// (`cardinality::at_most_one_state_subaction`) but it counts
// `StateSubactionMembership` children carrying a `kind` ∈ {entry,do,exit}.
// The tree-sitter parser lowers `entry action …` to a plain `ActionUsage`
// (tagged with a `stateSubactionKind` prop) and emits NO StateSubactionMembership,
// so the validator never matches real parsed input — the obligation is
// effectively ungated. Probe-confirmed: two `entry action`s ⇒ zero diagnostics.
// ---------------------------------------------------------------------------

#[test]
fn two_entry_subactions_on_state_def_is_flagged() {
    // OBL: at-most-one-each-subaction-kind
    // VERDICT: CONFORMS — GAP-STRUCT (S068/S069) closed: at_most_one_state_subaction
    //   now counts the parser's ActionUsage + stateSubactionKind shape (not just the
    //   never-minted StateSubactionMembership), so >1 entry/do/exit raises S068/S069.
    // SPEC-CORRECT expectation: two entry subactions on a state def violate the
    // ≤1-per-kind rule and MUST raise S068 or S069.
    let v = check("package P { state def Door { entry action a; entry action b; } }");
    assert!(
        v.has_rule("S068") || v.has_rule("S069"),
        "two entry subactions on a state def must raise S068/S069 \
         (§8.3.18.5/6 at-most-one entry), got {:?}",
        v.semantic
    );
}

// ---------------------------------------------------------------------------
// OBL: case-has-subject-and-objective (verification-analysis-cases.md / S064,S065, STRUCTURAL)
// §8.3.22.2: a case has ≤1 objective. The check FUNCTION exists
// (`cardinality::at_most_one_objective`, registered for CaseDefinition/CaseUsage)
// but the generated dispatcher routes by exact ElementKind: a `use case def`
// is a UseCaseDefinition (a *subtype* of CaseDefinition), so the rule is never
// dispatched to it. Probe-confirmed: two objectives ⇒ zero diagnostics.
// ---------------------------------------------------------------------------

#[test]
fn two_objectives_on_use_case_def_is_flagged() {
    // OBL: case-has-subject-and-objective
    // VERDICT: CONFORMS — GAP-STRUCT (S064/S065) closed: the semantic-validation
    //   dispatcher is now hierarchy-aware (sysml-core/build.rs passes the type
    //   hierarchy), so a rule registered on CaseDefinition also fires on the
    //   UseCaseDefinition subtype. SysML §8.3.22.2.
    // SPEC-CORRECT expectation: two objectives on a (use) case def violate the
    // ≤1-objective rule and MUST raise S064 or S065.
    let v = check("package P { use case def U { objective o1; objective o2; } }");
    assert!(
        v.has_rule("S064") || v.has_rule("S065"),
        "two objectives on a use case def must raise S064/S065 \
         (§8.3.22.2 at-most-one objective), got {:?}",
        v.semantic
    );
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn ssc_matrix_summary() {
    let src = include_str!("structural_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    // Distinct obligations probed (one `// OBL:` id may carry both a violating
    // and a conforming case). The section-header comments also start with
    // `// OBL:` but append a ` (` parenthetical citation — take only the bare
    // id before any space so headers and per-test markers collapse together.
    let obligations: std::collections::BTreeSet<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter_map(|l| l.strip_prefix("// OBL: "))
        .map(|id| id.split(' ').next().unwrap_or(id))
        .collect();
    println!(
        "SSC structural well-formedness matrix: {} verdict-marked cases over {} \
         distinct obligations — {conforms} CONFORMS, {unimpl} UNIMPLEMENTED",
        verdicts.len(),
        obligations.len()
    );
    assert!(
        verdicts.len() >= 11,
        "expected >=11 verdict-marked structural gates"
    );
    // After the 2026-06-21 fix-wave the structural well-formedness backlog is fully
    // closed: S068/S069 + S064/S065 + subject-first-parameter (S141-S144) +
    // port-usage-referential (S145/S146) all CLOSED; the three base-specialization
    // obligations reclassified SATISFIED-BY-CONSTRUCTION and removed. No structural
    // obligation remains UNIMPLEMENTED. A NEW unimplemented structural gap must be
    // recorded here (and this assertion updated) rather than silently dropped.
    assert_eq!(
        unimpl, 0,
        "the structural well-formedness backlog is fully closed; a new UNIMPLEMENTED \
         gap must be recorded explicitly, got {unimpl}"
    );
}
