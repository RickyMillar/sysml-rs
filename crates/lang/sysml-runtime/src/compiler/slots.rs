//! Slot minting plus writer-conflict and RS003-RS005 diagnostics.

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementId, ElementKind, Value};
use sysml_span::Diagnostic;

use crate::orchestrator::{Orchestrator, SubsystemIndex};
use crate::slots::SlotStore;

use super::*;

/// A state-machine assignment target collected during SM compilation,
/// classified `Discrete` in the slot table (design doc D-2.0.3).
pub(crate) struct SmAssignTarget {
    /// Context key the runtime writes (`"tripped"` or `"circuit1.tripped"`).
    pub(crate) runtime_name: String,
    /// Unprefixed target name as written in the SM action.
    pub(crate) bare_name: String,
    /// Index into the `InstanceSpec` slice when this target belongs to a
    /// multiplied per-instance SM.
    pub(crate) instance: Option<usize>,
    /// [`SubsystemIndex`](crate::orchestrator::SubsystemIndex) of the owning
    /// SM, captured at registration (RSC-4.2 L40) — never re-derived by name.
    pub(crate) subsystem: SubsystemIndex,
}

/// RSC-3.5b: a state-machine port-payload binding target. The SET of accept
/// ports a SM can receive payloads on is compile-static (`accept_ports()`),
/// so the compiler pre-mints a `{port}.payload` Discrete slot per port; the
/// SM executor then routes its tick-time payload bindings through them
/// (draining the `dynamic_keys` name-keyed fallback class). Only the message
/// VALUE is runtime-dynamic — exactly what the routed slot stores.
pub(crate) struct SmPayloadPort {
    /// Bare accept-port name (`"tripIn"`), as bound by the SM in `tick()`.
    pub(crate) port: String,
    /// Runtime-key BASE the SM writeback produces, sans the `.payload` /
    /// `_payload` suffix: `{prefix}.{port}` for a prefixed instance SM, or
    /// just `{port}` for a top-level SM.
    pub(crate) runtime_base: String,
    /// Canonical-key BASE: `{container}.{prefix}.{port}` for instance SMs,
    /// or `{port}` (== `runtime_base`) for a top-level SM.
    pub(crate) canonical_base: String,
    /// [`SubsystemIndex`](crate::orchestrator::SubsystemIndex) of the owning
    /// SM, captured at registration (RSC-4.2 L40) — never re-derived by name.
    pub(crate) subsystem: SubsystemIndex,
    /// Index into the `InstanceSpec` slice for a multiplied per-instance SM.
    pub(crate) instance: Option<usize>,
}

/// RSC-3.6: a read-leaf of a prefixed (instance-multiplied) state machine's
/// guard / `When` / `AfterExpr` expressions. A SM-only instance (no ODE, no
/// structured-action assignments) is minted no per-instance slots by the
/// ODE/assignment/computed steps, so its guard's reads resolve only through the
/// shared global context — leaving the executor scoped-view-bypass-INELIGIBLE
/// (`statemachine::scoped_bypass_eligible`). The compiler mints a per-instance
/// slot per read leaf so the guard binds to an instance-local `SlotRef`
/// (`SlotBinder::resolve_subsystem_local`) and the read becomes provably
/// servable from the instance's own slots (RSC-3.6, the 3.5f.3 prerequisite).
/// Read-only from the SM's perspective; seeded to reproduce the value the leaf
/// resolved to on the legacy scoped-context path (byte-identical, §1).
pub(crate) struct SmGuardRead {
    /// Runtime key the slot carries (`"controllerA.currentIn.reading.value"`).
    pub(crate) runtime_name: String,
    /// Canonical tree-path key (`"System.controllerA.currentIn.reading.value"`).
    pub(crate) canonical_name: String,
    /// The bare read key as it appears in the guard (`"currentIn.reading.value"`
    /// / `"threshold"`) — used to look up the bare-binding seed.
    pub(crate) leaf: String,
    /// Index into the `InstanceSpec` slice (the owning instance).
    pub(crate) instance: Option<usize>,
}

/// Borrowed inputs for [`ModelCompiler::mint_slot_store`].
pub(crate) struct SlotMintInputs<'a> {
    pub(crate) instances: &'a [InstanceSpec],
    pub(crate) ode_detections: &'a [OdeDetection],
    pub(crate) multiplied_ode_names: &'a HashSet<String>,
    pub(crate) computed_targets: &'a [String],
    pub(crate) sm_targets: &'a [SmAssignTarget],
    /// P5: bare (non-multiplied) SM names + their `SubsystemIndex`, captured
    /// at registration (the same list `wire_zero_crossing_detectors` consumes
    /// for WS-A2) — so a genuinely top-level `SmAssignTarget` (`instance:
    /// None`) can mint a `"{sm_name}.{var}"` alias resolving the qualified
    /// tree-path override spelling, mirroring what the instanced path
    /// already gets for free from its prefixed `runtime_name`. Looked up by
    /// `SubsystemIndex`, never re-derived by name at mint time.
    pub(crate) primary_sm_names: &'a [(String, SubsystemIndex)],
    /// RSC-3.6: per-instance guard/trigger read-leaf slot targets for prefixed
    /// state machines (see [`SmGuardRead`]).
    pub(crate) sm_guard_reads: &'a [SmGuardRead],
    /// RSC-3.5b: compile-static port-payload slot targets. Minted as Discrete
    /// slots owned by the receiving SM executor so its tick-time payload
    /// bindings route by SlotId (draining the `dynamic_keys` fallback class).
    pub(crate) sm_payload_ports: &'a [SmPayloadPort],
    pub(crate) override_map: &'a HashMap<&'a str, f64>,
    /// RSC-3.2: the classified link graph + port registry, so signal-link
    /// endpoint port features get minted as slots (Continuous, writer =
    /// Orchestrator — the link plane). `None` on paths that built no link
    /// graph (single-model `build_orchestrator`, tests).
    pub(crate) link_graph: Option<&'a crate::links::LinkGraph>,
    /// The compiled port registry, source of signal-port feature names + their
    /// initial values for RSC-3.2 minting.
    pub(crate) port_registry: Option<&'a crate::flows::PortRegistry>,
    /// RSC-3.4 / L31: physics executor write targets, compiled before the
    /// executor is consumed by `add_physics`. Minted as Continuous slots with
    /// `physics_writer` as the owning executor. `None` when no physics executor
    /// was built.
    pub(crate) physics_write_targets: Option<&'a [String]>,
    /// RSC-3.4 / L31: `WriterId` for the physics subsystem. `None` when no
    /// physics executor was registered.
    pub(crate) physics_writer: Option<crate::slots::WriterId>,
    /// WS-D Stage 2 duty-seam fix: the `SubsystemIndex`es of ODEs with a
    /// registered duty-cycle tracker (`Orchestrator::duty_tracker_indices`),
    /// so step 2c can mint-or-promote a `duty` slot + qualified alias for
    /// each — the same bare/qualified duality every other top-level ODE
    /// output already gets (P5). Empty on paths that never wire duty
    /// trackers (single-model builders, unit tests constructing
    /// `SlotMintInputs` directly).
    pub(crate) duty_tracker_odes: &'a HashSet<SubsystemIndex>,
    /// Registered discrete state-space subsystems and their state vectors
    /// (see [`DiscreteDetection`]). Minted exactly like top-level ODE state
    /// vars — `Continuous` variability, owned by the registering executor —
    /// because a discrete solver writes its state vector every tick through
    /// the same strict `WriteRoute`. Empty on paths that build no discrete
    /// subsystems (single-model builders, unit tests constructing
    /// `SlotMintInputs` directly).
    pub(crate) discrete_detections: &'a [DiscreteDetection],
}

