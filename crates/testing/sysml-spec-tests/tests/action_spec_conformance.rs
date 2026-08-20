//! ASC — Action control-node spec-conformance harness.
//!
//! Sibling to `constraint_spec_conformance.rs` (constraints/expressions) and
//! `runtime_spec_conformance.rs` (transfers/triggers/ports). This file covers
//! the **action control-node** semantic area: fork, join, decision, and merge.
//! Same convention: every test encodes ONE spec-defined obligation and asserts
//! the engine's CURRENT behavior against it, carrying a verdict marker on its
//! own line:
//!
//! - `// VERDICT: CONFORMS` — current behavior satisfies the obligation.
//! - `// VERDICT: DIVERGES — <reason>` — the test asserts what the engine
//!   ACTUALLY does today, which differs from the spec obligation. It fails only
//!   if behavior silently changes; flip the assertion when a fix-wave closes it.
//! - `// VERDICT: UNIMPLEMENTED — <missing>` — the obligation has no engine
//!   surface; the test pins the absence.
//!
//! Each test names the obligation it gates with an `// OBL:` line whose id
//! matches the obligation tracker at
//! `crates/testing/sysml-spec-tests/spec-obligations/actions.md`. That tracker
//! is the authority for spec citations; this file is the gate.
//!
//! Spec sources (cited per obligation in the tracker):
//! - SysML §7.17.3 / §8.4.13.4 "Control Nodes" (`SysML-spec-r2025-04_REF.html`)
//! - KerML `Kernel Semantic Library/ControlPerformances.kerml`
//!   (`DecisionPerformance`, `MergePerformance`, fork/join semantics)
//! - `sysml.library/Systems Library/Actions.sysml`
//!
//! ── GAP-ACT-COMPILE (why these tests build `ActionGraphIR` directly) ────────
//! These obligations are **behavioral** (token flow through control nodes), so
//! they MUST be gated at the runtime layer, NOT through `.sysml` source. The
//! source→IR lowering path (`sysml_runtime::actions::compile_action`) currently
//! DROPS the two pieces of model state these tests need:
//!   1. **succession guards** — guard expressions on successions are not lowered
//!      onto `ActionEdgeIR.guard`, so a source-level decision node has no guards
//!      to evaluate (every outgoing edge looks unguarded → "default" branch);
//!   2. **assignment RHS** — `Assign` nodes are lowered with a placeholder
//!      `LiteralInt(0)` instead of the real value expression.
//! Until that lowering gap (GAP-ACT-COMPILE) is closed, control-node semantics
//! can only be observed by constructing `ActionGraphIR` directly. These tests
//! therefore build the IR with the same node/edge idioms as the runtime's own
//! unit tests (`actions/mod.rs`), drive it with `ActionRunner`, and assert on
//! `final_bindings()` / `ActionStepResult::diagnostics`. NO LSP, NO service, NO
//! production code changes — this file measures.
//!
//! The summary test (`asc_matrix_summary`) self-scans this file via
//! `include_str!` and prints the CONFORMS / DIVERGES / UNIMPLEMENTED counts.

use std::collections::HashMap;
use sysml_core::Value;
use sysml_runtime::actions::{ActionGraphIR, ActionNodeIR, ActionRunner, ActionStepResult};
use sysml_runtime::expressions::{BinOp, EvalContext, ExprIR};

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Drive a runner to completion (bounded), collecting every step result.
/// Mirrors the `run_to_completion` helper in `actions/mod.rs` tests.
fn run_to_completion(runner: &mut ActionRunner, ctx: &EvalContext) -> Vec<ActionStepResult> {
    let mut results = Vec::new();
    for _ in 0..200 {
        let r = runner.step(ctx);
        let done = r.completed;
        results.push(r);
        if done {
            break;
        }
    }
    results
}

/// Snapshot of a runner's `final_bindings` as an owned map for easy asserts.
fn final_bindings(runner: &ActionRunner) -> HashMap<String, Value> {
    runner.final_bindings().clone()
}

