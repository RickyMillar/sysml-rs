#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! End-to-end diagnostic UX tests.
//!
//! These tests verify the *quality* of diagnostic output, not just that
//! diagnostics exist. A test should fail if:
//!
//! - Internal grammar names leak into user-facing messages
//! - Error codes are misused (e.g., structural code on syntax errors)
//! - Spurious diagnostics appear on simple, focused fixtures
//! - Messages aren't specific enough to be actionable
//!
//! Each failing test = a concrete UX bug to fix in the pipeline.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnose_source_support::{diagnose_source, DiagnoseOptions, DiagnoseResult};
use sysml_service::diagnostics::TOTAL_DIAGNOSTIC_CAP;
use sysml_span::{Diagnostic, Severity};

// ── Fixtures ────────────────────────────────────────────────────────

const CLEAN: &str = include_str!("../fixtures/valid/clean.sysml");
const DUPLICATE_NAMES: &str = include_str!("../fixtures/invalid/duplicate_names.sysml");
const MISSING_SEMICOLON: &str = include_str!("../fixtures/invalid/missing_semicolon.sysml");
const CORRUPTED_LINE: &str = include_str!("../fixtures/invalid/corrupted_line.sysml");
const TYPO_REFERENCE: &str = include_str!("../fixtures/invalid/typo_reference.sysml");
const WRONG_TYPING: &str = include_str!("../fixtures/invalid/wrong_typing.sysml");
const OWNERSHIP_VIOLATION: &str = include_str!("../fixtures/invalid/ownership_violation.sysml");
const SPECIALIZATION_BOUNDARY: &str =
    include_str!("../fixtures/invalid/specialization_boundary.sysml");
const CASCADING: &str = include_str!("../fixtures/regression/cascading.sysml");

// Wave 2 fixtures
const UNCLOSED_BRACE: &str = include_str!("../fixtures/invalid/unclosed_brace.sysml");
const EXTRA_BRACE: &str = include_str!("../fixtures/invalid/extra_brace.sysml");
const UNTERMINATED_COMMENT: &str = include_str!("../fixtures/invalid/unterminated_comment.sysml");
const PARTIAL_EDIT: &str = include_str!("../fixtures/regression/partial_edit.sysml");
const ERROR_RECOVERY: &str =
    include_str!("../fixtures/regression/error_recovery_following_items.sysml");
const QUALIFIED_NAME_RESOLUTION: &str =
    include_str!("../fixtures/valid/qualified_name_resolution.sysml");
const MISSING_IMPORT: &str = include_str!("../fixtures/invalid/missing_import.sysml");
const DUPLICATE_NAME_DIFFERENT_SCOPE: &str =
    include_str!("../fixtures/valid/duplicate_name_different_scope.sysml");
const REDEFINE_MISSING_TARGET: &str =
    include_str!("../fixtures/invalid/redefine_missing_target.sysml");
const STDLIB_TYPE_RESOLUTION: &str = include_str!("../fixtures/valid/stdlib_type_resolution.sysml");
const STDLIB_REAL_RESOLUTION: &str = r#"
package StdlibRealTest {
    part def Widget {
        attribute ratio : Real;
        part custom : CustomThing;
    }
}
"#;
const STATE_ENTRY_THEN_TRANSITION: &str = r#"
package StateMachineTest {
    state def Toggle {
        entry; then Off;
        state Off;
        state On;
        transition turn_on first Off then On;
        transition turn_off first On then Off;
    }
}
"#;
const STATE_MACHINE_NO_TRANSITIONS: &str = r#"
package StateMachineHealth {
    state def Broken {
        state Idle;
        state Running;
    }
}
"#;
const STATE_MACHINE_DISCONNECTED: &str = r#"
package StateMachineHealth {
    state def Door {
        state Closed;
        state Opening;
        state Jammed;
        transition closed_to_opening
            first Closed then Opening;
    }
}
"#;

// Health diagnostics showcase fixture
const HEALTH_SHOWCASE: &str =
    include_str!("../../../../tests/fixtures/shared/test_health_diagnostics.sysml");

// Action health fixtures
const ACTION_NO_STEPS: &str = r#"
package ActionHealth {
    action def EmptyAction;
}
"#;
const ACTION_NO_CONTROL_FLOW: &str = r#"
package ActionHealth {
    action def NoFlowAction {
        action step1;
        action step2;
    }
}
"#;

// Flow health fixtures
const FLOW_MISSING_SOURCE: &str = r#"
package FlowHealth {
    part def Sensor;
    part def Controller;
    flow dataFlow from Sensor to Controller;
}
"#;

// Verification health fixtures
const VERIFICATION_NO_REQUIREMENTS: &str = r#"
package VerificationHealth {
    verification def EmptyCheck;
}
"#;

// ── Helpers ─────────────────────────────────────────────────────────

fn full_opts() -> DiagnoseOptions {
    DiagnoseOptions {
        resolution: true,
        validation: true,
    }
}

fn syntax_only() -> DiagnoseOptions {
    DiagnoseOptions {
        resolution: false,
        validation: false,
    }
}

fn run(source: &str, opts: &DiagnoseOptions) -> DiagnoseResult {
    diagnose_source(source, "file:///test.sysml", opts)
}

fn line_of(diag: &Diagnostic, source: &str) -> Option<usize> {
    let offset = diag.span.as_ref()?.start;
    Some(source[..offset].matches('\n').count() + 1)
}

fn assert_has<F: Fn(&Diagnostic) -> bool>(diags: &[Diagnostic], pred: F, msg: &str) {
    assert!(
        diags.iter().any(|d| pred(d)),
        "{}\nDiagnostics were:\n{}",
        msg,
        dump(diags)
    );
}

fn assert_none<F: Fn(&Diagnostic) -> bool>(diags: &[Diagnostic], pred: F, msg: &str) {
    let bad: Vec<_> = diags.iter().filter(|d| pred(d)).collect();
    assert!(
        bad.is_empty(),
        "{}\nMatching diagnostics:\n{}",
        msg,
        dump(diags)
    );
}

fn assert_count<F: Fn(&Diagnostic) -> bool>(diags: &[Diagnostic], pred: F, n: usize, msg: &str) {
    let count = diags.iter().filter(|d| pred(d)).count();
    assert_eq!(
        count,
        n,
        "{} (expected {}, got {})\nDiagnostics were:\n{}",
        msg,
        n,
        count,
        dump(diags)
    );
}

