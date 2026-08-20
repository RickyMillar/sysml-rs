//! RSC-4.3 Wave-3 review finding F1 — multi-fire same-tick accumulation.
//!
//! `restep_and_fire` (`orchestrator.rs`) iterates a tick's located crossings
//! earliest-first and calls `fire_sm_mid_tick` per crossing. Before the fix,
//! the result was stashed via `fired_sm.insert(sm_index, fired)` — an
//! `insert` OVERWRITES on a repeated `sm_index`. Two located crossings
//! targeting the SAME SM in one tick (legal: two independent comparators each
//! sign-changing in the tick's window, e.g. opposing thresholds on a fast
//! oscillation) silently dropped every fire but the last from step 3's shared
//! tail — the earlier fire's occurrence record was never begun/ended and its
//! sends/port_sends were never routed, even though the fire genuinely
//! happened (its slot writeback DID land, since that happens inside
//! `fire_sm_mid_tick` itself, independent of the tail).
//!
//! This is a RED test written BEFORE the fix (confirmed red via a diagnostic
//! probe against the pre-fix `fired_sm: HashMap<_, (String, TickOutput)>`);
//! the fix accumulates into `HashMap<_, Vec<(String, TickOutput)>>` and
//! applies the tail's side effects to every fire, in fire order.
//!
//! Model: two INDEPENDENT monotone ODE state vars (`x` rising, `y` falling)
//! on the same part, each gating a DIFFERENT chained transition on the same
//! SM (`idle->up` on `x`, `up->down` on `y`). This is deliberately NOT a
//! single oscillating comparator crossing two opposing thresholds: the
//! detector's `check` is ENDPOINT-based (`ode_events.rs::locate_crossing`) —
//! `g_start`/`g_end` sign comparison only, no dense sub-tick scan — so a
//! Rising-armed and a Falling-armed threshold on the SAME variable can never
//! both net-satisfy their endpoint conditions within one window (the
//! endpoints can't be simultaneously "ended above T1" and "ended below T2"
//! for T1<T2). Two INDEPENDENT variables sidesteps that geometric impossibility
//! while still exercising the real multi-crossing-per-tick path: at a coarse
//! dt=1000ms, the tick's [0,1] window nets x: 0->1 (crosses 0.5 rising at
//! t=0.5s) AND y: 1->0 (crosses 0.2 falling at t=0.8s) — both located in the
//! SAME `check()` call, both targeting the SAME SM ("Flipper").

use sysml_core::{elaborate, Value};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::Orchestrator;

const TWO_VAR: &str = r#"
package TwoVar {
    private import ScalarValues::*;
    part def Osc {
        out attribute x : Real default 0.0;
        out attribute y : Real default 1.0;
        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative { return dxdt = 1.0; }
            calc def YDeriv :> GetDerivative { return dydt = 0.0 - 1.0; }
        }
    }
    state def Flipper {
        in attribute x : Real;
        in attribute y : Real;
        state idle;
        state up { entry send 1.0 via upPort; }
        state down { entry send 2.0 via downPort; }
        entry; then idle;
        transition idle_to_up first idle accept when x >= 0.5 then up;
        transition up_to_down first up accept when y <= 0.2 then down;
    }
}
"#;

fn build() -> Orchestrator {
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("TwoVar.sysml", TWO_VAR.to_owned())]);
    let mut graph = result.graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let mut orch = compiler
        .build_workspace_orchestrator(
            base_ctx, None, None, None, None, &[], Some(1000.0), Some(20_000.0),
        )
        .expect("TwoVar orchestrator builds");
    orch.context.set("x".to_owned(), Value::Float(0.0));
    orch.context.set("y".to_owned(), Value::Float(1.0));
    orch
}

/// Both crossings must be located and both must actually change the SM's
/// state within the FIRST tick (sanity: the multi-fire regime is real, not a
/// test-construction error).
#[test]
fn two_independent_crossings_locate_and_fire_within_one_tick() {
    let mut orch = build();
    assert_eq!(
        orch.crossing_detector_event_count("Osc"),
        2,
        "both `accept when` transitions must register a detector event"
    );
    let _ = orch.step();
    let state = orch
        .subsystems()
        .iter()
        .find(|s| s.name == "Flipper")
        .map(|s| s.executor.current_state_name().to_owned())
        .unwrap_or_default();
    assert_eq!(
        state, "down",
        "both transitions (idle->up on x>=0.5 at t=0.5s, up->down on y<=0.2 at t=0.8s) \
         must have fired WITHIN the single dt=1000ms tick"
    );
}

/// THE F1 regression: every fire's state-transition occurrence must be
/// recorded — not just the last one's. `up` was entered and exited entirely
/// within the same tick as `down`'s entry; pre-fix, `fired_sm.insert`
/// overwrote the `idle->up` fire before step 3's tail ever ran its
/// occurrence-lifecycle side effects for it, so `up` never appeared as a
/// completed (or even active) occurrence — invisible, not merely
/// short-duration.
#[test]
fn f1_every_fire_records_its_occurrence_not_just_the_last() {
    let mut orch = build();
    let _ = orch.step();

    let completed: Vec<(String, String)> = orch
        .occurrences()
        .completed()
        .iter()
        .map(|o| (o.subsystem.clone(), o.name.clone()))
        .collect();
    let active: Vec<(String, String)> = orch
        .occurrences()
        .active()
        .iter()
        .map(|o| (o.subsystem.clone(), o.name.clone()))
        .collect();

    assert!(
        completed.contains(&("Flipper".to_owned(), "up".to_owned())),
        "the idle->up fire's occurrence must be recorded as completed (up->down's tail \
         ends it) — pre-fix this fire's tail never ran, so `up` is entirely absent from \
         both completed AND active. completed={completed:?} active={active:?}"
    );
    assert!(
        active.contains(&("Flipper".to_owned(), "down".to_owned())),
        "the final fire's occurrence must still be recorded as active (unaffected by the \
         fix — this is the pre-fix survivor). completed={completed:?} active={active:?}"
    );
}

/// THE F1 regression, routing half: every fire's `send via <port>` must
/// appear in the tick's trace snapshot (`SubsystemState.sends`) — not just
/// the last fire's. `output.sends`/`output.port_sends` are set together by
/// the same compiler-emitted action (`statemachine/mod.rs`'s `SendActionUsage`
/// walk), so a dropped `up`-fire `TickOutput` loses BOTH the trace string
/// ("send via upPort") and the real routed payload identically — this
/// asserts the trace half, which is directly observable without wiring a
/// receiving port.
#[test]
fn f1_every_fires_send_reaches_the_tick_snapshot() {
    let mut orch = build();
    let _ = orch.step();

    let snap = orch
        .trace()
        .first()
        .expect("the first (and only, so far) tick must have produced a snapshot");
    let sends = &snap
        .subsystem_states
        .get("Flipper")
        .expect("Flipper subsystem must be in the snapshot")
        .sends;

    assert!(
        sends.contains(&"send via upPort".to_owned()),
        "the idle->up fire's `send via upPort` must reach the snapshot — pre-fix only the \
         LAST fire's TickOutput reached step 3, so this is silently dropped. sends={sends:?}"
    );
    assert!(
        sends.contains(&"send via downPort".to_owned()),
        "the up->down fire's `send via downPort` must ALSO be present (this one survives \
         even pre-fix, as the last/overwriting fire). sends={sends:?}"
    );
}