/// `lhs <op> rhs` over a feature reference and an int literal — used to build
/// guard expressions for decision edges.
fn guard_cmp(feature: &str, op: BinOp, rhs: i64) -> ExprIR {
    ExprIR::BinaryOp {
        op,
        left: Box::new(ExprIR::FeatureRef(feature.into())),
        right: Box::new(ExprIR::LiteralInt(rhs)),
    }
}

// ===========================================================================
// OBL-FORK — fork-node-concurrent-fanout
// "A fork orders itself before ALL outgoing targets (every branch activates)."
// Spec: SysML §7.17.3 / §8.4.13.4. A fork's every outgoing succession must
// carry a value ⇒ every branch runs.
// ===========================================================================

#[test]
fn fork_activates_every_outgoing_branch() {
    // OBL: fork-node-concurrent-fanout
    // VERDICT: CONFORMS
    // fork -> (x=1, y=2) -> join -> final. Both branch variables must appear in
    // the final bindings, proving BOTH branches activated (not just one).
    let mut graph = ActionGraphIR::new("fork_fanout", "ForkFanout");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let fork = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
    let bx = graph.add_node(ActionNodeIR::Assign {
        id: "ax".into(),
        target: "x".into(),
        value: ExprIR::LiteralInt(1),
    });
    let by = graph.add_node(ActionNodeIR::Assign {
        id: "ay".into(),
        target: "y".into(),
        value: ExprIR::LiteralInt(2),
    });
    let join = graph.add_node(ActionNodeIR::Join { id: "join1".into() });

    graph.add_edge(&initial, &fork);
    graph.add_edge(&fork, &bx);
    graph.add_edge(&fork, &by);
    graph.add_edge(&bx, &join);
    graph.add_edge(&by, &join);
    graph.add_edge(&join, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    let fb = final_bindings(&runner);
    assert_eq!(
        fb.get("x"),
        Some(&Value::Int(1)),
        "fork branch x must have activated"
    );
    assert_eq!(
        fb.get("y"),
        Some(&Value::Int(2)),
        "fork branch y must have activated"
    );
}

// ===========================================================================
// OBL-JOIN — join-node-synchronize-all-incoming
// "A join is ordered after ALL incoming sources complete." A join must NOT
// fire (forward its token) until every incoming branch has arrived.
// Spec: SysML §7.17.3 / §8.4.13.4.
// ===========================================================================

/// Build a staggered-fork control graph whose join/merge is selected by
/// `control`: a fast branch (`x`) reaches the control node in one hop, while a
/// slow branch (`y`) takes two hops. Returns the assembled graph; the control
/// node has id `"ctrl1"` and forwards to the single final node.
fn staggered_control_graph(name: &str, control: ActionNodeIR) -> ActionGraphIR {
    let mut graph = ActionGraphIR::new(name, name);
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let fork = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
    // Fast branch: single assign straight to the control node.
    let ax = graph.add_node(ActionNodeIR::Assign {
        id: "ax".into(),
        target: "x".into(),
        value: ExprIR::LiteralInt(1),
    });
    // Slow branch: TWO assigns in series before reaching the control node.
    let ay1 = graph.add_node(ActionNodeIR::Assign {
        id: "ay1".into(),
        target: "y1".into(),
        value: ExprIR::LiteralInt(2),
    });
    let ay2 = graph.add_node(ActionNodeIR::Assign {
        id: "ay2".into(),
        target: "y".into(),
        value: ExprIR::LiteralInt(3),
    });
    let ctrl = graph.add_node(control);

    graph.add_edge(&initial, &fork);
    graph.add_edge(&fork, &ax);
    graph.add_edge(&fork, &ay1);
    graph.add_edge(&ay1, &ay2);
    graph.add_edge(&ax, &ctrl);
    graph.add_edge(&ay2, &ctrl);
    graph.add_edge(&ctrl, &final_id);
    graph
}

#[test]
fn join_waits_for_all_incoming_before_firing() {
    // OBL: join-node-synchronize-all-incoming
    // VERDICT: CONFORMS
    // Staggered branches: the slow `y` branch reaches the join one step after
    // the fast `x` branch. Because the join holds the fast token until the slow
    // one arrives, NOTHING reaches the final node early — so on every step
    // before the join fires, the runner is NOT completed AND no binding has
    // landed in final_bindings. Only after both arrive do BOTH writes appear
    // together (synchronized).
    let graph = staggered_control_graph("JoinSync", ActionNodeIR::Join { id: "ctrl1".into() });
    let mut runner = ActionRunner::new(graph);
    let ctx = EvalContext::new();

    // Invariant across the run: final_bindings is empty until the moment the
    // join fires and the merged token reaches final. The fast `x` write must
    // NOT appear on its own (that would mean the join passed the first arrival).
    let mut x_appeared_before_completion = false;
    for _ in 0..200 {
        let r = runner.step(&ctx);
        let fb = final_bindings(&runner);
        if !runner.is_completed() && fb.contains_key("x") {
            x_appeared_before_completion = true;
        }
        if r.completed {
            break;
        }
    }
    assert!(runner.is_completed());
    assert!(
        !x_appeared_before_completion,
        "join must hold the fast branch's token — no write may reach final before the join fires"
    );

    // Once it fires, the post-join token carries every branch's writes together.
    let fb = final_bindings(&runner);
    assert_eq!(fb.get("x"), Some(&Value::Int(1)));
    assert_eq!(fb.get("y"), Some(&Value::Int(3)));
}

#[test]
fn join_does_not_complete_until_all_arrive() {
    // OBL: join-node-synchronize-all-incoming
    // VERDICT: CONFORMS
    // The slow branch needs strictly more steps to reach the join than the fast
    // branch. We assert completion happens no earlier than the step on which the
    // slow branch could possibly have arrived — i.e. the join genuinely waited
    // rather than firing on the first token. (Concretely: the fast branch needs
    // ~3 steps to reach+fire; the slow branch adds one hop, so a synchronizing
    // join cannot complete before the slow branch's extra hop is taken.)
    let graph = staggered_control_graph("JoinWait", ActionNodeIR::Join { id: "ctrl1".into() });
    let mut runner = ActionRunner::new(graph);
    let ctx = EvalContext::new();

    let mut completed_at = None;
    for step in 1..=200 {
        let r = runner.step(&ctx);
        if r.completed {
            completed_at = Some(step);
            break;
        }
    }
    let completed_at = completed_at.expect("join graph must complete");
    // initial->fork (1), fork->branches (2), fast ax->join + slow ay1->ay2 (3),
    // slow ay2->join + join fires->final (4), final processed/completed (5+).
    // A join that fired on the FIRST arrival would complete by step 4; requiring
    // both arrivals pushes completion strictly later.
    assert!(
        completed_at >= 5,
        "join completed too early ({completed_at} steps) — it did not wait for the slow branch"
    );
}

// ===========================================================================
// OBL-DECISION — decision-node-exactly-one-outgoing
// "A decision routes to exactly one outgoing branch per performance." Exactly
// one outgoing succession with a true guard is taken.
// Spec: SysML §7.17.3 / §8.4.13.4; ControlPerformances.kerml DecisionPerformance.
// ===========================================================================

#[test]
fn decision_routes_to_exactly_one_true_guard() {
    // OBL: decision-node-exactly-one-outgoing
    // VERDICT: CONFORMS
    // decision with two guarded branches (sel > 0 ⇒ set hi; sel < 0 ⇒ set lo).
    // With sel = 5 only the `hi` branch fires; `lo` is never written.
    let mut graph = ActionGraphIR::new("decision_one", "DecisionOne");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    // Seed `sel` first so the guards have something to read.
    let seed = graph.add_node(ActionNodeIR::Assign {
        id: "seed".into(),
        target: "sel".into(),
        value: ExprIR::LiteralInt(5),
    });
    let decision = graph.add_node(ActionNodeIR::Decision { id: "dec1".into() });
    let hi = graph.add_node(ActionNodeIR::Assign {
        id: "hi".into(),
        target: "hi".into(),
        value: ExprIR::LiteralInt(100),
    });
    let lo = graph.add_node(ActionNodeIR::Assign {
        id: "lo".into(),
        target: "lo".into(),
        value: ExprIR::LiteralInt(-100),
    });

    graph.add_edge(&initial, &seed);
    graph.add_edge(&seed, &decision);
    graph.add_guarded_edge(&decision, &hi, guard_cmp("sel", BinOp::GreaterThan, 0));
    graph.add_guarded_edge(&decision, &lo, guard_cmp("sel", BinOp::LessThan, 0));
    graph.add_edge(&hi, &final_id);
    graph.add_edge(&lo, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    let fb = final_bindings(&runner);
    assert_eq!(
        fb.get("hi"),
        Some(&Value::Int(100)),
        "the true-guard branch must fire"
    );
    assert!(
        !fb.contains_key("lo"),
        "the false-guard branch must NOT fire (exactly one outgoing)"
    );
}

#[test]
fn decision_no_matching_guard_diagnoses() {
    // OBL: decision-node-exactly-one-outgoing
    // VERDICT: CONFORMS
    // When no guard is satisfied and there is no unguarded default edge, the
    // decision emits a "no matching guard" diagnostic and routes nowhere. This
    // is the honest failure mode (no silent fall-through to an arbitrary
    // branch) consistent with the exactly-one obligation.
    let mut graph = ActionGraphIR::new("decision_nomatch", "DecisionNoMatch");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let seed = graph.add_node(ActionNodeIR::Assign {
        id: "seed".into(),
        target: "sel".into(),
        value: ExprIR::LiteralInt(0),
    });
    let decision = graph.add_node(ActionNodeIR::Decision { id: "dec1".into() });
    let hi = graph.add_node(ActionNodeIR::Assign {
        id: "hi".into(),
        target: "hi".into(),
        value: ExprIR::LiteralInt(100),
    });
    let lo = graph.add_node(ActionNodeIR::Assign {
        id: "lo".into(),
        target: "lo".into(),
        value: ExprIR::LiteralInt(-100),
    });

    graph.add_edge(&initial, &seed);
    graph.add_edge(&seed, &decision);
    // Both guards are false for sel == 0, and there is NO unguarded edge.
    graph.add_guarded_edge(&decision, &hi, guard_cmp("sel", BinOp::GreaterThan, 0));
    graph.add_guarded_edge(&decision, &lo, guard_cmp("sel", BinOp::LessThan, 0));
    graph.add_edge(&hi, &final_id);
    graph.add_edge(&lo, &final_id);

    let mut runner = ActionRunner::new(graph);
    let results = run_to_completion(&mut runner, &EvalContext::new());

    let all_diags: Vec<String> = results
        .iter()
        .flat_map(|r| r.diagnostics.iter().map(|d| d.message.clone()))
        .collect();
    assert!(
        all_diags.iter().any(|m| m.contains("no matching guard")),
        "a decision with no satisfiable guard and no default must diagnose; got {all_diags:?}"
    );

    // Neither branch fired.
    let fb = final_bindings(&runner);
    assert!(!fb.contains_key("hi"));
    assert!(!fb.contains_key("lo"));
}

// ===========================================================================
// OBL-MERGE — merge-node-any-one-incoming
// "A merge fires once per exactly-one incoming control" — a merge passes each
// arriving token straight through WITHOUT synchronizing (contrast with join).
// Spec: SysML §7.17.3 / §8.4.13.4; ControlPerformances.kerml MergePerformance.
// ===========================================================================

#[test]
fn merge_passes_each_arrival_through_without_sync() {
    // OBL: merge-node-any-one-incoming
    // VERDICT: CONFORMS
    // Identical staggered topology to the join tests, but with a Merge control
    // node. A merge does NOT synchronize: each arriving token is forwarded
    // immediately. So the fast `x` token passes straight through the merge to
    // the final node and lands in final_bindings BEFORE the slow `y` branch has
    // finished — the direct behavioral contrast with `join_waits_*` above, where
    // nothing could reach final before both branches arrived.
    let graph = staggered_control_graph("MergeAny", ActionNodeIR::Merge { id: "ctrl1".into() });
    let mut runner = ActionRunner::new(graph);
    let ctx = EvalContext::new();

    let mut x_reached_final_before_y = false;
    for _ in 0..200 {
        let r = runner.step(&ctx);
        let fb = final_bindings(&runner);
        // The fast branch's write reaching final while the slow branch's `y`
        // is still absent proves the merge passed the first arrival straight
        // through (no synchronization).
        if fb.contains_key("x") && !fb.contains_key("y") {
            x_reached_final_before_y = true;
        }
        if r.completed {
            break;
        }
    }
    assert!(runner.is_completed());
    assert!(
        x_reached_final_before_y,
        "a merge must forward the first arrival without waiting (no sync) — contrast with join"
    );

    // Both branches' writes ultimately arrive (each token passes the merge).
    let fb = final_bindings(&runner);
    assert_eq!(fb.get("x"), Some(&Value::Int(1)));
    assert_eq!(fb.get("y"), Some(&Value::Int(3)));
}

// ===========================================================================
// OBL-IF — if-action-evaluates-test-then-branch
// "An IfThenAction evaluates its ifTest; if true it performs the thenClause; an
// IfThenElseAction additionally performs the elseClause when ifTest is false."
// Spec: SysML §8.4.13.9; `Actions.sysml:399-420` (IfThenAction / IfThenElseAction).
//
// Runtime surface: `ActionNodeIR::If { condition, then_branch, else_branch }`.
// The runner evaluates `condition`; on Bool(true) it routes the token to
// `then_branch`, on Bool(false) it routes to `else_branch` (or, when None, falls
// through to the node's plain successor). This matches the spec's
// then/else-clause selection, so CONFORMS.
// ===========================================================================

#[test]
fn if_action_true_test_performs_then_clause() {
    // OBL: if-action-evaluates-test-then-branch
    // VERDICT: CONFORMS
    // if (g > 0) then { then_marker = 1 } else { else_marker = 2 }, with g = 5.
    // Only the then-clause must run.
    let mut graph = ActionGraphIR::new("if_true", "IfTrue");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let seed = graph.add_node(ActionNodeIR::Assign {
        id: "seed".into(),
        target: "g".into(),
        value: ExprIR::LiteralInt(5),
    });
    let then_node = graph.add_node(ActionNodeIR::Assign {
        id: "thenc".into(),
        target: "then_marker".into(),
        value: ExprIR::LiteralInt(1),
    });
    let else_node = graph.add_node(ActionNodeIR::Assign {
        id: "elsec".into(),
        target: "else_marker".into(),
        value: ExprIR::LiteralInt(2),
    });
    let if_node = graph.add_node(ActionNodeIR::If {
        id: "if1".into(),
        condition: guard_cmp("g", BinOp::GreaterThan, 0),
        then_branch: "thenc".into(),
        else_branch: Some("elsec".into()),
    });

    graph.add_edge(&initial, &seed);
    graph.add_edge(&seed, &if_node);
    graph.add_edge(&then_node, &final_id);
    graph.add_edge(&else_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    let fb = final_bindings(&runner);
    assert_eq!(
        fb.get("then_marker"),
        Some(&Value::Int(1)),
        "ifTest true ⇒ thenClause performed"
    );
    assert!(
        !fb.contains_key("else_marker"),
        "ifTest true ⇒ elseClause must NOT be performed"
    );
}

#[test]
fn if_action_false_test_performs_else_clause() {
    // OBL: if-action-evaluates-test-then-branch
    // VERDICT: CONFORMS
    // Same graph with g = -5: an IfThenElseAction performs ONLY the elseClause.
    let mut graph = ActionGraphIR::new("if_false", "IfFalse");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let seed = graph.add_node(ActionNodeIR::Assign {
        id: "seed".into(),
        target: "g".into(),
        value: ExprIR::LiteralInt(-5),
    });
    let then_node = graph.add_node(ActionNodeIR::Assign {
        id: "thenc".into(),
        target: "then_marker".into(),
        value: ExprIR::LiteralInt(1),
    });
    let else_node = graph.add_node(ActionNodeIR::Assign {
        id: "elsec".into(),
        target: "else_marker".into(),
        value: ExprIR::LiteralInt(2),
    });
    let if_node = graph.add_node(ActionNodeIR::If {
        id: "if1".into(),
        condition: guard_cmp("g", BinOp::GreaterThan, 0),
        then_branch: "thenc".into(),
        else_branch: Some("elsec".into()),
    });

    graph.add_edge(&initial, &seed);
    graph.add_edge(&seed, &if_node);
    graph.add_edge(&then_node, &final_id);
    graph.add_edge(&else_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    let fb = final_bindings(&runner);
    assert_eq!(
        fb.get("else_marker"),
        Some(&Value::Int(2)),
        "ifTest false ⇒ elseClause performed (IfThenElseAction)"
    );
    assert!(
        !fb.contains_key("then_marker"),
        "ifTest false ⇒ thenClause must NOT be performed"
    );
}

// ===========================================================================
// OBL-WHILE — while-loop-iterates-while-test
// "A WhileLoopAction performs its body while whileTest is true (and untilTest is
// false); it terminates when whileTest is false (or untilTest is true)."
// Spec: SysML §8.4.13.10; `Actions.sysml:452-484` (whileTest / untilTest / body).
//
// Runtime surface: `ActionNodeIR::WhileLoop { condition, body_entry, exit_node }`.
// `condition` models the spec's `whileTest`: the body executes once per true
// evaluation and the loop exits at the first false evaluation — the core
// while-iteration obligation, so CONFORMS for the whileTest path.
//
// NOTE (partial): the IR carries a SINGLE `condition` and has no separate
// `untilTest` channel, so the spec's until-test early-exit cannot be expressed
// on a directly-built graph. That is a modelling gap on the WhileLoop IR shape,
// NOT a defect of the while-test semantics this obligation gates; it is called
// out here and tracked separately rather than failing this CONFORMS row.
// ===========================================================================

#[test]
fn while_loop_iterates_exactly_while_test_true() {
    // OBL: while-loop-iterates-while-test
    // VERDICT: CONFORMS
    // i = 0; while (i < 3) { i = i + 1 }. The body must run exactly 3 times and
    // the loop must exit at the first false whileTest (i == 3).
    let mut graph = ActionGraphIR::new("while_test", "WhileTest");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let seed = graph.add_node(ActionNodeIR::Assign {
        id: "seed".into(),
        target: "i".into(),
        value: ExprIR::LiteralInt(0),
    });
    let body = graph.add_node(ActionNodeIR::Assign {
        id: "body".into(),
        target: "i".into(),
        value: ExprIR::BinaryOp {
            op: BinOp::Add,
            left: Box::new(ExprIR::FeatureRef("i".into())),
            right: Box::new(ExprIR::LiteralInt(1)),
        },
    });
    let while_node = graph.add_node(ActionNodeIR::WhileLoop {
        id: "while1".into(),
        condition: guard_cmp("i", BinOp::LessThan, 3),
        body_entry: "body".into(),
        exit_node: final_id.clone(),
    });

    graph.add_edge(&initial, &seed);
    graph.add_edge(&seed, &while_node);
    graph.add_edge(&body, &while_node); // back edge: body returns to the loop header
    graph.add_edge(&while_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    let results = run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    // Body executions = "i =" outputs from the body, excluding the seed. The
    // seed also writes "i", so count total minus the one seed write.
    let i_writes = results
        .iter()
        .flat_map(|r| &r.outputs)
        .filter(|o| o.contains("i ="))
        .count();
    assert_eq!(
        i_writes, 4,
        "1 seed + 3 body iterations (loop exits at first false whileTest)"
    );
    assert_eq!(
        final_bindings(&runner).get("i"),
        Some(&Value::Int(3)),
        "loop terminates exactly when whileTest (i < 3) is false"
    );
}

#[test]
fn while_loop_false_test_skips_body() {
    // OBL: while-loop-iterates-while-test
    // VERDICT: CONFORMS
    // whileTest false on entry ⇒ body performed zero times.
    let mut graph = ActionGraphIR::new("while_skip", "WhileSkip");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let body = graph.add_node(ActionNodeIR::Assign {
        id: "body".into(),
        target: "ran".into(),
        value: ExprIR::LiteralInt(1),
    });
    let while_node = graph.add_node(ActionNodeIR::WhileLoop {
        id: "while1".into(),
        condition: ExprIR::LiteralBool(false),
        body_entry: "body".into(),
        exit_node: final_id.clone(),
    });

    graph.add_edge(&initial, &while_node);
    graph.add_edge(&body, &while_node);
    graph.add_edge(&while_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());
    assert!(
        !final_bindings(&runner).contains_key("ran"),
        "whileTest false on entry ⇒ body never performed"
    );
}

// ===========================================================================
// OBL-FOR — for-loop-iterates-over-sequence
// "A ForLoopAction assigns each successive value from `seq` to its loop variable
// `var` and performs its body once per element."
// Spec: SysML §8.4.13.10; `Actions.sysml:485-531` (ForLoopAction over seq).
//
// Runtime surface: `ActionNodeIR::ForLoop { variable, sequence, body_entry,
// exit_node }`. The runner evaluates `sequence` to a `Value::List`, binds each
// element to `variable`, and runs the body per element. Matches the spec's
// per-element var-assign + body-perform, so CONFORMS.
// ===========================================================================

#[test]
fn for_loop_assigns_each_value_and_performs_body() {
    // OBL: for-loop-iterates-over-sequence
    // VERDICT: CONFORMS
    // for v in [10, 20, 30] { seen = v }. The body runs once per element and the
    // loop variable holds each successive value (last write is the final value).
    let mut graph = ActionGraphIR::new("for_seq", "ForSeq");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let body = graph.add_node(ActionNodeIR::Assign {
        id: "body".into(),
        target: "seen".into(),
        value: ExprIR::FeatureRef("v".into()),
    });
    let for_node = graph.add_node(ActionNodeIR::ForLoop {
        id: "for1".into(),
        variable: "v".into(),
        sequence: ExprIR::Sequence(vec![
            ExprIR::LiteralInt(10),
            ExprIR::LiteralInt(20),
            ExprIR::LiteralInt(30),
        ]),
        body_entry: "body".into(),
        exit_node: final_id.clone(),
    });

    graph.add_edge(&initial, &for_node);
    graph.add_edge(&body, &for_node);
    graph.add_edge(&for_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    let results = run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());

    let body_runs = results
        .iter()
        .flat_map(|r| &r.outputs)
        .filter(|o| o.contains("seen ="))
        .count();
    assert_eq!(body_runs, 3, "body performed once per sequence element");
    assert_eq!(
        final_bindings(&runner).get("seen"),
        Some(&Value::Int(30)),
        "loop var took each successive value; last element is 30"
    );
}

#[test]
fn for_loop_empty_sequence_performs_no_body() {
    // OBL: for-loop-iterates-over-sequence
    // VERDICT: CONFORMS
    // An empty seq ⇒ zero body performances (boundary of the per-element rule).
    let mut graph = ActionGraphIR::new("for_empty", "ForEmpty");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();

    let body = graph.add_node(ActionNodeIR::Assign {
        id: "body".into(),
        target: "ran".into(),
        value: ExprIR::LiteralInt(1),
    });
    let for_node = graph.add_node(ActionNodeIR::ForLoop {
        id: "for1".into(),
        variable: "v".into(),
        sequence: ExprIR::Sequence(vec![]),
        body_entry: "body".into(),
        exit_node: final_id.clone(),
    });

    graph.add_edge(&initial, &for_node);
    graph.add_edge(&body, &for_node);
    graph.add_edge(&for_node, &final_id);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());
    assert!(
        !final_bindings(&runner).contains_key("ran"),
        "empty seq ⇒ body never performed"
    );
}

// ===========================================================================
// OBL-PERFORM — perform-action-is-referential
// "A PerformActionUsage is ALWAYS referential (isComposite = false). When owned
// by a Part (or OccurrenceDefinition) it subsets `Parts::Part::performedActions`;
// the referenced Action is considered performed by the owning Part."
// Spec: SysML §8.4.13.11 / §7.17.6; `Parts.sysml:38` (performedActions feature).
//
// This is a STRUCTURAL static-semantics obligation about the model element
// (its `isComposite` flag and an implicit subsetting of `performedActions`), NOT
// a token-flow behavior. The runtime's execution surface for perform is
// `ActionNodeIR::Perform { action_ref, inputs, output_binding, sub_action }` — it
// invokes a sub-action but carries NO `isComposite`/referential flag and does NO
// `performedActions` subsetting (that would live in the parser/elaborator's
// model-graph stamping, not in the action runner). There is therefore NO runtime
// surface that observes this obligation: UNIMPLEMENTED. The test pins what IS
// observable (a Perform node executes its referenced sub-action) and is ignored
// until the structural referential/subsetting property is modelled and gated.
// ===========================================================================

#[test]
#[ignore = "UNIMPL: PerformActionUsage referential constraint (isComposite=false \
            + subsets Parts::Part::performedActions) is a STRUCTURAL model property \
            with no action-runtime surface — ActionNodeIR::Perform carries no \
            referential flag and does no performedActions subsetting (§8.4.13.11 / \
            §7.17.6; Parts.sysml:38). Needs model-graph stamping gated elsewhere."]
fn perform_action_is_referential() {
    // OBL: perform-action-is-referential
    // VERDICT: UNIMPLEMENTED
    // Compiling pin: a Perform node DOES execute its referenced (inline)
    // sub-action — the behavioral half that exists today. The referential /
    // performedActions-subsetting STRUCTURAL property has no runtime surface, so
    // this row stays ignored. The assertion below is the spec-correct behavioral
    // expectation that can compile; the structural obligation cannot be expressed
    // on ActionGraphIR at all.
    let mut sub = ActionGraphIR::new("sub", "Sub");
    let sub_initial = sub.initial_node_id.clone();
    let sub_final = sub.final_node_ids[0].clone();
    let sub_body = sub.add_node(ActionNodeIR::Assign {
        id: "sub_body".into(),
        target: "performed".into(),
        value: ExprIR::LiteralInt(1),
    });
    sub.add_edge(&sub_initial, &sub_body);
    sub.add_edge(&sub_body, &sub_final);

    let mut graph = ActionGraphIR::new("perform_ref", "PerformRef");
    let initial = graph.initial_node_id.clone();
    let final_id = graph.final_node_ids[0].clone();
    let perform = graph.add_node(ActionNodeIR::Perform {
        id: "perf1".into(),
        action_ref: "Sub".into(),
        inputs: Vec::new(),
        output_binding: None,
        sub_action: None,
    });
    graph.add_edge(&initial, &perform);
    graph.add_edge(&perform, &final_id);
    let graph = graph.with_sub_action("perf1", sub);

    let mut runner = ActionRunner::new(graph);
    run_to_completion(&mut runner, &EvalContext::new());
    assert!(runner.is_completed());
    // Behavioral half (perform invokes the referenced action) holds today; the
    // referential/subsetting STRUCTURAL obligation is what is UNIMPLEMENTED.
    assert_eq!(
        final_bindings(&runner).get("performed"),
        Some(&Value::Int(1)),
        "a Perform node performs its referenced action (behavioral half only)"
    );
}

// ===========================================================================
// Matrix summary — self-scans this file and prints verdict counts.
// ===========================================================================

#[test]
fn asc_matrix_summary() {
    let src = include_str!("action_spec_conformance.rs");
    let verdicts: Vec<&str> = src
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("// VERDICT: "))
        .collect();
    let conforms = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: CONFORMS"))
        .count();
    let diverges = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: DIVERGES"))
        .count();
    let unimpl = verdicts
        .iter()
        .filter(|l| l.starts_with("// VERDICT: UNIMPLEMENTED"))
        .count();
    println!(
        "ASC action control-node matrix: {} gated obligations — \
         {conforms} CONFORMS, {diverges} DIVERGES, {unimpl} UNIMPLEMENTED",
        verdicts.len()
    );
    // Control-node set (GAP-ACT-1): fork, join (x2), decision (x2), merge = 7.
    // Completeness-audit additions: if (x2), while (x2), for (x2), perform (x1) = 7.
    assert!(
        verdicts.len() >= 13,
        "expected ≥13 verdict-marked action gates (7 control-node + 6 if/while/for + 1 perform)"
    );
}