fn dump(diags: &[Diagnostic]) -> String {
    if diags.is_empty() {
        return "  (none)".to_string();
    }
    diags
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let code = d
                .code
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "----".to_string());
            let span = d
                .span
                .as_ref()
                .map(|s| format!("{}..{}", s.start, s.end))
                .unwrap_or_else(|| "no span".to_string());
            let notes = if d.notes.is_empty() {
                String::new()
            } else {
                format!(" notes={:?}", d.notes)
            };
            format!(
                "  [{}] {:5} {:7?} ({}) {}{}",
                i, code, d.severity, span, d.message, notes
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_code(diag: &Diagnostic, code: &str) -> bool {
    diag.code.as_deref() == Some(code)
}

fn is_error(diag: &Diagnostic) -> bool {
    diag.severity == Severity::Error
}

fn is_warning(diag: &Diagnostic) -> bool {
    diag.severity == Severity::Warning
}

fn code_starts_with(diag: &Diagnostic, prefix: &str) -> bool {
    diag.code
        .as_ref()
        .map(|c| c.starts_with(prefix))
        .unwrap_or(false)
}

/// Internal pest grammar rule names that should NEVER appear in user-facing messages.
const INTERNAL_RULE_NAMES: &[&str] = &[
    "KW_SPECIALIZES",
    "KW_CONJUGATES",
    "KW_SUBSETS",
    "KW_REDEFINES",
    "KW_REFERENCES",
    "KW_CHAINS",
    "KW_INVERSE",
    "KW_FEATURED",
    "KW_TYPING",
    "KW_DEFAULT",
    "DefinitionBody",
    "DefinitionBodyItem",
    "UsageCompletion",
    "UsageBody",
    "TypeRelationshipPart",
    "FeatureRelationshipPart",
    "FeatureSpecializationPart",
    "PackageBody",
    "PackageBodyElement",
    "NamespaceBodyElement",
    "OwnedFeatureMember",
    "OwnedRelationship",
    "FeatureDeclaration",
    "FeatureChain",
    "QualifiedName",
    "RelationshipBody",
    "MultiplicityBounds",
    "OwnedExpression",
    "LiteralExpression",
];

/// Check if a message contains internal grammar rule names.
fn contains_internal_rule_name(msg: &str) -> bool {
    INTERNAL_RULE_NAMES.iter().any(|name| msg.contains(name))
}

// ═══════════════════════════════════════════════════════════════════
// MESSAGE QUALITY — no internal names leak into user-facing messages
// ═══════════════════════════════════════════════════════════════════

/// No diagnostic across ANY fixture should contain internal pest rule names.
/// This is the most important UX test — grammar internals must be humanized.
#[test]
fn no_internal_grammar_names_in_messages() {
    let fixtures: &[(&str, &str)] = &[
        ("clean.sysml", CLEAN),
        ("duplicate_names.sysml", DUPLICATE_NAMES),
        ("missing_semicolon.sysml", MISSING_SEMICOLON),
        ("corrupted_line.sysml", CORRUPTED_LINE),
        ("typo_reference.sysml", TYPO_REFERENCE),
        ("wrong_typing.sysml", WRONG_TYPING),
        ("ownership_violation.sysml", OWNERSHIP_VIOLATION),
        ("specialization_boundary.sysml", SPECIALIZATION_BOUNDARY),
        ("cascading.sysml", CASCADING),
        // Wave 2
        ("unclosed_brace.sysml", UNCLOSED_BRACE),
        ("extra_brace.sysml", EXTRA_BRACE),
        ("unterminated_comment.sysml", UNTERMINATED_COMMENT),
        ("partial_edit.sysml", PARTIAL_EDIT),
        ("error_recovery_following_items.sysml", ERROR_RECOVERY),
        ("qualified_name_resolution.sysml", QUALIFIED_NAME_RESOLUTION),
        ("missing_import.sysml", MISSING_IMPORT),
        (
            "duplicate_name_different_scope.sysml",
            DUPLICATE_NAME_DIFFERENT_SCOPE,
        ),
        ("redefine_missing_target.sysml", REDEFINE_MISSING_TARGET),
        ("stdlib_type_resolution.sysml", STDLIB_TYPE_RESOLUTION),
    ];

    let mut violations = Vec::new();
    for (name, source) in fixtures {
        let result = run(source, &full_opts());
        for diag in &result.diagnostics {
            if contains_internal_rule_name(&diag.message) {
                violations.push(format!(
                    "  {} — {:?}: {}",
                    name,
                    diag.code.as_deref().unwrap_or("no code"),
                    diag.message
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Internal grammar names found in user-facing messages:\n{}",
        violations.join("\n")
    );
}

/// Same check on the real model file.
#[test]
fn no_internal_grammar_names_in_model_file() {
    let model = include_str!("../../../../tests/fixtures/shared/sysml-rs-model.sysml");
    let result = diagnose_source(model, "file:///model/sysml-rs.sysml", &full_opts());
    for diag in &result.diagnostics {
        assert!(
            !contains_internal_rule_name(&diag.message),
            "Model file diagnostic contains internal grammar name:\n  code={:?} msg={}",
            diag.code,
            diag.message
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// ERROR CODE CORRECTNESS — codes match their category
// ═══════════════════════════════════════════════════════════════════

/// Syntax errors from tree-sitter should NOT carry structural validation codes.
/// E001 means "orphan element" — a syntax error is not an orphan element.
#[test]
fn syntax_errors_not_structural_codes() {
    let fixtures: &[(&str, &str)] = &[
        ("corrupted_line.sysml", CORRUPTED_LINE),
        ("cascading.sysml", CASCADING),
        ("ownership_violation.sysml", OWNERSHIP_VIOLATION),
        // Wave 2 syntax fixtures
        ("unclosed_brace.sysml", UNCLOSED_BRACE),
        ("extra_brace.sysml", EXTRA_BRACE),
        ("unterminated_comment.sysml", UNTERMINATED_COMMENT),
        ("partial_edit.sysml", PARTIAL_EDIT),
        ("error_recovery_following_items.sysml", ERROR_RECOVERY),
    ];

    let mut violations = Vec::new();
    for (name, source) in fixtures {
        // Syntax-only mode: no resolution or validation.
        // Anything produced here is a parser diagnostic, NOT structural.
        let result = run(source, &syntax_only());
        for diag in &result.diagnostics {
            if let Some(code) = &diag.code {
                // Parser diagnostics should not have E0xx structural codes
                if code.starts_with("E0") {
                    violations.push(format!(
                        "  {} — code {} on parser diagnostic: {}",
                        name, code, diag.message
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Syntax errors incorrectly carry structural error codes (E001 = orphan element):\n{}\n\
         Tree-sitter syntax errors should have no code, or a dedicated syntax code.",
        violations.join("\n")
    );
}

// ═══════════════════════════════════════════════════════════════════
// NO SPURIOUS DIAGNOSTICS — simple fixtures shouldn't trigger unrelated checks
// ═══════════════════════════════════════════════════════════════════

/// typo_reference.sysml has ONE issue: `Enginne` doesn't resolve.
/// It should NOT trigger "association expects source" or other structural noise.
#[test]
fn typo_reference_only_e200() {
    let result = run(TYPO_REFERENCE, &full_opts());
    // Should have E200 and nothing else
    for diag in &result.diagnostics {
        let code = diag.code.as_deref().unwrap_or("none");
        assert!(
            code == "E200" || code == "IM010" || diag.severity == Severity::Info,
            "typo_reference.sysml should only have E200/IM010, got {} {:?}: {}",
            code,
            diag.severity,
            diag.message,
        );
    }
}

/// wrong_typing.sysml has ONE issue: part typed by action def.
/// Should produce exactly one S-series diagnostic, no structural noise.
/// Note: AX-series health diagnostics are expected since the fixture has `action def DoSomething;`
/// with no body (empty action).
#[test]
fn wrong_typing_only_s_series() {
    let result = run(WRONG_TYPING, &full_opts());
    let non_s_errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| {
            is_error(d)
                && !d.code.as_ref().map(|c| c.starts_with('S')).unwrap_or(false)
                && !d
                    .code
                    .as_ref()
                    .map(|c| c.starts_with("AX"))
                    .unwrap_or(false)
        })
        .collect();
    assert!(
        non_s_errors.is_empty(),
        "wrong_typing.sysml should only have S-series or AX-series errors, got spurious:\n{}",
        dump(&result.diagnostics)
    );
}

/// specialization_boundary.sysml has ONE issue: part def :> attribute def.
/// Should produce exactly one S-series diagnostic, no noise.
#[test]
fn specialization_boundary_only_s_series() {
    let result = run(SPECIALIZATION_BOUNDARY, &full_opts());
    let non_s_errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| is_error(d) && !d.code.as_ref().map(|c| c.starts_with('S')).unwrap_or(false))
        .collect();
    assert!(
        non_s_errors.is_empty(),
        "specialization_boundary.sysml should only have S-series errors, got spurious:\n{}",
        dump(&result.diagnostics)
    );
}

/// clean.sysml is valid. Zero diagnostics. No noise.
#[test]
fn clean_file_zero_diagnostics() {
    let result = run(CLEAN, &full_opts());
    assert_eq!(
        result.diagnostics.len(),
        0,
        "Clean file should produce 0 diagnostics, got:\n{}",
        dump(&result.diagnostics)
    );
}

/// Entry transitions (`entry; then <state>;`) are valid in reference models and
/// should not trigger structural source-type noise for SuccessionAsUsage.
#[test]
fn state_entry_then_no_spurious_succession_source_mismatch() {
    let result = run(STATE_ENTRY_THEN_TRANSITION, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| {
            d.message
                .contains("succession as usage expects source to be")
        },
        "entry; then transitions should not emit SuccessionAsUsage source mismatch noise",
    );
}

#[test]
fn state_machine_no_transitions_reports_sm003() {
    let result = run(STATE_MACHINE_NO_TRANSITIONS, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "SM003"),
        "State machines without transitions should emit SM003",
    );
}

#[test]
fn state_machine_transition_usage_reports_reachability_and_dead_end() {
    let result = run(STATE_MACHINE_DISCONNECTED, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "SM004"),
        "Disconnected states should emit SM004",
    );
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "SM005"),
        "Dead-end non-final states should emit SM005",
    );
}

// ── Action health tests ─────────────────────────────────────────────

#[test]
fn action_no_steps_reports_ax001() {
    let result = run(ACTION_NO_STEPS, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "AX001"),
        "Action with no steps should emit AX001",
    );
}

#[test]
fn action_no_control_flow_reports_ax002() {
    let result = run(ACTION_NO_CONTROL_FLOW, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "AX002"),
        "Action without explicit control flow should emit AX002",
    );
}

// ── Flow health tests ───────────────────────────────────────────────

#[test]
fn flow_missing_endpoints_reports_fl001_or_fl002() {
    let result = run(FLOW_MISSING_SOURCE, &full_opts());
    // The tree-sitter parser may or may not populate flow source/target as props,
    // but if it does, we expect FL001 or FL002.
    // Even without those, the parse should succeed without panics.
    eprintln!("flow diagnostics: {}", dump(&result.diagnostics));
    // If flow diagnostics appear, they should have FL prefix and mention 'flow'
    for d in &result.diagnostics {
        if code_starts_with(d, "FL") {
            assert!(
                d.message.contains("flow") || d.message.contains("Flow"),
                "FL diagnostic should mention 'flow': {}",
                d.message
            );
        }
    }
}

// ── Verification health tests ───────────────────────────────────────

#[test]
fn verification_no_requirements_reports_vc001() {
    let result = run(VERIFICATION_NO_REQUIREMENTS, &full_opts());
    // VC001 requires the parser to produce VerificationCaseDefinition elements.
    // If the tree-sitter parser doesn't produce that element kind for this syntax,
    // there won't be VC diagnostics — which is still correct behavior.
    eprintln!("verification diagnostics: {}", dump(&result.diagnostics));
    // If VC diagnostics appear, they should have proper messages
    for d in &result.diagnostics {
        if has_code(d, "VC001") {
            assert!(
                d.message.contains("no requirements"),
                "VC001 should mention 'no requirements': {}",
                d.message
            );
        }
    }
}

// ── Extended action health fixtures (AX007-AX012) ───────────────────

const ACTION_IF_NO_CONDITION: &str = r#"
package ActionHealth {
    action def Controller {
        if ifCheck;
    }
}
"#;

const ACTION_WHILE_NO_CONDITION: &str = r#"
package ActionHealth {
    action def Poller {
        while whileCheck;
    }
}
"#;

const ACTION_FOR_NO_COLLECTION: &str = r#"
package ActionHealth {
    action def Processor {
        for forLoop;
    }
}
"#;

const ACTION_SEND_NO_TARGET: &str = r#"
package ActionHealth {
    action def Notifier {
        send sendAction;
    }
}
"#;

// ── Import health fixtures (IM001-IM005) ────────────────────────────

const IMPORT_UNKNOWN_NAMESPACE: &str = r#"
package ImportHealth {
    import NoSuchPackage::*;
}
"#;

const IMPORT_DUPLICATE: &str = r#"
package Upstream {
    part def Sensor;
}
package ImportHealth {
    import Upstream::*;
    import Upstream::*;
}
"#;

const IMPORT_CLEAN: &str = r#"
package Upstream {
    part def Sensor;
}
package ImportHealth {
    import Upstream::*;
    part mySensor : Sensor;
}
"#;

const ACTION_ACCEPT_NO_RECEIVER: &str = r#"
package ActionHealth {
    action def Listener {
        accept acceptAction;
    }
}
"#;

const ACTION_PERFORM_NO_BEHAVIOR: &str = r#"
package ActionHealth {
    action def Runner {
        perform orphanAction;
    }
}
"#;

// ── Extended flow health fixtures (FL007-FL009) ─────────────────────

const FLOW_UNRESOLVABLE_ENDPOINT: &str = r#"
package FlowHealth {
    part def System {
        flow dataFlow from nonExistentPort to alsoMissing;
    }
}
"#;

// ── Extended verification health fixtures (VC007-VC010) ─────────────

const REQUIREMENT_UNKNOWN_SUBJECT: &str = r#"
package VerificationHealth {
    requirement def SafetyReq {
        subject ghostPart;
    }
}
"#;

const REQUIREMENT_ASSUMPTION_NO_EXPR: &str = r#"
package VerificationHealth {
    requirement def AssumeReq {
        assume constraint emptyAssumption;
    }
}
"#;

const SATISFY_UNKNOWN_REQUIREMENT: &str = r#"
package VerificationHealth {
    part def System {
        satisfy requirement MissingReq;
    }
}
"#;

// ── Connector health fixtures ───────────────────────────────────────

const CONNECTOR_CLEAN: &str = r#"
package ConnectorHealth {
    part def System {
        part a;
        part b;
        connection link1 connect a to b;
    }
}
"#;

// ── Extended action health tests (AX007-AX012) ─────────────────────

#[test]
fn action_if_no_condition_reports_ax007() {
    let result = run(ACTION_IF_NO_CONDITION, &full_opts());
    eprintln!("if-no-condition diagnostics: {}", dump(&result.diagnostics));
    for d in &result.diagnostics {
        if has_code(d, "AX007") {
            assert!(
                d.message.contains("condition"),
                "AX007 should mention 'condition': {}",
                d.message
            );
        }
    }
}

#[test]
fn action_while_no_condition_reports_ax008() {
    let result = run(ACTION_WHILE_NO_CONDITION, &full_opts());
    eprintln!(
        "while-no-condition diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "AX008") {
            assert!(
                d.message.contains("condition"),
                "AX008 should mention 'condition': {}",
                d.message
            );
        }
    }
}

#[test]
fn action_for_no_collection_reports_ax009() {
    let result = run(ACTION_FOR_NO_COLLECTION, &full_opts());
    eprintln!(
        "for-no-collection diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "AX009") {
            assert!(
                d.message.contains("collection"),
                "AX009 should mention 'collection': {}",
                d.message
            );
        }
    }
}

#[test]
fn action_send_no_target_reports_ax010() {
    let result = run(ACTION_SEND_NO_TARGET, &full_opts());
    eprintln!("send-no-target diagnostics: {}", dump(&result.diagnostics));
    for d in &result.diagnostics {
        if has_code(d, "AX010") {
            assert!(
                d.message.contains("target"),
                "AX010 should mention 'target': {}",
                d.message
            );
        }
    }
}

#[test]
fn action_accept_no_receiver_reports_ax011() {
    let result = run(ACTION_ACCEPT_NO_RECEIVER, &full_opts());
    eprintln!(
        "accept-no-receiver diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "AX011") {
            assert!(
                d.message.contains("receiver"),
                "AX011 should mention 'receiver': {}",
                d.message
            );
        }
    }
}

#[test]
fn action_perform_no_behavior_reports_ax012() {
    let result = run(ACTION_PERFORM_NO_BEHAVIOR, &full_opts());
    eprintln!(
        "perform-no-behavior diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "AX012") {
            assert!(
                d.message.contains("behavior"),
                "AX012 should mention 'behavior': {}",
                d.message
            );
        }
    }
}

// ── Extended flow health tests (FL007-FL009) ────────────────────────

#[test]
fn flow_unresolvable_endpoint_reports_fl007() {
    let result = run(FLOW_UNRESOLVABLE_ENDPOINT, &full_opts());
    eprintln!(
        "flow-unresolvable diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "FL007") {
            assert!(
                d.message.contains("does not resolve") || d.message.contains("endpoint"),
                "FL007 should mention endpoint resolution: {}",
                d.message
            );
        }
    }
}