/// P5: mint the `"{sm_name}.{var}"` alias for a genuinely top-level
/// (`instance: None`) `SmAssignTarget`'s slot, so the qualified tree-path
/// override spelling (`sessions.step overrides=[["<SubsystemName>.<var>",
/// ...]]`, the spelling the sim-app tree editor sends) resolves through
/// [`SlotStore::slot_by_name`](crate::slots::SlotStore::slot_by_name) the
/// same way the instanced path's already-prefixed `runtime_name` does today.
/// A plain [`add_alias`](crate::slots::SlotStore::add_alias) — never touches
/// `canonical_name`/`runtime_name` — so `WriteRoute::resolve_inner`'s strict
/// `canonical_name == runtime_name` invariant for `var_prefix: None`
/// subsystems (`slots.rs`) is untouched; core-steward-ruled (P5 consult)
/// after mutating `canonical_name` directly was found to demote the SM's own
/// tick-time writeback onto the legacy name-keyed fallback path.
///
/// No-op for an instanced target (`instance: Some(_)`) — its `runtime_name`
/// is already instance-prefixed, so nothing new needs resolving.
///
/// `sm_target.subsystem` is always present in `primary_sm_names` for a
/// genuinely top-level target — both are populated in the same
/// SM-registration pass, never re-derived by name (ledger L39/L40 direction)
/// — so a lookup miss is an internal invariant violation, not a case to
/// silently skip (RSC-4.2 ruling 4 fail-hard fold-in class).
fn mint_primary_sm_alias(
    store: &mut crate::slots::SlotStore,
    primary_sm_names: &[(String, SubsystemIndex)],
    sm_target: &SmAssignTarget,
    id: crate::slots::SlotId,
) -> Result<(), CompileError> {
    if sm_target.instance.is_some() {
        return Ok(());
    }
    let Some((sm_name, _)) = primary_sm_names
        .iter()
        .find(|(_, idx)| *idx == sm_target.subsystem)
    else {
        return Err(CompileError::from_message(format!(
            "internal: SM assignment target '{}' claims top-level subsystem \
             {:?} absent from primary_sm_names at slot-mint time (P5 mint-gap)",
            sm_target.bare_name, sm_target.subsystem
        )));
    };
    mint_qualified_alias(store, sm_name, &sm_target.bare_name, id);
    Ok(())
}

/// P5: mint a `"{qualifier}.{bare}"` alias for an otherwise-unqualified
/// top-level slot, so the qualified tree-path spelling the sim-app's model
/// tree sends (`ownerPath` is purely SysML-containment-derived — see
/// `buildModelTree.ts`) resolves through
/// [`SlotStore::slot_by_name`](crate::slots::SlotStore::slot_by_name) the
/// same way an instanced/prefixed slot's already-qualified `runtime_name`
/// does today. A plain [`add_alias`](crate::slots::SlotStore::add_alias) —
/// never touches `canonical_name`/`runtime_name` (core-steward P5 ruling:
/// mutating those for a `var_prefix: None` subsystem breaks
/// `WriteRoute::resolve_inner`'s strict-equality invariant and silently
/// demotes the owning executor's own tick-time writeback onto the legacy
/// name-keyed fallback path — verified by tracing `slots.rs:653-671` before
/// this fix was written this way).
fn mint_qualified_alias(
    store: &mut crate::slots::SlotStore,
    qualifier: &str,
    bare: &str,
    id: crate::slots::SlotId,
) {
    let alias = format!("{qualifier}.{bare}");
    if store.slot_by_name(&alias).is_none() {
        store.add_alias(alias, id);
    }
}

/// Nearest registered slot alias to `name` by Levenshtein distance,
/// accepted only at distance ≤ 2 (RSC-2.5: the RS003 hard error's
/// "did you mean" note). Runs only on the compile-error path.
fn nearest_slot_spelling(store: &crate::slots::SlotStore, name: &str) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for (candidate, _) in store.names() {
        if candidate == name || candidate.len().abs_diff(name.len()) > 2 {
            continue;
        }
        let d = levenshtein(name, candidate);
        if d <= 2 && best.is_none_or(|(bd, bc)| d < bd || (d == bd && candidate < bc)) {
            best = Some((d, candidate));
        }
    }
    best.map(|(_, c)| c.to_owned())
}

/// Plain DP Levenshtein distance over chars (short names; error path only).
fn levenshtein(a: &str, b: &str) -> usize {
    let mut prev: Vec<usize> = (0..=b.chars().count()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut curr = vec![i + 1];
        let mut prev_diag = prev.first().copied().unwrap_or(0);
        for (cb, &prev_above) in b.chars().zip(prev.iter().skip(1)) {
            let cost = usize::from(ca != cb);
            let ins = curr.last().copied().unwrap_or(0) + 1;
            let del = prev_above + 1;
            let sub = prev_diag + cost;
            curr.push(ins.min(del).min(sub));
            prev_diag = prev_above;
        }
        prev = curr;
    }
    prev.last().copied().unwrap_or(0)
}

impl ModelCompiler {
    /// RSC-3.0 category 2: mint (or promote) an ODE `GetOutput` signal-output
    /// slot so its writeback routes by `SlotId` (like a state var) through the
    /// strict [`WriteRoute::apply`](crate::slots::WriteRoute::apply), rather than
    /// name-keying through `apply_name_keyed`. Two cases:
    ///
    /// - **Already minted** (e.g. a defaulted `attribute i_drive` that the ODE
    ///   parameter collector also grabbed as `External`/`Parameter`): the
    ///   `GetOutput` calc is the tick-time producer, so PROMOTE ownership to the
    ///   owning ODE executor and **reclassify to `Continuous`** — the same motion
    ///   as the computed-target promotion in `mint_slot_store` step 4, and what
    ///   `claim_writer_promoting`'s contract mandates (the class of values the
    ///   executor produces at tick time). A slot already owned by a real executor
    ///   stays put and its claim feeds RS001.
    /// - **Not minted** (a pure `GetOutput` return like `kinetic_energy` with no
    ///   standalone attribute usage): mint fresh as `Continuous` (a continuously
    ///   recomputed derived output) owned by the ODE executor, init `Null`.
    ///
    /// Both branches settle on `Continuous`: a `GetOutput` signal recomputed
    /// every tick is a continuous derived output, not an externally-set
    /// `Parameter`. (D2, arc-review cleanup 2026-07-03 — the promote branch used
    /// to *preserve* the collected `Parameter` class, contradicting both the
    /// step-4 motion this doc claims and `claim_writer_promoting`'s own contract.
    /// The reclassification makes the slot dirty-scannable in the RSC-4.2
    /// convergence loop, which is correct for a coupled signal like `i_drive`.)
    ///
    /// `rid` is declaration-backed when the signal has an attribute usage, else
    /// a synthetic id — consumed only on the fresh-mint branch.
    fn mint_ode_signal_slot(
        store: &mut crate::slots::SlotStore,
        rid: crate::slots::RuntimeId,
        canonical_name: &str,
        runtime_name: &str,
        ode_writer: crate::slots::WriterId,
    ) {
        use crate::slots::{SlotMeta, Variability};
        if let Some(existing) = store.slot_by_name(runtime_name) {
            store.claim_writer_promoting(existing, ode_writer, Variability::Continuous);
            return;
        }
        store.intern(
            SlotMeta::new(
                rid,
                Variability::Continuous,
                ode_writer,
                canonical_name,
                runtime_name,
            ),
            Value::Null,
        );
    }

    /// Statically-known assignment targets of a compiled state machine.
    /// One home (RSC-2.4b): the walk lives in
    /// [`crate::statemachine::collect_assignment_targets`] so the slot-claim
    /// collection here and the runner's compiled write-set can never drift.
    pub(crate) fn collect_sm_assignment_targets(ir: &crate::StateMachineIR) -> Vec<String> {
        crate::statemachine::collect_assignment_targets(ir)
    }

