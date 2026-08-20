//! P5 — qualified `<PrimarySubsystemName>.<var>` override targets on
//! top-level (non-instanced) subsystems.
//!
//! Bug: the sim-app's model tree builds override keys as
//! `${ownerPath}.${name}`, where `ownerPath` is purely SysML-containment-derived
//! (never consults the runtime's subsystem list). For a PREFIXED
//! (instance-multiplied) subsystem this qualified spelling already resolves,
//! because its `runtime_name` is always instance-prefixed and happens to
//! coincide with `<SubsystemName>.<var>`. For a PRIMARY (top-level) subsystem
//! — no `part` usage, `var_prefix: None` — the mint pass registered only the
//! bare name, so the qualified spelling fell through to RS002 "unknown override
//! target" even though the exact same variable, unqualified, resolved fine.
//!
//! Fix (`mint_qualified_alias`/`mint_primary_sm_alias`): mint an extra
//! `SlotStore::add_alias` entry — never a `canonical_name`/`runtime_name`
//! mutation (core-steward P5 ruling: mutating those for a `var_prefix: None`
//! subsystem trips `WriteRoute::resolve_inner`'s strict `canonical_name ==
//! runtime_name` invariant and silently demotes the owning executor's own
//! tick-time writeback onto the legacy name-keyed fallback path) — under
//! `"{subsystem_name}.{bare_var}"` for every top-level ODE state var /
//! parameter / signal output and SM assignment target, mirroring what the
//! instanced path already gets for free from its prefixed `runtime_name`.
//!
//! This gate uses a self-contained generic model (`OverrideProbe`) with a
//! top-level ODE part (`Plant`, an ODE PARAMETER `bias`) and a top-level state
//! machine (`Regulator`, an SM-assign target `setpoint`), so both alias mint
//! paths (`mint_qualified_alias` for the ODE parameter, `mint_primary_sm_alias`
//! for the SM assignment) are exercised without any product fixture. The bare
//! and unknown-target paths are additionally covered by the espresso cell's
//! `cell_qualified_override_is_isolated` and by
//! `build_override_target_fail_hard`.

use sysml_core::{elaborate, Value};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::Orchestrator;

/// Generic top-level ODE + SM model. `Plant` is a non-instanced ODE part whose
/// derivative reads the bare-numeric parameter `bias` (swept in by
/// `detect_ode_from_ssr`); `Regulator` is a non-instanced state machine whose
/// entry action assigns `setpoint`. Both are `var_prefix: None` subsystems, so
/// the qualified override spellings `Plant.bias` / `Regulator.setpoint` only
/// resolve via the P5 aliases.
const OVERRIDE_PROBE: &str = r#"
package OverrideProbe {
    private import ScalarValues::*;

    part def Plant {
        attribute bias : Real default 0.0;
        attribute setpoint : Real default 3.47;
        out attribute x : Real default 0.0;

        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative {
                return dxdt = bias;
            }
        }
    }

    state def Regulator {
        state hold {
            entry action { setpoint = 3.47; }
        }
        entry; then hold;
    }
}
"#;

fn build_probe() -> Orchestrator {
    let parser = TreeSitterParser::new();
    let mut graph = parser
        .parse(&[SysmlFile::new("OverrideProbe.sysml", OVERRIDE_PROBE.to_owned())])
        .graph;
    elaborate::elaborate(&mut graph);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    compiler
        .build_workspace_orchestrator(
            base_ctx, None, None, None, None, &[], Some(1.0), Some(1_000.0),
        )
        .expect("override-probe orchestrator builds")
}

fn float_at(orch: &Orchestrator, key: &str) -> Option<f64> {
    match orch.context.get(key) {
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    }
}

/// A top-level ODE PARAMETER, qualified by its part-def subsystem name
/// (`Plant.bias`), resolves via `mint_qualified_alias` and reads back through
/// both the qualified and the bare spelling (same slot).
#[test]
fn p5_qualified_ode_parameter_override_resolves() {
    let mut orch = build_probe();

    orch.apply_overrides_with_aliases(&[("Plant.bias".to_owned(), "0.045".to_owned())])
        .expect("qualified ODE-parameter override must resolve via the new alias — was RS002");

    assert_eq!(
        float_at(&orch, "bias"),
        Some(0.045),
        "qualified spelling in -> bare (runtime_name) spelling reads back"
    );
    assert_eq!(
        float_at(&orch, "Plant.bias"),
        Some(0.045),
        "qualified spelling in -> qualified spelling reads back (alias resolves to the same slot)"
    );
}

/// An SM-assign target, qualified by its state-def subsystem name
/// (`Regulator.setpoint`), exercises `mint_primary_sm_alias` and reads back
/// through the bare spelling.
#[test]
fn p5_qualified_sm_assign_target_override_resolves() {
    let mut orch = build_probe();

    orch.apply_overrides_with_aliases(&[("Regulator.setpoint".to_owned(), "5.0".to_owned())])
        .expect("qualified SM-assign-target override must resolve via the new alias — was RS002");

    assert_eq!(
        float_at(&orch, "setpoint"),
        Some(5.0),
        "SM-qualified spelling in -> bare spelling reads back"
    );
}

/// Regression pin: the bare (unqualified) spelling that already worked before
/// this fix must keep working, unchanged.
#[test]
fn p5_bare_override_still_works() {
    let mut orch = build_probe();

    orch.apply_overrides_with_aliases(&[("bias".to_owned(), "0.09".to_owned())])
        .expect("bare override must keep working");
    assert_eq!(float_at(&orch, "bias"), Some(0.09));
}

/// Negative twin (the fix must not paper over genuinely-unknown targets): a
/// qualified name naming a REAL subsystem but a var that doesn't exist on it,
/// and a qualified name naming a subsystem that doesn't exist at all, must both
/// still hard-fail RS002 — the alias table only ever grows by real, minted
/// `(subsystem, var)` pairs, never by guessing/normalizing an unknown qualifier
/// away.
#[test]
fn p5_unknown_qualified_target_still_rs002() {
    let mut orch = build_probe();

    let real_subsystem_bogus_var = "Plant.NoSuchVariableXyz";
    let err = orch
        .apply_overrides_with_aliases(&[(real_subsystem_bogus_var.to_owned(), "1.0".to_owned())])
        .expect_err("a real subsystem name with a nonexistent var must still RS002");
    assert!(
        err.to_string().contains("RS002") && err.to_string().contains(real_subsystem_bogus_var),
        "error names its code and the offending target: {err}"
    );

    let bogus_subsystem_real_var = "NoSuchSubsystemXyz.bias";
    let err = orch
        .apply_overrides_with_aliases(&[(bogus_subsystem_real_var.to_owned(), "1.0".to_owned())])
        .expect_err("a nonexistent subsystem qualifying a real var name must still RS002");
    assert!(
        err.to_string().contains("RS002") && err.to_string().contains(bogus_subsystem_real_var),
        "error names its code and the offending target: {err}"
    );

    // Fail-hard, not silent creation: neither bogus name materializes.
    assert!(orch.context.get(real_subsystem_bogus_var).is_none());
    assert!(orch.context.get(bogus_subsystem_real_var).is_none());
}