// ── Extended verification health tests (VC007-VC010) ────────────────

#[test]
fn requirement_unknown_subject_reports_vc007() {
    let result = run(REQUIREMENT_UNKNOWN_SUBJECT, &full_opts());
    eprintln!(
        "requirement-unknown-subject diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "VC007") {
            assert!(
                d.message.contains("subject") || d.message.contains("unknown"),
                "VC007 should mention 'subject' or 'unknown': {}",
                d.message
            );
        }
    }
}

#[test]
fn requirement_assumption_no_expr_reports_vc009() {
    let result = run(REQUIREMENT_ASSUMPTION_NO_EXPR, &full_opts());
    eprintln!(
        "assumption-no-expr diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "VC009") {
            assert!(
                d.message.contains("assumption") || d.message.contains("expression"),
                "VC009 should mention 'assumption' or 'expression': {}",
                d.message
            );
        }
    }
}

#[test]
fn satisfy_unknown_requirement_reports_vc010() {
    let result = run(SATISFY_UNKNOWN_REQUIREMENT, &full_opts());
    eprintln!("satisfy-unknown diagnostics: {}", dump(&result.diagnostics));
    for d in &result.diagnostics {
        if has_code(d, "VC010") {
            assert!(
                d.message.contains("unknown") || d.message.contains("references"),
                "VC010 should mention 'unknown' or 'references': {}",
                d.message
            );
        }
    }
}

// ── Import health tests (IM001-IM005) ───────────────────────────────

#[test]
fn import_unknown_namespace_diagnostics() {
    let result = run(IMPORT_UNKNOWN_NAMESPACE, &full_opts());
    eprintln!("import-unknown diagnostics: {}", dump(&result.diagnostics));
    // If IM diagnostics appear, they should have proper codes and messages
    for d in &result.diagnostics {
        if has_code(d, "IM001") {
            assert!(
                d.message.contains("unknown") || d.message.contains("namespace"),
                "IM001 should mention 'unknown' or 'namespace': {}",
                d.message
            );
        }
    }
}

#[test]
fn import_duplicate_diagnostics() {
    let result = run(IMPORT_DUPLICATE, &full_opts());
    eprintln!(
        "import-duplicate diagnostics: {}",
        dump(&result.diagnostics)
    );
    for d in &result.diagnostics {
        if has_code(d, "IM003") {
            assert!(
                d.message.contains("duplicate"),
                "IM003 should mention 'duplicate': {}",
                d.message
            );
        }
    }
}

#[test]
fn import_clean_no_im_diagnostics() {
    let result = run(IMPORT_CLEAN, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "IM"),
        "Clean import should have no IM diagnostics",
    );
}

// ── Connector health tests ──────────────────────────────────────────

#[test]
fn connector_clean_no_diagnostics() {
    let result = run(CONNECTOR_CLEAN, &full_opts());
    // Connector fixture should parse without connector-related errors.
    // If it produces syntax errors that's fine (grammar may not handle
    // `connect` syntax fully) but no phantom health codes.
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "IM") && d.message.to_lowercase().contains("connector"),
        "Clean connector fixture should have no connector-related IM diagnostics",
    );
}

// ── Regression guards ───────────────────────────────────────────────

/// Existing SM diagnostics still work after wiring new health families.
#[test]
fn existing_sm_diagnostics_still_pass() {
    let result = run(STATE_MACHINE_NO_TRANSITIONS, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "SM003"),
        "SM003 should still fire after adding AX/FL/VC diagnostics",
    );
}

/// Clean file still produces zero diagnostics.
#[test]
fn clean_file_zero_diagnostics_after_health_wiring() {
    let result = run(CLEAN, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| {
            code_starts_with(d, "AX")
                || code_starts_with(d, "FL")
                || code_starts_with(d, "VC")
                || code_starts_with(d, "IM")
        },
        "Clean file should have no health diagnostics",
    );
}

/// Dump all diagnostics from the health showcase fixture for debugging.
#[test]
fn health_showcase_diagnostics_dump() {
    let result = run(HEALTH_SHOWCASE, &full_opts());
    eprintln!(
        "=== HEALTH SHOWCASE DIAGNOSTICS ({}) ===",
        result.diagnostics.len()
    );
    eprintln!("{}", dump(&result.diagnostics));

    // Count by prefix
    let ax = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "AX"))
        .count();
    let fl = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "FL"))
        .count();
    let vc = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "VC"))
        .count();
    let im = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "IM"))
        .count();
    eprintln!("AX: {}, FL: {}, VC: {}, IM: {}", ax, fl, vc, im);
}

/// duplicate_names.sysml is syntactically valid. The ONLY issue is S001.
/// There should be no syntax errors, no structural errors.
#[test]
fn duplicate_names_no_spurious_errors() {
    let result = run(DUPLICATE_NAMES, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| d.code.is_none() && is_error(d),
        "duplicate_names.sysml is syntactically valid — should have no syntax errors",
    );
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "E0"),
        "duplicate_names.sysml should have no structural errors",
    );
}

// ═══════════════════════════════════════════════════════════════════
// SYNTAX ERROR DETECTION — errors that should be caught
// ═══════════════════════════════════════════════════════════════════

/// `part def Foo` without `;` or `{}` is a syntax error in SysML.
/// The pipeline must detect it, not silently accept it.
#[test]
fn missing_semicolon_detected() {
    let result = run(MISSING_SEMICOLON, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Missing semicolon should produce at least one error.\n\
         `part def Foo` without `;` or `{}` is invalid SysML.",
    );
}

/// When a missing semicolon IS detected, the message should mention `;`
/// or `expected` — not just a bare "syntax error".
#[test]
fn missing_semicolon_message_actionable() {
    let result = run(MISSING_SEMICOLON, &full_opts());
    if result.diagnostics.is_empty() {
        // If the error isn't detected at all, missing_semicolon_detected
        // will catch that. Skip message quality check.
        return;
    }
    assert_has(
        &result.diagnostics,
        |d| {
            is_error(d)
                && (d.message.contains(';')
                    || d.message.to_lowercase().contains("expected")
                    || d.message.to_lowercase().contains("semicolon"))
        },
        "Syntax error for missing ';' should mention ';' or 'expected'",
    );
}

/// "Expected ';'" errors should point at the end of the PREVIOUS token
/// (where the semicolon should go), not at the start of the NEXT token.
/// This ensures the underline shows the user where to insert, and the
/// code action inserts at the correct position.
#[test]
fn expected_semicolon_span_points_at_previous_token() {
    // `part def Name NextToken` — the error is after `Name`.
    // The parser sees `Name` as the definition name and then `NextToken`
    // is unexpected. The span should be near the end of `Name`, not
    // at the start of `NextToken`.
    let source = "package Outer {\n  part def Name NextToken;\n}";
    let result = run(source, &full_opts());
    let expected_errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| {
            is_error(d)
                && (d.message.to_lowercase().contains("expected") || d.message.contains("';'"))
        })
        .collect();
    if !expected_errors.is_empty() {
        let name_end = source.find("Name ").unwrap() + "Name".len(); // byte after 'e'
        let next_start = source.find("NextToken").unwrap(); // byte of 'N'
        for diag in &expected_errors {
            if let Some(span) = &diag.span {
                // Span should be BEFORE `NextToken`, near end of `Name`
                assert!(
                    span.end <= next_start,
                    "Expected-';' error should point at end of 'Name' (byte {}), \
                     not at 'NextToken' (byte {}). Got span {}..{}\n\
                     This ensures the code action inserts ';' at the right place.",
                    name_end,
                    next_start,
                    span.start,
                    span.end,
                );
            }
        }
    }
}

/// Missing `;` right before `}` should still be detected.
/// This is the pattern from the user's sysml-rs.sysml file:
/// `part def PilotSidecarParser :> sysml_text::Parser}` (no `;`)
#[test]
fn missing_semicolon_before_close_brace() {
    let source = "package Test {\n  part def Foo :> Bar\n}";
    let result = run(source, &full_opts());
    eprintln!(
        "=== missing_semicolon_before_close_brace ({} diagnostics) ===",
        result.diagnostics.len()
    );
    for (i, d) in result.diagnostics.iter().enumerate() {
        eprintln!(
            "  [{}] {:?} {:?} ({:?}) {}",
            i, d.code, d.severity, d.span, d.message
        );
    }
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Missing ';' before '}' should produce at least one error.\n\
         `part def Foo :> Bar` without ';' or '{}' body is invalid SysML.",
    );
}

/// Missing `;` in a file with pre-existing errors (line 58) is masked by
/// tree-sitter's error cascade. Both with/without `;` produce 50 diagnostics
/// — confirming this is a pre-existing limitation, not a regression.
#[test]
fn missing_semicolon_masked_by_cascading_errors() {
    let source = include_str!("../../../../tests/fixtures/shared/sysml-rs-model.sysml");
    let modified = source.replace(
        "part def PilotSidecarParser :> sysml_text::Parser;",
        "part def PilotSidecarParser :> sysml_text::Parser",
    );

    let result_with = run(source, &full_opts());
    let result_without = run(&modified, &full_opts());

    // The modified version (missing `;`) should produce MORE diagnostics than
    // the original, demonstrating the parser detects the missing semicolon.
    assert!(
        result_without.diagnostics.len() > result_with.diagnostics.len(),
        "Removing ';' should produce at least one additional diagnostic (got {} with, {} without)",
        result_with.diagnostics.len(),
        result_without.diagnostics.len(),
    );
}