    /// Mint the compile-known slot table (RSC-2.1/2.2, design doc D-2.0.2 /
    /// D-2.0.3). As of RSC-2.2 the table is consumed: the orchestrator
    /// attaches it to the master `EvalContext` (write-through routing,
    /// D-2.0.6) and [`SlotStore::multi_writer_conflicts`] gates the RS001
    /// hard error at build time.
    ///
    /// Classification sources, in claim order (first claim wins a name; a
    /// later pass targeting an existing slot records an additional writer
    /// claim instead of minting — that claim is exactly what RS001 reports):
    /// 1. orchestrator bookkeeping (`t_ms`, `tick`, `__clock_time`),
    /// 2. ODE state vars → `Continuous` / `Executor(owning ODE subsystem)`
    ///    (top-level and per-instance, the latter with both canonical and
    ///    runtime name forms),
    /// 3. ODE parameters → `Parameter` / `External`,
    /// 4. computed/gated expression targets → `Continuous` / `Orchestrator`,
    /// 5. SM assignment targets (structured AND parsed `Simple` strings) →
    ///    `Discrete` / `Executor(owning SM subsystem)`,
    /// 6. remaining bare bindings: literal defaults → `Parameter` /
    ///    `External`; lazily-resolved expression defaults (`Value::Ref`,
    ///    materialized by the orchestrator's snapshot ref-resolution) →
    ///    `Continuous` / `Orchestrator`.
    ///
    /// Variables only discoverable at runtime (dynamic names, port/flow
    /// keys — Phase 3) are deliberately absent. ODE/SM writers resolve
    /// through the `SubsystemIndex` each detection/target captured directly
    /// at its own registration call site (RSC-4.2 L40) — never re-derived by
    /// name. A writer-claimed ODE detection whose `subsystem_index` is still
    /// `None` at mint time (its subsystem failed to register) is a
    /// **hard error** (ruling 4) — the soft `WriterId::Orchestrator`
    /// placeholder fallback is deleted.
    pub(crate) fn mint_slot_store(&self, inputs: &SlotMintInputs<'_>) -> Result<SlotStore, CompileError> {
        use crate::slots::{RuntimeId, SlotMeta, Variability, WriterId};
        use smallvec::SmallVec;

        let mut store = SlotStore::new();
        let bare = self.collect_bare_bindings();

        // 1. Orchestrator bookkeeping slots. These have no model
        // declaration; their RuntimeIds use deterministic synthetic ids.
        let bookkeeping_slots: [(&str, Variability, Value); 3] = [
            ("t_ms", Variability::Continuous, Value::Float(0.0)),
            ("tick", Variability::Discrete, Value::Int(0)),
            ("__clock_time", Variability::Continuous, Value::Float(0.0)),
        ];
        for (name, variability, init) in bookkeeping_slots {
            let rid = RuntimeId::top_level(ElementId::from_string(format!(
                "sysml-runtime:bookkeeping:{name}"
            )));
            store.intern(
                SlotMeta::new(rid, variability, WriterId::Orchestrator, name, name).bookkeeping(),
                init,
            );
        }

        // 2. Top-level (non-multiplied) ODE detections.
        for ode in inputs.ode_detections {
            if ode
                .name
                .as_ref()
                .is_some_and(|n| inputs.multiplied_ode_names.contains(n))
            {
                continue;
            }
            // The SubsystemIndex is captured directly on the detection at
            // its own registration call site (RSC-4.2 L40) — an unresolved
            // index for a writer-claimed ODE is a hard mint-time error
            // (ruling 4), not a silent Orchestrator-owned fallback.
            let Some(ode_writer) = ode.subsystem_index.map(WriterId::from) else {
                return Err(CompileError::from_message(format!(
                    "internal: ODE detection '{}' has no registered subsystem at \
                     slot-mint time (RSC-4.2 mint-gap)",
                    ode.name.as_deref().unwrap_or("<unnamed>")
                )));
            };
            for (i, var) in ode.state_vars.iter().enumerate() {
                let Some(decl) = self.find_ode_feature_decl(ode, var, &bare) else {
                    continue;
                };
                let init = ode
                    .initial_values
                    .get(i)
                    .copied()
                    .map(Value::Float)
                    .unwrap_or(Value::Null);
                let id = store.intern(
                    SlotMeta::new(
                        RuntimeId::top_level(decl),
                        Variability::Continuous,
                        ode_writer,
                        var.as_str(),
                        var.as_str(),
                    ),
                    init,
                );
                // P5: qualified-alias the bare top-level state var under the
                // ODE's own registered name, mirroring the per-instance
                // branch's `canonical_prefix`-qualified `canonical_name`
                // (step 3 below) — via `add_alias`, not `canonical_name`
                // mutation (a `var_prefix: None` top-level ODE would otherwise
                // trip `WriteRoute::resolve_inner`'s strict-equality
                // invariant; see `mint_qualified_alias`'s doc).
                if let Some(ode_name) = ode.name.as_deref() {
                    mint_qualified_alias(&mut store, ode_name, var, id);
                }
            }
            for (k, v) in &ode.parameters {
                let Some(decl) = self.find_ode_feature_decl(ode, k, &bare) else {
                    continue;
                };
                let val = inputs.override_map.get(k.as_str()).copied().unwrap_or(*v);
                let id = store.intern(
                    SlotMeta::new(
                        RuntimeId::top_level(decl),
                        Variability::Parameter,
                        WriterId::External,
                        k.as_str(),
                        k.as_str(),
                    ),
                    Value::Float(val),
                );
                // P5: same qualified-alias treatment for top-level ODE
                // parameters (e.g. a top-level ODE parameter `I_residual`
                // — the reported RS002 case: `sessions.step
                // overrides=[["<Model>.I_residual", ...]]`).
                if let Some(ode_name) = ode.name.as_deref() {
                    mint_qualified_alias(&mut store, ode_name, k, id);
                }
            }
            // 2b. RSC-3.0 category 2: GetOutput signal outputs (e.g.
            // `kinetic_energy`, `i_drive`). Mint/promote after params so a
            // signal also collected as a defaulted parameter is promoted, not
            // double-minted. Sorted for deterministic mint order (ws_determinism).
            let mut sig_names: Vec<&String> = ode.signal_exprs.keys().collect();
            sig_names.sort();
            for sig in sig_names {
                let rid = self
                    .find_ode_feature_decl(ode, sig, &bare)
                    .map(RuntimeId::top_level)
                    .unwrap_or_else(|| {
                        RuntimeId::top_level(ElementId::from_string(format!(
                            "ode-signal:{}:{sig}",
                            ode.name.as_deref().unwrap_or("ode")
                        )))
                    });
                Self::mint_ode_signal_slot(&mut store, rid, sig, sig, ode_writer);
                // P5: same qualified-alias treatment for top-level GetOutput
                // signals (e.g. `i_drive`). `mint_ode_signal_slot` mints
                // (or promotes an existing slot) under the bare `sig`
                // spelling for both name forms at this call site, so it's
                // always resolvable right after the call.
                if let (Some(ode_name), Some(id)) = (ode.name.as_deref(), store.slot_by_name(sig))
                {
                    mint_qualified_alias(&mut store, ode_name, sig, id);
                }
            }
            // 2c. WS-D Stage 2 duty-seam fix (steward-ruled 2026-07-06): mint
            // a `duty` slot + `{ode_name}.duty` qualified alias for every ODE
            // with a registered duty-cycle tracker. `Orchestrator::
            // add_duty_tracker` writes `"{ode_name}.duty"` into the master
            // context; `EvalContext::set` write-throughs via
            // `SlotStore::set_by_name` (writer-agnostic) — but only once a
            // slot named `duty` (bare) exists AND is aliased under the
            // qualified spelling to the SAME `SlotId`, which is exactly what
            // was missing: nothing minted `duty` before this step, so a
            // model-level calc-def referencing bare `duty` (e.g.
            // `interpolateSaturating(dutyToCurrentLUT, duty)`) resolved
            // against a slot the tracker's write never touched and stayed
            // frozen at the model's literal default forever. This is
            // orchestrator bookkeeping (crossing-event cadence), not an
            // OdeWriteSet-claimed continuous state, so the writer is
            // `WriterId::Orchestrator`, not `Executor(ode_index)` — same
            // classification as `t_ms`/`tick`/`__clock_time` above, computed/
            // gated → Continuous/Orchestrator (compiler.rs:5013).
            // Unconditional mint (no gating on a declared `duty` attribute),
            // mirroring `mint_ode_signal_slot`'s synthetic fresh-mint branch:
            // `find_ode_feature_decl` finds a declaration-backed id when the
            // model declares `attribute duty` (promoting that slot's writer),
            // and falls back to a synthetic id when it doesn't — the
            // introspection-only path with no `duty` attribute at all
            // (protection_core_physics.rs's non-fault-detection gate) keeps working
            // unchanged either way.
            if let Some(idx) = ode.subsystem_index {
                if inputs.duty_tracker_odes.contains(&idx) {
                    let duty_rid = self
                        .find_ode_feature_decl(ode, "duty", &bare)
                        .map(RuntimeId::top_level)
                        .unwrap_or_else(|| {
                            RuntimeId::top_level(ElementId::from_string(format!(
                                "ode-signal:{}:duty",
                                ode.name.as_deref().unwrap_or("ode")
                            )))
                        });
                    Self::mint_ode_signal_slot(
                        &mut store,
                        duty_rid,
                        "duty",
                        "duty",
                        WriterId::Orchestrator,
                    );
                    if let (Some(ode_name), Some(id)) =
                        (ode.name.as_deref(), store.slot_by_name("duty"))
                    {
                        mint_qualified_alias(&mut store, ode_name, "duty", id);
                    }
                }
            }
        }

        // 2d. Discrete state-space state vectors. Same contract as the
        // top-level ODE arm above: one `Continuous` slot per state variable,
        // owned by the registering executor, so the solver's per-tick
        // writeback routes by `SlotId` instead of resolving to nothing.
        //
        // These were never minted. `add_discrete` registered the subsystem,
        // `prepare_slot_writeback` built a write-set of unrouted strict routes,
        // and the first tick panicked in `WriteRoute::apply` — the assertion
        // that says "statically impossible after the RS005 gate", which was
        // true only because `DiscreteStateSolver` did not report its unrouted
        // writes to that gate. Both halves are fixed together; neither alone
        // is enough (reporting without minting turns the panic into a build
        // error, minting without reporting leaves the next gap silent).
        for discrete in inputs.discrete_detections {
            let Some(writer) = discrete.subsystem_index.map(WriterId::from) else {
                return Err(CompileError::from_message(format!(
                    "internal: discrete detection '{}' has no registered subsystem \
                     at slot-mint time",
                    discrete.label
                )));
            };
            for (i, var) in discrete.state_vars.iter().enumerate() {
                let Some(decl) = discrete
                    .scope_ids
                    .iter()
                    .find_map(|scope| self.find_feature_decl(scope, var))
                    .or_else(|| bare.get(var).map(|(id, _)| id.clone()))
                else {
                    continue;
                };
                let init = discrete
                    .initial_values
                    .get(i)
                    .copied()
                    .map(Value::Float)
                    .unwrap_or(Value::Null);
                let id = store.intern(
                    SlotMeta::new(
                        RuntimeId::top_level(decl),
                        Variability::Continuous,
                        writer,
                        var.as_str(),
                        var.as_str(),
                    ),
                    init,
                );
                // Same qualified spelling every other top-level subsystem
                // output gets, so `<Subsystem>.<var>` overrides resolve.
                mint_qualified_alias(&mut store, &discrete.label, var, id);
            }
        }

        // 3. Instance-scoped ODE variables: one slot per (instance, var)
        // with instance_path = the usage-ElementId chain and both name
        // forms (canonical tree path + legacy var_prefix form).
        for inst in inputs.instances {
            for ode in &inst.ode_detections {
                let (canonical_prefix, sub_path) = self.instance_canonical_prefix(inst, ode);
                let mut instance_path: SmallVec<[ElementId; 4]> = SmallVec::new();
                if let Some(usage) = &inst.usage_id {
                    instance_path.push(usage.clone());
                }
                for (_, id) in &sub_path {
                    instance_path.push(id.clone());
                }

                // Instance ODE subsystems register as `{prefix}.{ode_name}`
                // (see `build_workspace_orchestrator` step 4). The
                // SubsystemIndex is captured directly on this per-instance
                // detection at its own registration call site (RSC-4.2 L40).
                let Some(ode_writer) = ode.subsystem_index.map(WriterId::from) else {
                    return Err(CompileError::from_message(format!(
                        "internal: instance ODE detection '{}.{}' has no registered \
                         subsystem at slot-mint time (RSC-4.2 mint-gap)",
                        inst.prefix,
                        ode.name.as_deref().unwrap_or("ode")
                    )));
                };
                for (i, var) in ode.state_vars.iter().enumerate() {
                    let Some(decl) = self.find_ode_feature_decl(ode, var, &bare) else {
                        continue;
                    };
                    let runtime_name = format!("{}.{}", inst.prefix, var);
                    let canonical_name = format!("{canonical_prefix}.{var}");
                    let init = ode
                        .initial_values
                        .get(i)
                        .copied()
                        .map(Value::Float)
                        .unwrap_or(Value::Null);
                    store.intern(
                        SlotMeta::new(
                            RuntimeId {
                                declaration: decl,
                                instance_path: instance_path.clone(),
                            },
                            Variability::Continuous,
                            ode_writer,
                            canonical_name,
                            runtime_name,
                        ),
                        init,
                    );
                }
                for (k, v) in &ode.parameters {
                    let Some(decl) = self.find_ode_feature_decl(ode, k, &bare) else {
                        continue;
                    };
                    let runtime_name = format!("{}.{}", inst.prefix, k);
                    let canonical_name = format!("{canonical_prefix}.{k}");
                    let val = inputs
                        .override_map
                        .get(runtime_name.as_str())
                        .or_else(|| inputs.override_map.get(k.as_str()))
                        .copied()
                        .unwrap_or(*v);
                    store.intern(
                        SlotMeta::new(
                            RuntimeId {
                                declaration: decl,
                                instance_path: instance_path.clone(),
                            },
                            Variability::Parameter,
                            WriterId::External,
                            canonical_name,
                            runtime_name,
                        ),
                        Value::Float(val),
                    );
                }
                // RSC-3.0 category 2: per-instance GetOutput signal outputs.
                // Same promote-or-mint as the top-level pass, with instance-
                // scoped name forms + instance_path identity.
                let mut sig_names: Vec<&String> = ode.signal_exprs.keys().collect();
                sig_names.sort();
                for sig in sig_names {
                    let runtime_name = format!("{}.{}", inst.prefix, sig);
                    let canonical_name = format!("{canonical_prefix}.{sig}");
                    let rid = RuntimeId {
                        declaration: self
                            .find_ode_feature_decl(ode, sig, &bare)
                            .unwrap_or_else(|| {
                                ElementId::from_string(format!("ode-signal:{}", runtime_name))
                            }),
                        instance_path: instance_path.clone(),
                    };
                    Self::mint_ode_signal_slot(
                        &mut store,
                        rid,
                        &canonical_name,
                        &runtime_name,
                        ode_writer,
                    );
                }
            }
        }

        // 3b. RSC-3.4 / L32: per-instance config attribute slots. These are
        // declared as attribute usages on the ODE type and have compile-time
        // defaults stored in `config_entries`; they belong to the External
        // writer (user can set them at runtime via override_map).
        for inst in inputs.instances {
            for (config_key, default_val) in &inst.config_entries {
                let runtime_name = format!("{}.{}", inst.prefix, config_key);
                let canonical_name = match inst.container_name.as_deref() {
                    Some(container) => format!("{container}.{runtime_name}"),
                    None => runtime_name.clone(),
                };
                // Skip if already minted by another step (e.g. ODE parameters
                // may overlap with config attributes in some models).
                if store.slot_by_name(&runtime_name).is_some() {
                    continue;
                }
                let val = inputs
                    .override_map
                    .get(runtime_name.as_str())
                    .or_else(|| inputs.override_map.get(config_key.as_str()))
                    .copied()
                    .unwrap_or(*default_val);
                let mut instance_path: smallvec::SmallVec<[ElementId; 4]> =
                    smallvec::SmallVec::new();
                if let Some(usage) = &inst.usage_id {
                    instance_path.push(usage.clone());
                }
                let rid = RuntimeId {
                    declaration: ElementId::from_string(format!("rsc34-config:{runtime_name}")),
                    instance_path,
                };
                store.intern(
                    SlotMeta::new(
                        rid,
                        Variability::Parameter,
                        WriterId::External,
                        canonical_name,
                        runtime_name,
                    ),
                    Value::Float(val),
                );
            }
        }

        // 4. Computed/gated expression targets → Continuous/Orchestrator.
        for target in inputs.computed_targets {
            if let Some(existing) = store.slot_by_name(target) {
                // A computed expression is the tick-time producer of its
                // target. If a prior pass minted the slot as a non-tick-time
                // Parameter/External (a defaulted attribute also collected as
                // an ODE parameter), PROMOTE ownership to the Orchestrator
                // (Continuous) so the computed write routes by SlotId instead
                // of refusing on the External-owner mismatch. A slot already
                // owned by a real executor (e.g. an ODE state var) is a genuine
                // last-write-wins collision — ownership is untouched and the
                // claim feeds RS001 (design doc §2.4).
                store.claim_writer_promoting(
                    existing,
                    WriterId::Orchestrator,
                    Variability::Continuous,
                );
                continue;
            }
            if let Some((prefix, attr)) = target.split_once('.') {
                // Instance-scoped `{prefix}.{attr}` target: the attribute
                // is declared directly on the instance's type definition
                // (mirrors detect_instance_scoped_expressions). Targets
                // whose prefix is not a discovered instance (port-shaped
                // keys, superset cache entries) stay out of the table.
                let Some(inst) = inputs.instances.iter().find(|i| i.prefix == prefix) else {
                    continue;
                };
                let Some(type_id) = inst.type_def_id.as_ref() else {
                    continue;
                };
                let Some(decl) = self
                    .graph
                    .children_of(type_id)
                    .find(|c| {
                        c.kind == ElementKind::AttributeUsage && c.name.as_deref() == Some(attr)
                    })
                    .map(|c| c.id.clone())
                else {
                    continue;
                };
                let mut instance_path: SmallVec<[ElementId; 4]> = SmallVec::new();
                if let Some(usage) = &inst.usage_id {
                    instance_path.push(usage.clone());
                }
                let canonical_name = match inst.container_name.as_deref() {
                    Some(container) => format!("{container}.{prefix}.{attr}"),
                    None => target.clone(),
                };
                store.intern(
                    SlotMeta::new(
                        RuntimeId {
                            declaration: decl,
                            instance_path,
                        },
                        Variability::Continuous,
                        WriterId::Orchestrator,
                        canonical_name,
                        target.as_str(),
                    ),
                    Value::Null,
                );
            } else {
                let Some((decl, bound)) = bare.get(target.as_str()) else {
                    continue;
                };
                // Computed bindings appear in the bare walk as Value::Ref
                // (expression, no literal) — the value materializes per
                // tick, so the slot starts Null.
                let init = if matches!(bound, Value::Ref(_)) {
                    Value::Null
                } else {
                    bound.clone()
                };
                store.intern(
                    SlotMeta::new(
                        RuntimeId::top_level(decl.clone()),
                        Variability::Continuous,
                        WriterId::Orchestrator,
                        target.as_str(),
                        target.as_str(),
                    ),
                    init,
                );
            }
        }

        // 5. SM assignment targets → Discrete / Executor(owning SM).
        for sm_target in inputs.sm_targets {
            let writer: WriterId = sm_target.subsystem.into();
            if let Some(existing) = store.slot_by_name(&sm_target.runtime_name) {
                // The SM targets a slot another pass already minted. If that
                // pass classified it as a non-tick-time Parameter/External
                // (e.g. `V_applied` — a defaulted attribute collected as an
                // ODE parameter in step 2 because the RHS reads it), the SM is
                // its true tick-time owner: PROMOTE ownership so
                // `WriteRoute::resolve` routes the SM's write by SlotId instead
                // of refusing on the External-owner mismatch. If the slot is
                // already owned by a real executor (two SMs, or SM over an ODE
                // state var), ownership is untouched and the claim feeds RS001.
                store.claim_writer_promoting(
                    existing,
                    writer,
                    crate::slots::Variability::Discrete,
                );
                mint_primary_sm_alias(&mut store, inputs.primary_sm_names, sm_target, existing)?;
                continue;
            }
            let inst = sm_target.instance.and_then(|i| inputs.instances.get(i));
            let decl = match inst {
                Some(inst) => inst
                    .type_def_id
                    .as_ref()
                    .and_then(|tid| self.find_feature_decl(tid, &sm_target.bare_name)),
                None => self.find_unique_named_attribute(&sm_target.bare_name),
            };
            let Some(decl) = decl else { continue };
            let mut instance_path: SmallVec<[ElementId; 4]> = SmallVec::new();
            if let Some(usage) = inst.and_then(|i| i.usage_id.as_ref()) {
                instance_path.push(usage.clone());
            }
            // Canonical tree path for instance-scoped targets: the instance
            // tree gives `{container}.{instance}.{var}` (the SM has no
            // ODE-style sub-part chain — its target attribute is declared
            // on the instance's type). Top-level targets have no longer
            // path; canonical == runtime there.
            let canonical_name = match inst.and_then(|i| i.container_name.as_deref()) {
                Some(container) => {
                    format!(
                        "{container}.{}.{}",
                        inst.map(|i| i.prefix.as_str()).unwrap_or_default(),
                        sm_target.bare_name
                    )
                }
                None => sm_target.runtime_name.clone(),
            };
            let init = bare
                .get(&sm_target.bare_name)
                .map(|(_, v)| v.clone())
                .filter(|v| !matches!(v, Value::Ref(_)))
                .unwrap_or(Value::Null);
            let id = store.intern(
                SlotMeta::new(
                    RuntimeId {
                        declaration: decl,
                        instance_path,
                    },
                    Variability::Discrete,
                    writer,
                    canonical_name,
                    sm_target.runtime_name.as_str(),
                ),
                init,
            );
            mint_primary_sm_alias(&mut store, inputs.primary_sm_names, sm_target, id)?;
        }

        // 5b. RSC-3.5b: SM port-payload slots. The SET of ports a SM can
        // receive a payload on is compile-static (`accept_ports()`); only the
        // delivered VALUE is runtime-dynamic — which is exactly what a slot
        // stores. Mint a Discrete slot per `{port}.payload` / `{port}_payload`
        // key, owned (writer) by the receiving SM executor. The SM's
        // `prepare_slot_writeback` then resolves a WriteRoute per key by SlotId
        // (draining the `dynamic_keys` name-keyed fallback class — design doc
        // D-3.0.5 "SM payload bindings become slots"). The declaration id is a
        // deterministic synthetic keyed off the runtime name (the payload is a
        // transient transfer binding, not a declared attribute usage).
        for pp in inputs.sm_payload_ports {
            let writer: WriterId = pp.subsystem.into();
            let mut instance_path: SmallVec<[ElementId; 4]> = SmallVec::new();
            if let Some(usage) = pp
                .instance
                .and_then(|i| inputs.instances.get(i))
                .and_then(|inst| inst.usage_id.as_ref())
            {
                instance_path.push(usage.clone());
            }
            // Both spellings the SM writeback can produce for this port
            // (`.payload` dot form + `_payload` underscore form) intern to
            // distinct slots — the SM routes each independently.
            for (suffix, runtime, canonical) in [
                (
                    "payload",
                    format!("{}.payload", pp.runtime_base),
                    format!("{}.payload", pp.canonical_base),
                ),
                (
                    "_payload",
                    format!("{}_payload", pp.runtime_base),
                    format!("{}_payload", pp.canonical_base),
                ),
            ] {
                if store.slot_by_name(&runtime).is_some() {
                    // Already minted (collision with an assignment target or a
                    // prior port). The receiving SM is the tick-time owner of
                    // its payload slot: promote away from a non-tick-time
                    // Parameter/External mint so the write routes by SlotId;
                    // a real executor owner is untouched and the claim surfaces
                    // any genuine multi-writer conflict via RS001.
                    if let Some(existing) = store.slot_by_name(&runtime) {
                        store.claim_writer_promoting(existing, writer, Variability::Discrete);
                    }
                    continue;
                }
                let rid = RuntimeId {
                    declaration: ElementId::from_string(format!(
                        "rsc35b-payload:{}.{}.{suffix}",
                        pp.subsystem.index(),
                        pp.port
                    )),
                    instance_path: instance_path.clone(),
                };
                store.intern(
                    SlotMeta::new(rid, Variability::Discrete, writer, canonical, runtime),
                    Value::Null,
                );
            }
        }

        // 6. Remaining bare bindings: literal defaults are attributes
        // written by nothing at tick time → Parameter/External.
        // `Value::Ref` bindings are lazily-resolved expression defaults
        // (expression provenance: the ref points at the declaring element
        // whose value materializes through the orchestrator's snapshot
        // ref-resolution, not through any executor) → Continuous /
        // Orchestrator, seeded with the same Ref the legacy map carries.
        //
        // EXCEPTION — a bare binding that is ALSO a top-level SM assignment
        // target (a plain `attribute` written by an SM entry/transition action,
        // e.g. `boilerTemp = 20`) has a real tick-time writer: the SM. Step 5's
        // promotion could not claim it — the slot did not exist there (bare
        // bindings mint LAST) and its declaration is not unique across the
        // model (`find_unique_named_attribute` returns None), so step 5 skipped
        // it and it would otherwise fall here as `External`. Mint it owned by
        // the SM's `Executor` writer (Discrete) so `WriteRoute::resolve` routes
        // the SM's write by `SlotId` instead of hard-erroring on the deleted
        // name-keyed path. Same single-writer promotion species as `V_applied`.
        // A target written by two SMs still surfaces via step 5's multi-writer
        // path (RS001); here each bare name has exactly one top-level SM writer.
        let sm_target_writer: HashMap<&str, WriterId> = inputs
            .sm_targets
            .iter()
            .filter(|t| t.instance.is_none())
            .map(|t| (t.runtime_name.as_str(), WriterId::from(t.subsystem)))
            .collect();
        for (name, (decl, value)) in &bare {
            if store.slot_by_name(name).is_some() {
                continue;
            }
            let (variability, writer) = if let Some(&sm_writer) =
                sm_target_writer.get(name.as_str())
            {
                (Variability::Discrete, sm_writer)
            } else if matches!(value, Value::Ref(_)) {
                (Variability::Continuous, WriterId::Orchestrator)
            } else {
                (Variability::Parameter, WriterId::External)
            };
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(decl.clone()),
                    variability,
                    writer,
                    name.as_str(),
                    name.as_str(),
                ),
                value.clone(),
            );
        }

        // 6c. RSC-3.6: per-instance guard/trigger read-leaf slots for prefixed
        // (instance-multiplied) state machines. A SM-only instance gets no
        // slots from the ODE/assignment/computed steps, so its guard's reads
        // (`currentIn.reading.value`, `threshold`) resolve only through the
        // shared global context — leaving the executor scoped-view-bypass-
        // INELIGIBLE (`statemachine::scoped_bypass_eligible`). An instance-local
        // slot per read leaf lets the guard bind to an instance-local `SlotRef`
        // (the read is then provably servable from the instance's own slots —
        // the RSC-3.5f.3 prerequisite). Seed from the bare binding when the leaf
        // is a bare literal/Ref default (so `threshold` mirrors its `Float(5.0)`
        // Parameter/External bare slot); else synthetic decl + `Null`,
        // Continuous/Orchestrator — a signal-port-feature chain with no delivery
        // (`Null > threshold` errors → guard event-name fallback → false,
        // exactly the legacy chain-resolution verdict; byte-identical, §1).
        // Skip-if-present so an assignment-target / ODE / computed slot of the
        // same instance key is never clobbered.
        for gr in inputs.sm_guard_reads {
            if store.slot_by_name(&gr.runtime_name).is_some() {
                continue;
            }
            let mut instance_path: SmallVec<[ElementId; 4]> = SmallVec::new();
            if let Some(usage) = gr
                .instance
                .and_then(|i| inputs.instances.get(i))
                .and_then(|i| i.usage_id.as_ref())
            {
                instance_path.push(usage.clone());
            }
            let (decl, init, variability, writer) = match bare.get(gr.leaf.as_str()) {
                Some((decl, value @ Value::Ref(_))) => (
                    decl.clone(),
                    value.clone(),
                    Variability::Continuous,
                    WriterId::Orchestrator,
                ),
                Some((decl, value)) => (
                    decl.clone(),
                    value.clone(),
                    Variability::Parameter,
                    WriterId::External,
                ),
                None => (
                    ElementId::from_string(format!("rsc36-guard-read:{}", gr.runtime_name)),
                    Value::Null,
                    Variability::Continuous,
                    WriterId::Orchestrator,
                ),
            };
            store.intern(
                SlotMeta::new(
                    RuntimeId {
                        declaration: decl,
                        instance_path,
                    },
                    variability,
                    writer,
                    gr.canonical_name.as_str(),
                    gr.runtime_name.as_str(),
                ),
                init,
            );
        }

        // 7. RSC-3.2: signal-link endpoint port-feature slots (design doc
        // D-3.0.3). For BOTH endpoints of every classified SignalLink, mint a
        // Continuous slot per port feature spelled `"{owner}.{port}.{feature}"`
        // — exactly today's port_values / context keys — written by the
        // Orchestrator (the link plane). The slot table grows; nothing else
        // about minting changes (these names are inert on the legacy map until
        // a write routes through them, which already happens in
        // `propagate_port_values` phases 1/3). The directed-propagation pass is
        // compiled separately in `compile_signal_propagation` once the store
        // is live.
        if let (Some(link_graph), Some(registry)) = (inputs.link_graph, inputs.port_registry) {
            self.mint_signal_feature_slots(&mut store, link_graph, registry);
        }

        // 8. RSC-3.4 / L31: physics node variables minted as Continuous slots with
        // writer = physics executor. Keys are derived from `collect_physics_write_targets`
        // (seeded node features, equality targets, conservation incoming flows,
        // constitutive effort/flow variables, DAE state vector names). Pre-minting
        // before `prepare_slot_writeback` runs lets that method use `resolve` (hard
        // assert) instead of `resolve_quiet` — a missing slot becomes a loud failure
        // during development rather than a silent fallback to the name-keyed path.
        if let (Some(targets), Some(writer)) = (inputs.physics_write_targets, inputs.physics_writer)
        {
            for target in targets {
                if store.slot_by_name(target).is_some() {
                    // Already minted by a previous step. The physics executor
                    // is the tick-time (Continuous) owner of its node
                    // variables: promote away from a non-tick-time
                    // Parameter/External mint so the write routes by SlotId. A
                    // real executor owner (e.g. an ODE state-var name collision)
                    // is untouched and the claim surfaces the conflict via RS001.
                    let existing_id = store.slot_by_name(target).unwrap();
                    store.claim_writer_promoting(existing_id, writer, Variability::Continuous);
                    continue;
                }
                let rid =
                    RuntimeId::top_level(ElementId::from_string(format!("rsc34-physics:{target}")));
                store.intern(
                    SlotMeta::new(
                        rid,
                        Variability::Continuous,
                        writer,
                        target.as_str(),
                        target.as_str(),
                    ),
                    Value::Float(0.0),
                );
            }
        }

        // 9. RSC-3.7 C.1: SampledFunction lookup tables minted as global
        // `Parameter` slots. Today these are injected into the master context
        // under the legacy magic key `__sf_{name}` (step 2c of
        // `build_workspace_orchestrator` / step 6b of the prepared single-ODE
        // path); the reads spell them the same way — the CoreODE model literally
        // (`interpolateSaturating(__sf_HfromB_ascending, B)`) and the scenario
        // waveform wiring synthetically (`wire_scenario_waveforms` generates
        // `interpolateLinear(__sf_ref, __clock_time)`). The slot is keyed by that
        // `__sf_{name}` runtime spelling so the existing `bind_slots` passes
        // (orchestrator scope + ODE subsystem scope) rewrite the
        // `FeatureRef("__sf_…")` read to a `SlotRef` with zero new binding code,
        // and the read resolves through the slot store in every context. The
        // declared-feature-name alias (`HfromB_ascending`, what the model reads
        // once the `__sf_` prefix is dropped) is a C.3 concern and is added
        // there, guarded against colliding with an unrelated same-named slot.
        // These tables are read-only injected data, never written at tick time →
        // `WriterId::External`, like ODE parameters. Extraction already succeeded
        // upstream (the `?` at step 2c / `prepare_single_ode`), so the re-walk
        // here cannot newly fail. NB: the SampledFunction's *declared* name may
        // already back an unrelated context key / slot (golden's
        // `lightingWaveform` is a context value that is NOT the lookup table), so
        // the slot uses ONLY the `__sf_` spelling for BOTH names — giving it the
        // declared name as `canonical` would let `seed_from_map` overwrite the
        // lookup-table payload with that unrelated context value at handle-attach
        // time. The declared-name resolution the C.3 model edit needs is added
        // per-SF, guarded against exactly that collision (see below).
        for (sf_name, sf_value) in self.extract_sampled_functions().unwrap_or_default() {
            let runtime_key = format!("__sf_{sf_name}");
            if store.slot_by_name(&runtime_key).is_some() {
                continue;
            }
            let rid = RuntimeId::top_level(ElementId::from_string(format!(
                "sysml-runtime:sampled-function:{sf_name}"
            )));
            let id = store.intern(
                SlotMeta::new(
                    rid,
                    Variability::Parameter,
                    WriterId::External,
                    runtime_key.as_str(),
                    runtime_key.as_str(),
                ),
                sf_value,
            );
            // RSC-3.7 C.3: also resolve the slot by the SampledFunction's
            // DECLARED name (`HfromB_ascending`) so a model that reads the
            // feature directly (`interpolate(HfromB_ascending, B)`, the spec
            // form) binds to it. Added as a plain alias — NOT a canonical/runtime
            // name — so `seed_from_map` cannot overwrite the lookup-table payload
            // with an unrelated same-named context value at handle-attach time.
            // Guarded: skip if the declared name already resolves to a different
            // slot (first-wins `add_alias` would no-op anyway; the guard makes
            // the intent explicit and avoids a misleading alias).
            if store.slot_by_name(&sf_name).is_none() {
                store.add_alias(sf_name, id);
            }
        }

        // RSC-5.1 (D-5.0.3): attach measurement references to ISQ-typed slots as
        // pure metadata. A single post-pass over the minted store — far cleaner
        // than threading inference through ~15 scattered intern sites, and the
        // slot plane (compile-time mint) is the "one home" for mRef. Nothing
        // reads m_ref yet, so this is byte-identical; it establishes the slot
        // plane as the mRef authority. The `maybe_tag_isq` reconciliation (M2)
        // and the explicit `[unit]` inference path (#1, parser-gated) land in
        // follow-up steps.
        let inferred: Vec<(crate::slots::SlotId, crate::slots::MeasurementRef)> = store
            .iter()
            .filter_map(|(id, meta, _)| {
                infer_m_ref(&self.graph, &meta.runtime_id.declaration).map(|m| (id, m))
            })
            .collect();
        for (id, m_ref) in inferred {
            store.set_m_ref(id, Some(m_ref));
        }

        Ok(store)
    }

    /// RSC-5.1 (D-5.0.3): infer a slot's [`MeasurementRef`] from its declaring
    /// element's ISQ type. Mirrors `maybe_tag_isq`'s resolution exactly (same
    /// `AttributeUsage` scope, same `resolve_attribute_type_name` + `ISQ_TYPES`
    /// match) so the slot mRef dimension is consistent with the value the eval
    /// context still tags today — the precondition for delegating/deleting
    /// `maybe_tag_isq` in the M2 follow-up.
    ///
    /// ISQ-type-only inference yields `unit: None`, `scale: 1.0`, `offset: 0.0`
    /// (the SI-base magnitude — byte-identical to today). An explicit `[unit]`
    /// annotation (inference path #1) overrides this once the parser-crate `[`
    /// fix (D-5.0.5) lands.

    /// RSC-3.2: mint Continuous slots for the port features on both endpoints
    /// of every classified [`crate::links::LinkClass::SignalLink`].
    ///
    /// Spelling is `"{owner}.{port}.{feature}"` (today's port_values/context
    /// key); writer is [`WriterId::Orchestrator`] (the signal-link plane). The
    /// declaration id is the resolved port-feature element when recoverable,
    /// else a deterministic synthetic id keyed off the slot name so the
    /// RuntimeId stays unique and stable across runs. Idempotent re-interning
    /// (a port feeding multiple signal links) returns the existing slot.
    fn mint_signal_feature_slots(
        &self,
        store: &mut SlotStore,
        link_graph: &crate::links::LinkGraph,
        registry: &crate::flows::PortRegistry,
    ) {
        use crate::slots::{RuntimeId, SlotMeta, Variability, WriterId};

        // Slots are minted on the per-instance basis (`{owner}.{port}.{feature}`),
        // but the feature *set* is read from the definition-keyed registry via the
        // endpoint's resolved registry key (ledger L36). The declaration element id
        // for each feature is resolved through the participant usage's type so the
        // slot's mRef (ISQ dimension) can be inferred from the real attribute —
        // falling back to a deterministic synthetic id when unresolvable.
        let mint_endpoint = |store: &mut SlotStore, endpoint: &crate::links::LinkEndpoint| {
            let reg_key = endpoint.registry_key();
            let Some(inst) = registry.get(&reg_key) else {
                return;
            };
            let slot_prefix = endpoint.key();
            // The element(s) whose children are this port's typed features (the
            // declaration elements we want for mRef inference, ledger L36/RSC-5.2).
            // The endpoint's stamped id resolves to the PARTICIPANT (its type is the
            // owning part def), whose child named `endpoint.port` is the port USAGE.
            // Port features are normally declared on the port DEFINITION (the usage's
            // type) — mirroring `compile_port_from_elaborated`, which reads both the
            // resolved port def's children AND the usage's own children. We search
            // both, def first, so `infer_m_ref` sees the real ISQ-typed attribute.
            use sysml_core::resolution::scoping::chaining::find_feature_type;
            let mut feature_parents: Vec<ElementId> = Vec::new();
            if let Some(port_usage_id) = endpoint.element_id.as_ref().and_then(|usage_id| {
                let part_def = find_feature_type(&self.graph, usage_id)?;
                self.graph
                    .children_of(&part_def)
                    .find(|c| c.name.as_deref() == Some(endpoint.port.as_str()))
                    .map(|p| p.id.clone())
            }) {
                if let Some(port_def) = find_feature_type(&self.graph, &port_usage_id) {
                    feature_parents.push(port_def);
                }
                feature_parents.push(port_usage_id);
            }
            // Feature-name discovery routes through the shared def-keyed bridge
            // (ledger L36, CLAUDE.md #4/#5); the per-feature value + mRef loop
            // below is signal-slot-specific and stays here.
            for feat_name in crate::links::endpoint_feature_names(registry, endpoint) {
                let Some(feat) = inst.features.get(&feat_name) else {
                    continue;
                };
                let slot_name = format!("{slot_prefix}.{feat_name}");
                // mRef (ISQ dimension) inferred from the resolved port-feature
                // element — used by RSC-5.3 boundary conversion and the UQ002
                // cross-dim check. `None` for non-ISQ features (e.g. item-typed
                // payloads), keeping those slots dimensionless as before.
                let m_ref = feature_parents
                    .iter()
                    .find_map(|pid| {
                        self.graph
                            .children_of(pid)
                            .find(|f| f.name.as_deref() == Some(feat_name.as_str()))
                            .map(|f| f.id.clone())
                    })
                    .and_then(|fid| infer_m_ref(&self.graph, &fid));
                // RuntimeId identity: a deterministic synthetic id PER instance-
                // qualified slot name. Signal slots are minted per ENDPOINT INSTANCE,
                // but the port-feature declaration element is SHARED across instances
                // (one port def, many usages) — keying the RuntimeId on it would
                // collapse `controllerA.currentIn.reading` and `controllerB...` into
                // one slot (the store interns by `RuntimeId`). The mRef rides
                // separately (above), so we keep the unique synthetic identity and
                // still carry the real dimension.
                let decl = ElementId::from_string(format!("rsc32-signal-feature:{slot_name}"));
                store.intern(
                    SlotMeta::new(
                        RuntimeId::top_level(decl),
                        Variability::Continuous,
                        WriterId::Orchestrator,
                        slot_name.as_str(),
                        slot_name.as_str(),
                    )
                    .with_m_ref(m_ref),
                    feat.value.clone(),
                );
            }
        };

        for &link_id in link_graph.ids_of_class(crate::links::LinkClass::SignalLink) {
            let Some(link) = link_graph.get(link_id) else {
                continue;
            };
            mint_endpoint(store, &link.source);
            mint_endpoint(store, &link.target);
        }
    }

    /// RS001 — `multiple runtime writers` (RSC-2.2, design doc D-2.0.3
    /// rule 1, hard error per user decision 2026-06-11): one diagnostic per
    /// slot claimed by ≥2 distinct tick-time writers, naming the variable,
    /// every claiming writer (subsystem names resolved by indexing
    /// `subsystem_names`, already index-aligned with `WriterId::Executor`
    /// — RSC-4.2 L40, display-only, never used to derive an index), and the
    /// owning declaration element. Returns `Ok(())` when the table is
    /// conflict-free.
    pub(crate) fn check_multi_writer_conflicts(
        &self,
        store: &SlotStore,
        subsystem_names: &[String],
    ) -> Result<(), CompileError> {
        use crate::slots::WriterId;

        let conflicts = store.multi_writer_conflicts();
        if conflicts.is_empty() {
            return Ok(());
        }
        let describe = |w: &WriterId| -> String {
            match w {
                WriterId::Executor(i) => match subsystem_names.get(*i as usize) {
                    Some(n) => format!("executor subsystem '{n}'"),
                    None => format!("executor #{i}"),
                },
                WriterId::Orchestrator => {
                    "the orchestrator (computed/gated expressions)".to_owned()
                }
                WriterId::External => "external (overrides)".to_owned(),
            }
        };
        let diagnostics: Vec<Diagnostic> = conflicts
            .iter()
            .map(|(id, name, writers)| {
                let writer_list = writers
                    .iter()
                    .map(describe)
                    .collect::<Vec<_>>()
                    .join(" and ");
                let mut diag = Diagnostic::error(format!(
                    "multiple runtime writers for '{name}': {writer_list} — \
                     one slot must have exactly one tick-time writer"
                ))
                .with_code("RS001");
                if let Some(meta) = store.meta(*id) {
                    let decl = &meta.runtime_id.declaration;
                    let owner_desc = self
                        .graph
                        .get_element(decl)
                        .and_then(|e| e.name.clone())
                        .map(|n| format!("'{n}' ({decl})"))
                        .unwrap_or_else(|| format!("{decl}"));
                    diag = diag.with_note(format!(
                        "declared on element {owner_desc}; runtime name '{}', canonical name '{}'",
                        meta.runtime_name, meta.canonical_name
                    ));
                }
                diag.with_note(
                    "give each writer its own attribute, or move the assignment so a single \
                     executor owns the variable",
                )
            })
            .collect();
        Err(CompileError::from_diagnostics(diagnostics))
    }

    /// RS003 — `unresolved runtime name` (RSC-2.3, upgraded to a **hard
    /// error** at RSC-2.5 per design doc D-2.0.4): one error per distinct
    /// name that the slot-binding pass could resolve to neither a slot nor
    /// any named element in the model graph, attributed to the binding
    /// scope(s) that referenced it (subsystem name or the orchestrator's
    /// own expression scope), with a nearest-spelling suggestion when a
    /// registered slot alias is within edit distance 2 (computed only on
    /// this error path — never at tick time).
    ///
    /// Names that DO match a graph feature are exempt — they legitimately
    /// resolve at eval time (Ref chains, redefinitions). Expression-AST
    /// nodes don't count as declarations. Everything else used to fall
    /// through to the eval-time graph-wide name scan; that scan is deleted
    /// (RSC-2.5), so the compile fails instead of deferring the miss.
    ///
    /// Returns `(errors, warnings)`. A name unresolved ONLY in the
    /// constraint scope ([`RS003_CONSTRAINT_SCOPE`]
    /// (crate::orchestrator::RS003_CONSTRAINT_SCOPE)) is a *warning*:
    /// constraints have pinned tick-time skip semantics for missing
    /// operands, and the service verification path re-evaluates them with
    /// injected variables (`sim_time_ms`, `sim_completed`, analysis
    /// parameters) no orchestrator slot can know about. Names missed by
    /// any other scope (computed expressions, executor-retained
    /// expressions) fail the compile — the tick loop WILL evaluate those
    /// with no scan to save them.
    pub(crate) fn rs003_diagnostics(&self, orchestrator: &Orchestrator) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
        use std::collections::BTreeMap;

        let mut scopes_by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (scope, names) in orchestrator.bind_unresolved_scopes() {
            for name in names {
                let entry = scopes_by_name.entry(name.as_str()).or_default();
                if !entry.contains(&scope.as_str()) {
                    entry.push(scope.as_str());
                }
            }
        }
        if scopes_by_name.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let store = orchestrator.slot_store();
        let (mut errors, mut warnings) = (Vec::new(), Vec::new());
        for (name, scopes) in scopes_by_name {
            // A master-context variable by this name already exists at
            // compile time (orchestrator-injected globals: `__sf_*`
            // waveform keys, seeded signal values) → resolvable at eval
            // time in every scope (unprefixed globals copy through scoped
            // views). The global binder declares these as locals; subsystem
            // binders don't see them, so exempt them here.
            if orchestrator.context.get(name).is_some() {
                continue;
            }
            // A graph feature by this name exists → legitimately
            // resolvable at eval time (Ref chains, redefinitions);
            // not an RS003 candidate. Expression-AST nodes don't count
            // — the reference expression itself carrying the name is
            // not a declaration of it.
            if self
                .graph
                .elements
                .values()
                .any(|e| !is_expression_ast_kind(&e.kind) && e.name.as_deref() == Some(name))
            {
                continue;
            }
            let constraint_only = scopes
                .iter()
                .all(|s| *s == crate::orchestrator::RS003_CONSTRAINT_SCOPE);
            let message = format!(
                "unresolved runtime name '{name}' (referenced by {}): resolves to \
                 neither a runtime variable slot nor a model feature",
                scopes.join(", ")
            );
            let mut diag = if constraint_only {
                Diagnostic::warning(format!(
                    "{message}; the constraint evaluates as skipped unless the name \
                     is provided at evaluation time (e.g. service-injected \
                     verification variables)"
                ))
            } else {
                Diagnostic::error(message)
            }
            .with_code("RS003");
            if let Some(suggestion) = nearest_slot_spelling(&store, name) {
                diag = diag.with_note(format!("nearest known runtime name: '{suggestion}'"));
            }
            if constraint_only {
                warnings.push(diag);
            } else {
                errors.push(diag);
            }
        }
        (errors, warnings)
    }

    /// RS004 — `prefixed subsystem not scoped-view-bypass-eligible` (RSC-3.5f.3,
    /// the build-time gate that replaced `build_scoped_context`). Every
    /// instance-multiplied (prefixed) subsystem MUST read through the thin
    /// slot view: its guards / triggers / structured-action reads have to bind
    /// to instance-local slots so no per-prefix variable-map clone is needed.
    /// The legacy scoped-context clone that used to serve ineligible prefixed
    /// subsystems is deleted, so an ineligible one can no longer run — it is a
    /// hard compile error here rather than a tick-time `unreachable!`.
    /// [`scoped_view_fallbacks`](Orchestrator::scoped_view_fallbacks) names each
    /// offending `(subsystem, phase)`; empty means every prefixed subsystem
    /// bypasses — the corpus-wide invariant the bypass census pins.
    pub(crate) fn rs004_diagnostics(&self, orchestrator: &Orchestrator) -> Vec<Diagnostic> {
        orchestrator
            .scoped_view_fallbacks()
            .into_iter()
            .map(|(name, phase)| {
                Diagnostic::error(format!(
                    "prefixed subsystem '{name}' ({phase:?}) is not scoped-view-\
                     bypass-eligible: its instance-local reads do not all bind to \
                     slots, so it cannot run on the slot-backed path (the legacy \
                     scoped-context clone was removed in RSC-3.5f.3)"
                ))
                .with_code("RS004")
                .with_note(
                    "make every guard / trigger / structured-action read resolve to \
                     an instance-local slot (see \
                     StateMachineRunner::scoped_bypass_eligible), or remove the \
                     construct that blocks eligibility (parallel regions, composite \
                     states, unbound guards, At triggers)",
                )
            })
            .collect()
    }

    /// RS005 — `strict-write mint gap` (A2, arc-review cleanup 2026-07-03).
    /// Every write on the strict [`WriteRoute::apply`]
    /// (crate::slots::WriteRoute::apply) path — ODE state vectors, ODE signal
    /// outputs (RSC-3.0 category 2), state-machine assignment targets, hybrid
    /// continuous state — MUST resolve to a claimed slot. The strict `apply`
    /// treats an unrouted route as a **statically impossible** invariant
    /// violation: it fires a `debug_assert` in debug builds and, in a default
    /// release build (no `tracing` feature), silently drops the write. That
    /// silent drop is exactly the principle-1 failure the slot plane exists to
    /// prevent, so — like RS001/RS003/RS004 — we detect it ONCE at compile and
    /// hard-fail the build, rather than shipping a per-tick guard for a fully
    /// static defect ([`unrouted_slot_writes`]
    /// (crate::orchestrator::Orchestrator::unrouted_slot_writes) is the census).
    ///
    /// Narrower than the observability census `sm_slot_fallbacks` etc.: it does
    /// NOT flag the name-keyed [`apply_name_keyed`]
    /// (crate::slots::WriteRoute::apply_name_keyed) path (physics port/flow
    /// writes, SM port payloads — L34-gated) or runtime-dynamic keys
    /// (`__clock_time`, port bindings), which are not mint gaps. Empty
    /// corpus-wide today, so this closes the gap with zero known live misses.
    pub(crate) fn rs005_diagnostics(&self, orchestrator: &Orchestrator) -> Vec<Diagnostic> {
        orchestrator
            .unrouted_slot_writes()
            .into_iter()
            .map(|(subsystem, keys)| {
                Diagnostic::error(format!(
                    "strict-write mint gap in subsystem '{subsystem}': the claimed \
                     write target(s) [{}] resolved to no slot, so the strict \
                     WriteRoute::apply would silently drop them in a release build",
                    keys.join(", ")
                ))
                .with_code("RS005")
                .with_note(
                    "every state vector / ODE signal output / SM assignment target / \
                     hybrid continuous-state write must mint a claimed slot. This is a \
                     mint gap (e.g. an ambiguous or unfindable attribute declaration \
                     that mint_slot_store skipped), not a name-keyed physics/payload \
                     write — those legitimately use apply_name_keyed",
                )
            })
            .collect()
    }

}