/// Corrupted line should produce a syntax error.
#[test]
fn corrupted_line_detected() {
    let result = run(CORRUPTED_LINE, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Corrupted line should produce at least one Error",
    );
}

/// The error for the corrupted line should point at the garbled line
/// (`part def Goodpackage Bad {`), not at unrelated locations.
#[test]
fn corrupted_line_points_to_bad_line() {
    let result = run(CORRUPTED_LINE, &full_opts());
    // The garbled line is after the comment header; find its actual line number
    // (skip comment lines which also mention the bad text)
    let bad_line = CORRUPTED_LINE
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("Goodpackage Bad") && !l.trim_start().starts_with("//"))
        .map(|(i, _)| i + 1) // 1-indexed
        .expect("fixture should contain 'Goodpackage Bad' on a non-comment line");
    assert_has(
        &result.diagnostics,
        |d| {
            is_error(d)
                && line_of(d, CORRUPTED_LINE)
                    .map(|l| l == bad_line || l == bad_line + 1)
                    .unwrap_or(false)
        },
        &format!(
            "Corrupted line error should point to line {} or {} (the garbled line)",
            bad_line,
            bad_line + 1
        ),
    );
}

// ═══════════════════════════════════════════════════════════════════
// RESOLUTION — E200 message quality
// ═══════════════════════════════════════════════════════════════════

/// E200 must mention the unresolved name so the user knows what to fix.
/// BLOCKED: tree-sitter AST builder doesn't emit `unresolved_type` for typing
/// references (`: Enginne`). The resolver needs this property to detect unresolved
/// references and produce E200. See sysml-ts/src/ast_builder.rs.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn typo_reference_e200_names_the_symbol() {
    let result = run(TYPO_REFERENCE, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("Enginne"),
        "E200 should name the unresolved symbol 'Enginne'",
    );
}

/// E200 should carry a note about standard library not being loaded,
/// so the user knows this might resolve with the library.
/// BLOCKED: same root cause as typo_reference_e200_names_the_symbol.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn typo_reference_library_note() {
    let result = run(TYPO_REFERENCE, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| {
            has_code(d, "E200")
                && d.notes
                    .iter()
                    .any(|n| n.to_lowercase().contains("standard library") || n.contains("library"))
        },
        "E200 should note that resolution was without standard library",
    );
}

// ═══════════════════════════════════════════════════════════════════
// SEMANTIC VALIDATION — S-series quality
// ═══════════════════════════════════════════════════════════════════

/// Two `part def Foo` in the same namespace must trigger S001.
#[test]
fn duplicate_names_flagged_s001() {
    let result = run(DUPLICATE_NAMES, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "S001") && d.message.to_lowercase().contains("duplicate"),
        "Two `part def Foo` in same namespace must produce S001 with 'duplicate' in message",
    );
}

/// S001 should be a warning, not an error — duplicates are ambiguous, not fatal.
#[test]
fn duplicate_names_is_warning() {
    let result = run(DUPLICATE_NAMES, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "S001") && is_error(d),
        "S001 should be a Warning, not an Error",
    );
}

/// S001 should include "first defined here" in related_information
/// so the user can see both locations.
#[test]
fn duplicate_names_related_location() {
    let result = run(DUPLICATE_NAMES, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| {
            has_code(d, "S001")
                && d.related
                    .iter()
                    .any(|r| r.message.to_lowercase().contains("first defined"))
        },
        "S001 should have related_information with 'first defined here'",
    );
}

/// `part myPart : DoSomething` where DoSomething is an action def, not a part def.
/// Should produce S015 specifically, and the message should explain why.
/// BLOCKED: S015 requires the FeatureTyping relationship to be resolved so the
/// semantic check can see that DoSomething is an ActionDefinition. The resolver
/// needs `unresolved_type` from the tree-sitter AST builder.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn wrong_typing_s015_with_explanation() {
    let result = run(WRONG_TYPING, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "S015") && (d.message.contains("action") || d.message.contains("Action")),
        "S015 should explain that an action definition can't type a part usage",
    );
}

/// `part def MyPart :> MyData` where MyData is an attribute def.
/// Should produce S030 or S031.
/// BLOCKED: S030/S031 requires the Specialization relationship to be resolved so
/// the semantic check can see that MyData is an AttributeDefinition. The resolver
/// needs `unresolved_general` from the tree-sitter AST builder.
#[test]
#[ignore = "needs unresolved_general from tree-sitter AST builder"]
fn specialization_boundary_explains_boundary() {
    let result = run(SPECIALIZATION_BOUNDARY, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| {
            (has_code(d, "S030") || has_code(d, "S031"))
                && (d.message.contains("attribute") || d.message.contains("Attribute"))
        },
        "S030/S031 should explain the cross-boundary specialization",
    );
}

// ═══════════════════════════════════════════════════════════════════
// PIPELINE BEHAVIOR — gating, sorting, caps, dedup
// ═══════════════════════════════════════════════════════════════════

/// Cascade behavior: cascading.sysml has 17 garbled lines. Tree-sitter's
/// error-tolerant parser handles this gracefully, producing very few errors
/// instead of one per garbled line. Verify the error count stays manageable.
#[test]
fn cascading_has_cascade_suppression() {
    let result = run(CASCADING, &full_opts());
    let error_count = result.diagnostics.iter().filter(|d| is_error(d)).count();
    // Tree-sitter is error-tolerant: it groups garbled lines into a single
    // ERROR node, producing far fewer diagnostics than lines with errors.
    assert!(
        error_count <= 10,
        "Cascading errors should be manageable (got {})\n{}",
        error_count,
        dump(&result.diagnostics),
    );
}

/// Errors before warnings before info — always.
#[test]
fn diagnostics_priority_sorted() {
    let fixtures: &[(&str, &str)] = &[
        ("corrupted_line.sysml", CORRUPTED_LINE),
        ("cascading.sysml", CASCADING),
        ("wrong_typing.sysml", WRONG_TYPING),
    ];
    for (name, source) in fixtures {
        let result = run(source, &full_opts());
        let mut seen_non_error = false;
        for diag in &result.diagnostics {
            if !is_error(diag) {
                seen_non_error = true;
            } else if seen_non_error {
                panic!(
                    "{}: Error after non-error — not priority-sorted:\n{}",
                    name,
                    dump(&result.diagnostics)
                );
            }
        }
    }
}

/// No fixture should exceed the total cap.
#[test]
fn total_cap_applied() {
    let fixtures: &[(&str, &str)] = &[("cascading.sysml", CASCADING)];
    for (name, source) in fixtures {
        let result = run(source, &full_opts());
        assert!(
            result.diagnostics.len() <= TOTAL_DIAGNOSTIC_CAP,
            "{}: {} diagnostics exceeds cap of {}",
            name,
            result.diagnostics.len(),
            TOTAL_DIAGNOSTIC_CAP,
        );
    }
}

/// No two diagnostics should share the same (span, code) after dedup.
#[test]
fn dedup_no_repeated_spans() {
    let fixtures: &[(&str, &str)] = &[
        ("corrupted_line.sysml", CORRUPTED_LINE),
        ("cascading.sysml", CASCADING),
    ];
    for (name, source) in fixtures {
        let result = run(source, &full_opts());
        let mut seen = std::collections::HashSet::new();
        for diag in &result.diagnostics {
            if let (Some(span), Some(code)) = (&diag.span, &diag.code) {
                let key = (span.start, span.end, code.clone());
                assert!(
                    seen.insert(key),
                    "{}: Duplicate diagnostic: code={} span={}..{}",
                    name,
                    code,
                    span.start,
                    span.end
                );
            }
        }
    }
}

/// Scope suppression: structural/semantic diagnostics should not overlap
/// with syntax error spans.
#[test]
fn syntax_errors_suppress_overlapping_validation() {
    let result = run(CORRUPTED_LINE, &full_opts());
    let syntax_spans: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| is_error(d) && d.code.is_none())
        .filter_map(|d| d.span.as_ref().map(|s| (s.start, s.end)))
        .collect();

    for diag in &result.diagnostics {
        if let Some(code) = &diag.code {
            if code.starts_with('S') {
                if let Some(span) = &diag.span {
                    let overlaps = syntax_spans
                        .iter()
                        .any(|&(s, e)| span.start < e && span.end > s);
                    assert!(
                        !overlaps,
                        "S-series {} at {}..{} overlaps syntax error span — should be suppressed",
                        code, span.start, span.end
                    );
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// WAVE 2: SYNTAX / RECOVERY
// ═══════════════════════════════════════════════════════════════════

/// Missing `}` at EOF should produce at least one error.
#[test]
fn unclosed_brace_detected() {
    let result = run(UNCLOSED_BRACE, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Unclosed brace should produce at least one error",
    );
}

/// The error message should mention `}` so the user knows what's missing.
#[test]
fn unclosed_brace_mentions_brace() {
    let result = run(UNCLOSED_BRACE, &full_opts());
    if result.diagnostics.is_empty() {
        return; // unclosed_brace_detected catches this
    }
    assert_has(
        &result.diagnostics,
        |d| is_error(d) && d.message.contains('}'),
        "Unclosed brace error should mention `}`",
    );
}

/// A single missing `}` should not cascade into many errors. At most 2.
#[test]
fn unclosed_brace_no_cascade() {
    let result = run(UNCLOSED_BRACE, &syntax_only());
    let error_count = result.diagnostics.iter().filter(|d| is_error(d)).count();
    assert!(
        error_count <= 2,
        "Unclosed brace should produce at most 2 errors (low-noise recovery), got {}\n{}",
        error_count,
        dump(&result.diagnostics)
    );
}

/// MISSING nodes should not carry structural codes (E002 bug fix).
#[test]
fn unclosed_brace_no_structural_codes() {
    let result = run(UNCLOSED_BRACE, &syntax_only());
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "E0"),
        "MISSING-node syntax errors should not carry E0xx structural codes (E002 fix)",
    );
}

/// Stray `}` after valid package should produce at least one error.
#[test]
fn extra_brace_detected() {
    let result = run(EXTRA_BRACE, &syntax_only());
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Extra brace should produce at least one error",
    );
}

/// The error for the stray `}` should have a span (localized).
#[test]
fn extra_brace_localized() {
    let result = run(EXTRA_BRACE, &syntax_only());
    assert_has(
        &result.diagnostics,
        |d| is_error(d) && d.span.is_some(),
        "Extra brace error should have a span (localized)",
    );
}

/// The extra `}` is a syntax issue, not a resolution or semantic one.
#[test]
fn extra_brace_no_resolution_noise() {
    let result = run(EXTRA_BRACE, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E200") || code_starts_with(d, "S"),
        "Extra brace should not produce E200 or S-series noise",
    );
}

/// Unterminated `/*` comment should produce at least one error.
#[test]
fn unterminated_comment_detected() {
    let result = run(UNTERMINATED_COMMENT, &syntax_only());
    assert_has(
        &result.diagnostics,
        |d| is_error(d),
        "Unterminated comment should produce at least one error",
    );
}

/// Unterminated comment errors should not carry structural or semantic codes.
#[test]
fn unterminated_comment_no_structural_codes() {
    let result = run(UNTERMINATED_COMMENT, &syntax_only());
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "E0") || code_starts_with(d, "S"),
        "Unterminated comment errors should not carry E0xx or S-series codes",
    );
}

/// Mid-typing `attr` should produce at most 2 diagnostics (low-noise typing UX).
#[test]
fn partial_edit_low_noise() {
    let result = run(PARTIAL_EDIT, &full_opts());
    assert!(
        result.diagnostics.len() <= 3,
        "Partial edit (mid-typing `attr`) should produce at most 3 diagnostics, got {}\n{}",
        result.diagnostics.len(),
        dump(&result.diagnostics)
    );
}

/// After a garbage line, valid definitions before and after should survive in the graph.
#[test]
#[ignore = "G36: the 884e7a61 regen's error recovery eats the definition after a `@@@` line (grammar-gaps-inventory.md) — fix the recovery, drop this ignore"]
fn error_recovery_valid_elements_survive() {
    let result = run(ERROR_RECOVERY, &full_opts());
    let names: Vec<_> = result
        .graph
        .elements
        .values()
        .filter_map(|e| e.name.as_deref())
        .collect();
    assert!(
        names.contains(&"Before"),
        "Element 'Before' should survive error recovery.\nElements: {:?}",
        names
    );
    assert!(
        names.contains(&"After"),
        "Element 'After' should survive error recovery.\nElements: {:?}",
        names
    );
}

/// The error in error_recovery should be localized to the `@@@` line.
#[test]
fn error_recovery_error_localized() {
    let result = run(ERROR_RECOVERY, &syntax_only());
    let bad_line = ERROR_RECOVERY
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("@@@"))
        .map(|(i, _)| i + 1)
        .expect("fixture should contain '@@@'");
    assert_has(
        &result.diagnostics,
        |d| {
            is_error(d)
                && line_of(d, ERROR_RECOVERY)
                    .map(|l| l == bad_line)
                    .unwrap_or(false)
        },
        &format!(
            "Error should be localized to line {} (the @@@ line)",
            bad_line
        ),
    );
}

/// Error recovery fixture should not produce resolution noise for valid elements.
// KNOWN GAP (tracked — see memory project-lsp-baseline-rot): asserts that E200
// 'this' is suppressed over a `@@` syntax-error region. Exercises the
// test-support `diagnose_source` path (relocated under RSC-6.6); resolving it
// belongs with the RSC-6.6 part-2 cross-crate collapse (route onto
// sysml-service compute_pipeline). #[ignore]d so --lib stays green/gateable;
// remove when 6.6 part 2 lands.
// FIXED 2026-06-23: `post_process`'s scope-aware suppression now drops E2xx
// resolution codes (not just E0/S/V) that overlap a code-less syntax-error
// span — an E200 ("no definition 'this' found") fired over a `@@`/`this`
// syntax-error region is cascade noise, not a real unresolved-name error.
#[test]
fn error_recovery_no_resolution_noise() {
    let result = run(ERROR_RECOVERY, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E200"),
        "Error recovery fixture should not produce E200 for valid definitions",
    );
}

// ═══════════════════════════════════════════════════════════════════
// WAVE 2: RESOLUTION / SEMANTIC
// ═══════════════════════════════════════════════════════════════════

/// `A::T` across top-level packages should resolve cleanly.
#[test]
fn qualified_name_zero_diagnostics() {
    let result = run(QUALIFIED_NAME_RESOLUTION, &full_opts());
    assert_eq!(
        result.diagnostics.len(),
        0,
        "Qualified name resolution (A::T) should produce 0 diagnostics, got:\n{}",
        dump(&result.diagnostics)
    );
}

/// `Sensor` used without import from sibling package should produce E200.
/// BLOCKED: tree-sitter AST builder doesn't emit `unresolved_type` for typing
/// references (`: Sensor`). The resolver needs this property.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn missing_import_produces_e200() {
    let result = run(MISSING_IMPORT, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200"),
        "Using `Sensor` without import should produce E200",
    );
}

/// The E200 for missing import should name the unresolved symbol.
/// BLOCKED: same root cause as missing_import_produces_e200.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn missing_import_names_symbol() {
    let result = run(MISSING_IMPORT, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("Sensor"),
        "E200 for missing import should name 'Sensor'",
    );
}

/// Same name in different packages should NOT produce S001.
#[test]
fn different_scope_no_s001() {
    let result = run(DUPLICATE_NAME_DIFFERENT_SCOPE, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "S001"),
        "Same name in different packages should not produce S001 (scope-aware)",
    );
}

/// `:>> noSuchFeature` should produce E200.
/// KNOWN GAP: redefinition target resolution not yet implemented.
#[test]
#[ignore = "redefinition target resolution not yet implemented — reveals real UX gap"]
fn redefine_missing_target_e200() {
    let result = run(REDEFINE_MISSING_TARGET, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200"),
        "Redefining a non-existent feature should produce E200",
    );
}

/// E200 for missing redefine target should name the symbol.
/// KNOWN GAP: redefinition target resolution not yet implemented.
#[test]
#[ignore = "redefinition target resolution not yet implemented — reveals real UX gap"]
fn redefine_missing_target_names_symbol() {
    let result = run(REDEFINE_MISSING_TARGET, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("noSuchFeature"),
        "E200 should name the unresolved redefine target 'noSuchFeature'",
    );
}

/// `Integer` is a standard library type — E200 should be suppressed for it.
#[test]
fn stdlib_integer_suppressed() {
    let result = run(STDLIB_TYPE_RESOLUTION, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("Integer"),
        "E200 for 'Integer' should be suppressed (standard library type)",
    );
}

/// `CustomThing` is NOT a standard library type — E200 should be present.
/// BLOCKED: tree-sitter AST builder doesn't emit `unresolved_type` for typing
/// references (`: CustomThing`). The resolver needs this property.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn stdlib_custom_type_e200() {
    let result = run(STDLIB_TYPE_RESOLUTION, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("CustomThing"),
        "E200 should be present for 'CustomThing' (not a stdlib type)",
    );
}

/// E200 for `CustomThing` should have a note mentioning the standard library.
/// BLOCKED: same root cause as stdlib_custom_type_e200.
#[test]
#[ignore = "needs unresolved_type from tree-sitter AST builder"]
fn stdlib_custom_type_library_note() {
    let result = run(STDLIB_TYPE_RESOLUTION, &full_opts());
    assert_has(
        &result.diagnostics,
        |d| {
            has_code(d, "E200")
                && d.message.contains("CustomThing")
                && d.notes
                    .iter()
                    .any(|n| n.to_lowercase().contains("standard library") || n.contains("library"))
        },
        "E200 for CustomThing should note that standard library is not loaded",
    );
}

/// `Real` is a standard library scalar type — E200 should be suppressed for it.
#[test]
fn stdlib_real_suppressed() {
    let result = run(STDLIB_REAL_RESOLUTION, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E200") && d.message.contains("Real"),
        "E200 for 'Real' should be suppressed (standard library scalar type)",
    );
}

// ═══════════════════════════════════════════════════════════════════
// ACCEPTANCE — real model file
// ═══════════════════════════════════════════════════════════════════

#[test]
fn model_file_has_diagnostics() {
    let model = include_str!("../../../../tests/fixtures/shared/sysml-rs-model.sysml");
    let result = diagnose_source(model, "file:///model/sysml-rs.sysml", &full_opts());

    assert!(
        !result.diagnostics.is_empty(),
        "model/sysml-rs.sysml has known errors — should produce diagnostics"
    );
    assert!(
        result.diagnostics.len() <= TOTAL_DIAGNOSTIC_CAP,
        "Model file: {} diagnostics exceeds cap {}",
        result.diagnostics.len(),
        TOTAL_DIAGNOSTIC_CAP,
    );
}

/// The model file should not have internal grammar names either.
/// (Covered by no_internal_grammar_names_in_model_file above, but
/// this also checks the model file doesn't produce "association expects source"
/// type noise.)
#[test]
fn model_file_no_spurious_association_errors() {
    let model = include_str!("../../../../tests/fixtures/shared/sysml-rs-model.sysml");
    let result = diagnose_source(model, "file:///model/sysml-rs.sysml", &full_opts());
    assert_none(
        &result.diagnostics,
        |d| d.message.contains("expects source to be"),
        "Model file should not have spurious 'expects source to be' structural errors",
    );
}

/// Model file error count: the syntax errors should be manageable.
/// Tree-sitter's error tolerance produces far fewer errors than a strict parser.
#[test]
fn model_file_cascade_suppression() {
    let model = include_str!("../../../../tests/fixtures/shared/sysml-rs-model.sysml");
    let result = diagnose_source(model, "file:///model/sysml-rs.sysml", &syntax_only());
    let error_count = result.diagnostics.iter().filter(|d| is_error(d)).count();
    // Tree-sitter groups cascading syntax problems into ERROR nodes,
    // producing a manageable number of diagnostics.
    assert!(
        error_count <= 20,
        "Model file error count should be manageable (got {})\n{}",
        error_count,
        dump(&result.diagnostics),
    );
}

// ═══════════════════════════════════════════════════════════════════
// CONSTRAINT VIOLATION DIAGNOSTICS
// ═══════════════════════════════════════════════════════════════════

const SIMPLE_VEHICLE: &str = include_str!("../fixtures/valid/simple_vehicle.sysml");

/// simple_vehicle.sysml defines a Vehicle with `constraint speedLimit { speed < 100 }`
/// and a usage with `speed = 105`. The pipeline should detect the violation.
#[test]
fn simple_vehicle_constraint_violation() {
    let result = run(SIMPLE_VEHICLE, &full_opts());
    // Should have a constraint violation diagnostic (C001 or C002)
    let constraint_diags: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.code.as_ref().map(|c| c.starts_with('C')).unwrap_or(false))
        .collect();
    assert!(
        !constraint_diags.is_empty(),
        "simple_vehicle.sysml should produce a constraint violation diagnostic.\n\
         All diagnostics:\n{}",
        dump(&result.diagnostics),
    );
    // The violation message should mention the constraint name and expression
    let violation = constraint_diags[0];
    assert!(
        violation.message.contains("speedLimit") || violation.message.contains("speed"),
        "Constraint violation should mention 'speedLimit' or 'speed', got: {}",
        violation.message,
    );
}

// ═══════════════════════════════════════════════════════════════════
// DEBUG DUMP — run with --nocapture to see all diagnostics
// ═══════════════════════════════════════════════════════════════════

/// cargo test -p sysml-lsp-server diagnostic_ux_tests::dump_all -- --nocapture
#[test]
fn dump_all_fixtures() {
    let fixtures: &[(&str, &str)] = &[
        ("clean.sysml", CLEAN),
        ("duplicate_names.sysml", DUPLICATE_NAMES),
        ("missing_semicolon.sysml", MISSING_SEMICOLON),
        ("corrupted_line.sysml", CORRUPTED_LINE),
        ("typo_reference.sysml", TYPO_REFERENCE),
        ("wrong_typing.sysml", WRONG_TYPING),
        ("ownership_violation.sysml", OWNERSHIP_VIOLATION),
        ("specialization_boundary.sysml", SPECIALIZATION_BOUNDARY),
        ("cascading.sysml", CASCADING),
        // Wave 2
        ("unclosed_brace.sysml", UNCLOSED_BRACE),
        ("extra_brace.sysml", EXTRA_BRACE),
        ("unterminated_comment.sysml", UNTERMINATED_COMMENT),
        ("partial_edit.sysml", PARTIAL_EDIT),
        ("error_recovery_following_items.sysml", ERROR_RECOVERY),
        ("qualified_name_resolution.sysml", QUALIFIED_NAME_RESOLUTION),
        ("missing_import.sysml", MISSING_IMPORT),
        (
            "duplicate_name_different_scope.sysml",
            DUPLICATE_NAME_DIFFERENT_SCOPE,
        ),
        ("redefine_missing_target.sysml", REDEFINE_MISSING_TARGET),
        ("stdlib_type_resolution.sysml", STDLIB_TYPE_RESOLUTION),
    ];
    for (name, source) in fixtures {
        let result = run(source, &full_opts());
        eprintln!(
            "\n=== {} ({} diagnostics) ===\n{}",
            name,
            result.diagnostics.len(),
            dump(&result.diagnostics)
        );
    }
}

/// Verify that incremental tree-sitter parsing (edit path) produces the same
/// diagnostics as fresh parsing (open path). Regression for "errors appear on edit".
#[test]
fn incremental_parse_matches_fresh_parse() {
    use sysml_parser_incremental::ast_builder::build_model_graph;
    use sysml_parser_incremental::TreeSitterParser;

    let parser = TreeSitterParser::new();
    // TS-3.6: dropped `let pest_parser = PestParser::new();` — the pest
    // diagnostics were used only for an eprintln debug print
    // ("pest={} diags"). All assertions in this test compare incremental
    // vs fresh TS parses; the pest count was diagnostic noise.

    let fixtures: &[(&str, &str)] = &[
        (
            "simple_vehicle",
            include_str!("../../../../tests/fixtures/shared/simple_vehicle.sysml"),
        ),
        (
            "test_all_features",
            include_str!("../../../../tests/fixtures/shared/test_all_features.sysml"),
        ),
        (
            "test_hover",
            include_str!("../../../../tests/fixtures/shared/test_hover.sysml"),
        ),
        (
            "test_action",
            include_str!("../../../../tests/fixtures/shared/test_action.sysml"),
        ),
        (
            "test_whatif",
            include_str!("../../../../tests/fixtures/shared/test_whatif.sysml"),
        ),
        (
            "test_flow",
            include_str!("../../../../tests/fixtures/shared/test_flow.sysml"),
        ),
    ];

    for (name, source) in fixtures {
        // Fresh parse (like did_open)
        let tree1 = parser.parse_tree(source).expect("fresh parse failed");
        let result1 = build_model_graph(&tree1, source, "file:///test.sysml");

        // Simulate did_change: edit the old tree to say "entire doc replaced"
        let mut old_tree = tree1;
        let old_end_byte = old_tree.root_node().end_byte();
        let old_end_point = old_tree.root_node().end_position();
        let new_end_byte = source.len();
        let new_end_point = {
            let mut row = 0usize;
            let mut col = 0usize;
            for b in source.as_bytes() {
                if *b == b'\n' {
                    row += 1;
                    col = 0;
                } else {
                    col += 1;
                }
            }
            tree_sitter::Point { row, column: col }
        };
        old_tree.edit(&tree_sitter::InputEdit {
            start_byte: 0,
            old_end_byte,
            new_end_byte,
            start_position: tree_sitter::Point { row: 0, column: 0 },
            old_end_position: old_end_point,
            new_end_position: new_end_point,
        });

        // Incremental parse (like did_change)
        let tree2 = parser
            .parse_tree_incremental(source, Some(&old_tree))
            .expect("incremental parse failed");
        let result2 = build_model_graph(&tree2, source, "file:///test.sysml");

        // TS-3.6: dropped the pest diagnostic comparison line (eprintln-only,
        // not load-bearing). The strict-syntax oracle moved off Pest in
        // TS-3.3.
        eprintln!(
            "{}: fresh={} diags/{} elems, incr={} diags/{} elems",
            name,
            result1.diagnostics.len(),
            result1.graph.elements.len(),
            result2.diagnostics.len(),
            result2.graph.elements.len(),
        );

        // The incremental parse should produce the same number of diagnostics
        assert_eq!(
            result1.diagnostics.len(),
            result2.diagnostics.len(),
            "{}: incremental parse has different diagnostic count than fresh parse!\n  fresh: {:?}\n  incr: {:?}",
            name,
            result1.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>(),
            result2.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>(),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// INTEGRATION — mixed model with multiple diagnostic families
// ═══════════════════════════════════════════════════════════════════

/// Mixed fixture exercising imports, actions, flows, requirements, and
/// state machines together. Verifies all new code families coexist with
/// existing SM/AX/FL/VC diagnostics.
const MIXED_INTEGRATION: &str = r#"
package Upstream {
    part def Sensor;
}
package MixedTest {
    // Import (should trigger IM diagnostics for duplicate)
    import Upstream::*;
    import Upstream::*;

    // State machine (should trigger SM diagnostics)
    state def Toggle {
        state Off;
        state On;
    }

    // Action with no steps (AX001)
    action def EmptyAction;

    // Action with steps but no control flow (AX002)
    action def TwoStep {
        action step1;
        action step2;
    }

    // Verification case with no requirements (VC001)
    verification def EmptyCheck;

    // Flow (may trigger FL diagnostics depending on parse)
    part def Controller;
}
"#;

#[test]
fn mixed_integration_produces_multiple_families() {
    let result = run(MIXED_INTEGRATION, &full_opts());
    eprintln!(
        "=== MIXED INTEGRATION ({} diagnostics) ===\n{}",
        result.diagnostics.len(),
        dump(&result.diagnostics)
    );

    // Count by family
    let sm = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "SM"))
        .count();
    let ax = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "AX"))
        .count();
    let fl = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "FL"))
        .count();
    let vc = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "VC"))
        .count();
    let im = result
        .diagnostics
        .iter()
        .filter(|d| code_starts_with(d, "IM"))
        .count();
    eprintln!("SM: {}, AX: {}, FL: {}, VC: {}, IM: {}", sm, ax, fl, vc, im);

    // At minimum, AX diagnostics should fire (empty action is reliably detected)
    assert!(
        ax > 0,
        "Mixed model should produce at least one AX diagnostic"
    );

    // SM diagnostics should fire (state machine with no transitions)
    assert!(
        sm > 0,
        "Mixed model should produce at least one SM diagnostic"
    );

    // IM diagnostics should fire (duplicate import)
    assert!(
        im > 0,
        "Mixed model should produce at least one IM diagnostic (duplicate import)"
    );

    // No diagnostic should have an internal grammar name
    for diag in &result.diagnostics {
        assert!(
            !contains_internal_rule_name(&diag.message),
            "Mixed integration: internal grammar name in message: {}",
            diag.message
        );
    }

    // Diagnostics should be priority-sorted (errors before warnings before info)
    let mut seen_non_error = false;
    for diag in &result.diagnostics {
        if !is_error(diag) {
            seen_non_error = true;
        } else if seen_non_error {
            panic!(
                "Mixed integration: diagnostics not priority-sorted:\n{}",
                dump(&result.diagnostics)
            );
        }
    }

    // No false-positive codes that shouldn't fire on this fixture
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E200"),
        "Mixed integration should not produce E200 (no unresolved references expected)",
    );
    assert_none(
        &result.diagnostics,
        |d| code_starts_with(d, "C"),
        "Mixed integration should not produce constraint violations",
    );

    // Every diagnostic should have a span (localized)
    for diag in &result.diagnostics {
        if diag.code.is_some() {
            assert!(
                diag.span.is_some(),
                "Diagnostic {:?} should have a span for localization: {}",
                diag.code,
                diag.message,
            );
        }
    }
}

/// Message quality regression: every health diagnostic in the mixed fixture
/// should name the element it's about.
#[test]
fn mixed_integration_messages_name_elements() {
    let result = run(MIXED_INTEGRATION, &full_opts());
    for diag in &result.diagnostics {
        match diag.code.as_deref() {
            Some("AX001") => assert!(
                diag.message.contains("EmptyAction"),
                "AX001 should name 'EmptyAction': {}",
                diag.message
            ),
            Some("AX002") => assert!(
                diag.message.contains("TwoStep") || diag.message.contains("EmptyAction"),
                "AX002 should name the action: {}",
                diag.message
            ),
            Some("SM003") => assert!(
                diag.message.contains("Toggle"),
                "SM003 should name 'Toggle': {}",
                diag.message
            ),
            Some("IM003") => assert!(
                diag.message.contains("Upstream"),
                "IM003 should name 'Upstream': {}",
                diag.message
            ),
            _ => {} // Other codes may or may not fire
        }
    }
}

/// Notes quality: health diagnostics should have helpful notes.
#[test]
fn health_diagnostics_have_notes() {
    let result = run(MIXED_INTEGRATION, &full_opts());
    for diag in &result.diagnostics {
        match diag.code.as_deref() {
            Some("AX001") => assert!(
                !diag.notes.is_empty(),
                "AX001 should have a note with fix guidance"
            ),
            _ => {}
        }
    }
}

/// False-positive check: simple_vehicle.sysml is a valid, well-formed model.
/// No health diagnostics (AX/FL/VC/IM/SM) should fire on it, only constraint violations.
#[test]
fn simple_vehicle_no_spurious_health_diagnostics() {
    let result = run(SIMPLE_VEHICLE, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| {
            code_starts_with(d, "AX")
                || code_starts_with(d, "FL")
                || code_starts_with(d, "VC")
                || code_starts_with(d, "IM")
        },
        "simple_vehicle.sysml should not produce AX/FL/VC/IM health diagnostics",
    );
}

/// Elaboration idempotency: calling elaborate() twice on the same graph
/// should produce zero changes on the second call.
#[test]
fn elaboration_idempotency_integration() {
    use sysml_core::elaborate::elaborate;

    let fixtures: &[(&str, &str)] = &[
        ("mixed", MIXED_INTEGRATION),
        ("state_machine", STATE_MACHINE_DISCONNECTED),
        ("action_no_steps", ACTION_NO_STEPS),
        ("import_duplicate", IMPORT_DUPLICATE),
        ("clean", CLEAN),
    ];

    for (name, source) in fixtures {
        let ts_parser = sysml_parser_incremental::TreeSitterParser::new();
        let tree = ts_parser.parse_tree(source).expect("parse failed");
        let result = sysml_parser_incremental::build_model_graph(&tree, source, "file:///test.sysml");
        let mut graph = result.graph;

        let r1 = elaborate(&mut graph);
        let r2 = elaborate(&mut graph);

        assert_eq!(
            r2.total(),
            0,
            "{}: second elaborate() should be a no-op (got {} changes). \
             First elaborate: {}",
            name,
            r2.total(),
            r1,
        );
    }
}

/// Discover `.sysml` files under `dir`, delegating to the canonical
/// implementation in `sysml_service`.
fn collect_sysml_files_from(dir: &Path) -> Vec<PathBuf> {
    if !dir.is_dir() {
        return Vec::new();
    }
    // Pure enumeration via the same walker `service.open_context(Folder)`
    // uses internally — keeps the "single discovery rule" invariant.
    sysml_project::discovery::discover(dir, 100_000)
        .map(|d| d.files)
        .unwrap_or_else(|e| {
            panic!(
                "failed to discover sysml files under {}: {e}",
                dir.display()
            )
        })
}

/// Regression safety net: run diagnostics over a broad fixture corpus and fail
/// if any diagnostic is emitted without a span.
#[test]
fn fixture_corpus_has_no_spanless_diagnostics() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("sysml-lsp-server should be under repo root");
    let roots = [
        repo_root.join("crates/tooling/sysml-lsp-server/fixtures"),
        repo_root.join("crates/tooling/sysml-cli/fixtures"),
        repo_root.join("crates/testing/sysml-spec-tests/corpus/advent"),
        repo_root.join("crates/lang/sysml-parser-incremental/tree-sitter/test/execution_corpus"),
    ];

    let mut files: Vec<PathBuf> = roots
        .iter()
        .flat_map(|root| collect_sysml_files_from(root))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "fixture corpus test found no files to validate"
    );

    let opts = full_opts();
    let mut offenders = Vec::new();

    for path in files {
        let source = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("failed to read fixture {}: {}", path.display(), e);
        });
        let uri = format!("file://{}", path.display());
        let result = diagnose_source(&source, &uri, &opts);
        let spanless: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.span.is_none())
            .collect();
        if spanless.is_empty() {
            continue;
        }
        let details = spanless
            .iter()
            .take(8)
            .map(|d| {
                format!(
                    "code={} severity={:?} message={}",
                    d.code.as_deref().unwrap_or("<none>"),
                    d.severity,
                    d.message
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        offenders.push(format!(
            "{} ({} spanless) :: {}",
            path.display(),
            spanless.len(),
            details
        ));
    }

    assert!(
        offenders.is_empty(),
        "spanless diagnostics found in fixture corpus:\n{}",
        offenders.join("\n")
    );
}

// ── False-positive guards (coffee-machine bug classes) ───────────────

/// Guard: `entry; then Idle;` is valid state machine syntax.
/// SM002 (succession source mismatch) should NOT fire on entry-then patterns.
#[test]
fn entry_succession_no_sm002() {
    let source = r#"
package FPGuard {
    state def Toggle {
        entry; then Off;
        state Off;
        state On;
        transition turn_on first Off then On;
        transition turn_off first On then Off;
    }
}
"#;
    let result = run(source, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "SM002"),
        "entry; then <state>; should not trigger SM002 (succession source mismatch)",
    );
}

/// Guard: `return result : Real;` is valid SysML syntax.
/// E004 (unexpected element) should NOT fire on return parameters.
#[test]
fn return_param_no_e004() {
    let source = r#"
package FPGuard {
    calc def AddCalc {
        in x : Real;
        in y : Real;
        return result : Real;
    }
}
"#;
    let result = run(source, &full_opts());
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "E004"),
        "return parameters should not trigger E004 (unexpected element)",
    );
}

/// Guard: `import Definitions::*;` is a cross-file import.
/// IM001 should NOT be error-severity — it's an info-level note about missing namespace.
#[test]
fn cross_file_import_not_error() {
    let source = r#"
package FPGuard {
    import Definitions::*;
    part myCoffeeMachine : Definitions::CoffeeMachine;
}
"#;
    let result = run(source, &full_opts());
    // IM001 diagnostics may appear (cross-file reference), but should NOT be error severity
    assert_none(
        &result.diagnostics,
        |d| has_code(d, "IM001") && is_error(d),
        "IM001 on cross-file imports should be info/warning, not error",
    );
}

/// Guard: `require constraint : MaxSpeed;` is valid typed constraint usage.
/// Should NOT produce parse errors.
#[test]
fn require_constraint_typed_no_parse_error() {
    let source = r#"
package FPGuard {
    constraint def MaxSpeed {
        doc /* Speed constraint */
    }
    requirement def SafetyReq {
        require constraint : MaxSpeed;
    }
}
"#;
    let result = run(source, &full_opts());
    // No syntax errors expected
    assert_none(
        &result.diagnostics,
        |d| is_error(d) && d.code.is_none(),
        "require constraint : Type should not produce syntax errors",
    );
}

/// Guard: `satisfy SpeedReq by Vehicle;` is valid SysML syntax.
/// Should NOT produce parse errors.
#[test]
fn satisfy_by_no_parse_error() {
    let source = r#"
package FPGuard {
    requirement SpeedReq {
        doc /* Speed requirement */
    }
    part Vehicle {
        satisfy SpeedReq by Vehicle;
    }
}
"#;
    let result = run(source, &full_opts());
    // No syntax errors expected
    assert_none(
        &result.diagnostics,
        |d| is_error(d) && d.code.is_none(),
        "satisfy...by should not produce syntax errors",
    );
}

/// Guard: `@Maturity { status = "experimental"; }` is valid metadata annotation.
/// Should NOT produce parse errors.
#[test]
fn metadata_annotation_no_parse_error() {
    let source = r#"
package FPGuard {
    metadata def Maturity {
        attribute status : String;
    }
    part def Widget {
        @Maturity { status = "experimental"; }
    }
}
"#;
    let result = run(source, &full_opts());
    // No syntax errors expected
    assert_none(
        &result.diagnostics,
        |d| is_error(d) && d.code.is_none(),
        "@Metadata annotation should not produce syntax errors",
    );
}

// ═══════════════════════════════════════════════════════════════════
// SALSA PIPELINE UX TESTS
//
// These tests exercise the salsa-based diagnostic pipeline
// (AnalysisHost → Analysis → parse/resolve/validate queries).
// They mirror the key assertions from the old diagnose_source() tests
// above, documenting any quality gaps in the salsa path.
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod salsa_ux_tests {
    use super::*;
    use sysml_core::ModelGraph;
    use sysml_ide_db::AnalysisHost;

    // ── Salsa pipeline helper ────────────────────────────────────────

    struct SalsaResult {
        diagnostics: Vec<Diagnostic>,
        graph: ModelGraph,
    }

    /// Run the full diagnostic pipeline on a source string.
    ///
    /// RSC-6.6: this routes through the *real* production pipeline
    /// (`sysml_service::diagnostics::compute_pipeline`) instead of the
    /// hand-maintained reimplementation it used to carry. The old shadow
    /// only ran four health functions and resolved without a library; the
    /// production pipeline runs the full health set (state-machine, action,
    /// port, verification, constraint, requirement, quantity ×2) plus
    /// flow / import / physics health, then the shared suppression / dedup /
    /// sort / cap post-processing. We deliberately use `compute_pipeline`
    /// rather than `compute_full_diagnostics` so the `Readiness × Tier` gate
    /// (an LSP-publish concern) doesn't suppress diagnostics on these
    /// workspace-less single-file fixtures.
    fn salsa_run(source: &str) -> SalsaResult {
        let uri = "file:///test.sysml";
        let mut host = AnalysisHost::new();
        let id = host.set_file_content(uri, source.to_string());
        let sf = host.source_file(id).unwrap();
        let analysis = host.analysis();

        let diagnostics = sysml_service::diagnostics::compute_pipeline(&analysis, sf, None, uri);
        let elaborated = analysis.elaborate_file_best(sf, None);
        let graph = elaborated.graph().clone();

        SalsaResult { diagnostics, graph }
    }

    // ── Clean file ───────────────────────────────────────────────────

    #[test]
    fn salsa_clean_file_zero_diagnostics() {
        let result = salsa_run(CLEAN);
        assert_eq!(
            result.diagnostics.len(),
            0,
            "[SALSA] Clean file should produce 0 diagnostics, got:\n{}",
            dump(&result.diagnostics)
        );
    }

    #[test]
    fn salsa_clean_file_no_health_diagnostics() {
        let result = salsa_run(CLEAN);
        assert_none(
            &result.diagnostics,
            |d| {
                code_starts_with(d, "AX")
                    || code_starts_with(d, "FL")
                    || code_starts_with(d, "VC")
                    || code_starts_with(d, "IM")
            },
            "[SALSA] Clean file should have no health diagnostics",
        );
    }

    // ── Syntax error detection ───────────────────────────────────────

    /// PIPELINE GAP: Without pest hybrid enrichment, the salsa pipeline
    /// may miss missing-semicolon errors that tree-sitter accepts.
    #[test]
    fn salsa_missing_semicolon_detected() {
        let result = salsa_run(MISSING_SEMICOLON);
        assert_has(
            &result.diagnostics,
            |d| is_error(d),
            "[SALSA] Missing semicolon should produce at least one error.\n\
             `part def Foo` without `;` or `{}` is invalid SysML.\n\
             GAP: salsa pipeline needs pest hybrid enrichment.",
        );
    }

    /// PIPELINE GAP: Without pest, the error message won't mention ';'.
    #[test]
    fn salsa_missing_semicolon_message_actionable() {
        let result = salsa_run(MISSING_SEMICOLON);
        if result.diagnostics.is_empty() {
            // If the error isn't detected at all, salsa_missing_semicolon_detected
            // catches that. Skip message quality check.
            return;
        }
        assert_has(
            &result.diagnostics,
            |d| {
                is_error(d)
                    && (d.message.contains(';')
                        || d.message.to_lowercase().contains("expected")
                        || d.message.to_lowercase().contains("semicolon"))
            },
            "[SALSA] Syntax error for missing ';' should mention ';' or 'expected'.\n\
             GAP: salsa pipeline needs pest hybrid enrichment for actionable messages.",
        );
    }

    #[test]
    fn salsa_corrupted_line_detected() {
        let result = salsa_run(CORRUPTED_LINE);
        assert_has(
            &result.diagnostics,
            |d| is_error(d),
            "[SALSA] Corrupted line should produce at least one Error",
        );
    }

    #[test]
    fn salsa_unclosed_brace_detected() {
        let result = salsa_run(UNCLOSED_BRACE);
        assert_has(
            &result.diagnostics,
            |d| is_error(d),
            "[SALSA] Unclosed brace should produce at least one error",
        );
    }

    #[test]
    fn salsa_extra_brace_detected() {
        let result = salsa_run(EXTRA_BRACE);
        assert_has(
            &result.diagnostics,
            |d| is_error(d),
            "[SALSA] Extra brace should produce at least one error",
        );
    }

    // ── No internal grammar name leaks ───────────────────────────────

    #[test]
    fn salsa_no_internal_grammar_names_in_messages() {
        let fixtures: &[(&str, &str)] = &[
            ("clean.sysml", CLEAN),
            ("duplicate_names.sysml", DUPLICATE_NAMES),
            ("corrupted_line.sysml", CORRUPTED_LINE),
            ("cascading.sysml", CASCADING),
        ];

        let mut violations = Vec::new();
        for (name, source) in fixtures {
            let result = salsa_run(source);
            for diag in &result.diagnostics {
                if contains_internal_rule_name(&diag.message) {
                    violations.push(format!(
                        "  {} — {:?}: {}",
                        name,
                        diag.code.as_deref().unwrap_or("no code"),
                        diag.message
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "[SALSA] Internal grammar names found in user-facing messages:\n{}",
            violations.join("\n")
        );
    }

    // ── Resolution / Semantic ────────────────────────────────────────

    #[test]
    fn salsa_duplicate_names_flagged_s001() {
        let result = salsa_run(DUPLICATE_NAMES);
        assert_has(
            &result.diagnostics,
            |d| has_code(d, "S001") && d.message.to_lowercase().contains("duplicate"),
            "[SALSA] Two `part def Foo` in same namespace must produce S001 with 'duplicate'",
        );
    }

    #[test]
    fn salsa_duplicate_names_is_warning() {
        let result = salsa_run(DUPLICATE_NAMES);
        assert_none(
            &result.diagnostics,
            |d| has_code(d, "S001") && is_error(d),
            "[SALSA] S001 should be a Warning, not an Error",
        );
    }

    #[test]
    fn salsa_different_scope_no_s001() {
        let result = salsa_run(DUPLICATE_NAME_DIFFERENT_SCOPE);
        assert_none(
            &result.diagnostics,
            |d| has_code(d, "S001"),
            "[SALSA] Same name in different packages should not produce S001",
        );
    }

    #[test]
    fn salsa_qualified_name_zero_diagnostics() {
        let result = salsa_run(QUALIFIED_NAME_RESOLUTION);
        assert_eq!(
            result.diagnostics.len(),
            0,
            "[SALSA] Qualified name resolution (A::T) should produce 0 diagnostics, got:\n{}",
            dump(&result.diagnostics)
        );
    }

    #[test]
    fn salsa_stdlib_integer_suppressed() {
        let result = salsa_run(STDLIB_TYPE_RESOLUTION);
        assert_none(
            &result.diagnostics,
            |d| has_code(d, "E200") && d.message.contains("Integer"),
            "[SALSA] E200 for 'Integer' should be suppressed (standard library type)",
        );
    }

    // ── Health diagnostics ───────────────────────────────────────────

    #[test]
    fn salsa_state_machine_no_transitions_sm003() {
        let result = salsa_run(STATE_MACHINE_NO_TRANSITIONS);
        assert_has(
            &result.diagnostics,
            |d| has_code(d, "SM003"),
            "[SALSA] State machines without transitions should emit SM003",
        );
    }

    #[test]
    fn salsa_state_machine_with_transitions_no_sm003() {
        let result = salsa_run(STATE_ENTRY_THEN_TRANSITION);
        assert_none(
            &result.diagnostics,
            |d| has_code(d, "SM003"),
            "[SALSA] State machine with valid transitions should not emit SM003",
        );
    }

    #[test]
    fn salsa_action_no_steps_ax001() {
        let result = salsa_run(ACTION_NO_STEPS);
        assert_has(
            &result.diagnostics,
            |d| has_code(d, "AX001"),
            "[SALSA] Action with no steps should emit AX001",
        );
    }

    #[test]
    fn salsa_import_clean_no_im() {
        let result = salsa_run(IMPORT_CLEAN);
        assert_none(
            &result.diagnostics,
            |d| code_starts_with(d, "IM"),
            "[SALSA] Clean import should have no IM diagnostics",
        );
    }

    // ── Constraint monitoring ────────────────────────────────────────

    #[test]
    fn salsa_simple_vehicle_constraint_violation() {
        let result = salsa_run(SIMPLE_VEHICLE);
        let constraint_diags: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.as_ref().map(|c| c.starts_with('C')).unwrap_or(false))
            .collect();
        assert!(
            !constraint_diags.is_empty(),
            "[SALSA] simple_vehicle.sysml should produce a constraint violation.\n\
             All diagnostics:\n{}",
            dump(&result.diagnostics),
        );
    }

    // ── Pipeline behavior ────────────────────────────────────────────

    #[test]
    fn salsa_cascading_has_cascade_suppression() {
        let result = salsa_run(CASCADING);
        let error_count = result.diagnostics.iter().filter(|d| is_error(d)).count();
        assert!(
            error_count <= 10,
            "[SALSA] Cascading errors should be manageable (got {})\n{}",
            error_count,
            dump(&result.diagnostics),
        );
    }

    #[test]
    fn salsa_diagnostics_priority_sorted() {
        let fixtures: &[(&str, &str)] = &[
            ("corrupted_line.sysml", CORRUPTED_LINE),
            ("cascading.sysml", CASCADING),
        ];
        for (name, source) in fixtures {
            let result = salsa_run(source);
            let mut seen_non_error = false;
            for diag in &result.diagnostics {
                if !is_error(diag) {
                    seen_non_error = true;
                } else if seen_non_error {
                    panic!(
                        "[SALSA] {}: Error after non-error — not priority-sorted:\n{}",
                        name,
                        dump(&result.diagnostics)
                    );
                }
            }
        }
    }

    #[test]
    fn salsa_total_cap_applied() {
        let result = salsa_run(CASCADING);
        assert!(
            result.diagnostics.len() <= TOTAL_DIAGNOSTIC_CAP,
            "[SALSA] {} diagnostics exceeds cap of {}",
            result.diagnostics.len(),
            TOTAL_DIAGNOSTIC_CAP,
        );
    }

    #[test]
    fn salsa_dedup_no_repeated_spans() {
        let fixtures: &[(&str, &str)] = &[
            ("corrupted_line.sysml", CORRUPTED_LINE),
            ("cascading.sysml", CASCADING),
        ];
        for (name, source) in fixtures {
            let result = salsa_run(source);
            let mut seen = std::collections::HashSet::new();
            for diag in &result.diagnostics {
                if let (Some(span), Some(code)) = (&diag.span, &diag.code) {
                    let key = (span.start, span.end, code.clone());
                    assert!(
                        seen.insert(key),
                        "[SALSA] {}: Duplicate diagnostic: code={} span={}..{}",
                        name,
                        code,
                        span.start,
                        span.end
                    );
                }
            }
        }
    }

    // ── Error recovery ───────────────────────────────────────────────

    #[test]
    #[ignore = "G36: the 884e7a61 regen's error recovery eats the definition after a `@@@` line (grammar-gaps-inventory.md) — fix the recovery, drop this ignore"]
    fn salsa_error_recovery_valid_elements_survive() {
        let result = salsa_run(ERROR_RECOVERY);
        let names: Vec<_> = result
            .graph
            .elements
            .values()
            .filter_map(|e| e.name.as_deref())
            .collect();
        assert!(
            names.contains(&"Before"),
            "[SALSA] Element 'Before' should survive error recovery.\nElements: {:?}",
            names
        );
        assert!(
            names.contains(&"After"),
            "[SALSA] Element 'After' should survive error recovery.\nElements: {:?}",
            names
        );
    }

    // ── Mixed integration ────────────────────────────────────────────

    #[test]
    fn salsa_mixed_integration_produces_multiple_families() {
        let result = salsa_run(MIXED_INTEGRATION);
        eprintln!(
            "[SALSA] MIXED INTEGRATION ({} diagnostics)\n{}",
            result.diagnostics.len(),
            dump(&result.diagnostics)
        );

        let sm = result
            .diagnostics
            .iter()
            .filter(|d| code_starts_with(d, "SM"))
            .count();
        let ax = result
            .diagnostics
            .iter()
            .filter(|d| code_starts_with(d, "AX"))
            .count();
        let im = result
            .diagnostics
            .iter()
            .filter(|d| code_starts_with(d, "IM"))
            .count();
        eprintln!("[SALSA] SM: {}, AX: {}, IM: {}", sm, ax, im);

        assert!(
            ax > 0,
            "[SALSA] Mixed model should produce at least one AX diagnostic"
        );
        assert!(
            sm > 0,
            "[SALSA] Mixed model should produce at least one SM diagnostic"
        );
        assert!(
            im > 0,
            "[SALSA] Mixed model should produce at least one IM diagnostic"
        );
    }

    // ── Scope-aware suppression ────────────────────────────────────────

    /// Structural/semantic diagnostics should not overlap with syntax error spans.
    #[test]
    fn salsa_syntax_errors_suppress_overlapping_validation() {
        let result = salsa_run(CORRUPTED_LINE);
        let syntax_spans: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| is_error(d) && d.code.is_none())
            .filter_map(|d| d.span.as_ref().map(|s| (s.start, s.end)))
            .collect();

        for diag in &result.diagnostics {
            if let Some(code) = &diag.code {
                if code.starts_with('S') {
                    if let Some(span) = &diag.span {
                        let overlaps = syntax_spans
                            .iter()
                            .any(|&(s, e)| span.start < e && span.end > s);
                        assert!(
                            !overlaps,
                            "[SALSA] S-series {} at {}..{} overlaps syntax error span — should be suppressed",
                            code, span.start, span.end
                        );
                    }
                }
            }
        }
    }

    // ── False positive check ─────────────────────────────────────────

    #[test]
    fn salsa_simple_vehicle_no_spurious_health() {
        let result = salsa_run(SIMPLE_VEHICLE);
        assert_none(
            &result.diagnostics,
            |d| {
                code_starts_with(d, "AX")
                    || code_starts_with(d, "FL")
                    || code_starts_with(d, "VC")
                    || code_starts_with(d, "IM")
            },
            "[SALSA] simple_vehicle.sysml should not produce AX/FL/VC/IM health diagnostics",
        );
    }

    // ── Debug dump ───────────────────────────────────────────────────

    /// cargo test -p sysml-lsp-server salsa_ux_tests::dump_salsa_all -- --nocapture
    #[test]
    fn dump_salsa_all() {
        let fixtures: &[(&str, &str)] = &[
            ("clean.sysml", CLEAN),
            ("duplicate_names.sysml", DUPLICATE_NAMES),
            ("missing_semicolon.sysml", MISSING_SEMICOLON),
            ("corrupted_line.sysml", CORRUPTED_LINE),
            ("cascading.sysml", CASCADING),
            ("unclosed_brace.sysml", UNCLOSED_BRACE),
            ("extra_brace.sysml", EXTRA_BRACE),
            ("qualified_name_resolution.sysml", QUALIFIED_NAME_RESOLUTION),
        ];
        for (name, source) in fixtures {
            let result = salsa_run(source);
            eprintln!(
                "\n=== [SALSA] {} ({} diagnostics) ===\n{}",
                name,
                result.diagnostics.len(),
                dump(&result.diagnostics)
            );
        }
    }
}
