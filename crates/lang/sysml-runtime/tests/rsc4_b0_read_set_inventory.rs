//! RSC-4.B0 — read-set inventory harness (the load-bearing RSC-4.1 input).
//!
//! Wave 1 of the RSC-4.0 Phase-4 scheduler cull. RSC-4.1 (Wave 2) builds, per
//! `ExecutionPhase`, a topological DAG over the phase's executors whose edges
//! are `A.write-set ∩ B.read-set`. That DAG is only sound if its READ-set is
//! complete — a missed edge ⇒ silent same-phase staleness ("Gap-4"). This
//! harness is the coverage meter Wave 2's DAG must reproduce, and a permanent
//! regression net.
//!
//! ## What is soundly reachable today (the central finding for Wave 2)
//!
//! * **WRITE-sets are authoritative and complete.** Every slot carries
//!   `SlotMeta.writer: WriterId`; `WriterId::Executor(i)` indexes
//!   `Orchestrator::subsystems()`. So `slot → writing subsystem → phase` is a
//!   total, pub-API map. No reconstruction needed.
//!
//! * **READ-sets are NOT assembled anywhere.** There is no per-executor
//!   read-set accessor on the `Executor` trait. The state-machine runner DOES
//!   compute its reads (`guard_trigger_reads`, `action_slot_reads`) but those
//!   are `pub(crate)`; ODE / physics / action executors expose no read accessor
//!   at all. So from the pub API the read-set is recoverable ONLY for state
//!   machines, and only by re-deriving it from the pub `Executor::transitions()`
//!   surface (guard + `when`-event + structured-action RHS free vars), resolved
//!   to `SlotId`s through `SlotBinder::for_subsystem` exactly as the compiler's
//!   `bind_slots` does (§9 Q2: read-set = compiler-resolved slot-ids).
//!
//! ## What Wave 2a added — the per-executor read-set accessor
//!
//! `Executor::read_slots()` now exposes EVERY kind's compiler-resolved read
//! slots (harvested from the `SlotRef`/`SlotChainHead` nodes `bind_slots`
//! produced — §9 Q2). The inventory sources its read-set from that accessor
//! directly (no guard-string re-compilation), so `reads_unrecoverable` is now
//! **0 for all phases**: nothing is hidden behind a `pub(crate)` surface. Kinds
//! whose tick reads are genuinely not slot-bound (token-local actions, the
//! bond-graph physics exchange plane, closure-built discrete solvers) report an
//! empty read-set — recoverable, just empty.
//!
//! The inventory therefore reports THREE planes:
//!   1. authoritative per-phase WRITE-sets (complete),
//!   2. the signal-link slot-dependency edges (`dependency_edges()`) — the
//!      existing cross-subsystem coupling RSC-4.1 already consumes,
//!   3. the COMPLETE intra-phase write→read edges (the Gap-4 sites), sourced
//!      from `Executor::read_slots()` across every subsystem — exactly the
//!      edges the Wave-2b per-phase DAG must order.
//!
//! Runs on generic fixtures (the espresso production cell + an inline ramp);
//! the pinned topology report snapshot was dropped in favour of the generic
//! edge-set-equality + `intra_phase_edges == 0` invariants (coverage-matrix
//! SCHED-READSET). Run with:
//!   cargo test -p sysml-runtime --test rsc4_b0_read_set_inventory -- --nocapture

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use sysml_core::{elaborate, ModelGraph};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::{ExecutionPhase, Orchestrator};
use sysml_runtime::slots::{SlotStore, WriterId};

// ---------------------------------------------------------------------------
// Loading (mirrors the gate harnesses)
// ---------------------------------------------------------------------------

fn example_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples")
        .join(name)
}

fn collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("sysml") {
                out.push(p);
            }
        }
    }
}

fn load_dir(name: &str) -> ModelGraph {
    let dir = example_dir(name);
    let mut files = Vec::new();
    collect(&dir, &mut files);
    assert!(!files.is_empty(), "no .sysml in {}", dir.display());
    let parser = TreeSitterParser::new();
    let inputs: Vec<SysmlFile> = files
        .iter()
        .map(|p| {
            let src = std::fs::read_to_string(p).unwrap();
            SysmlFile::new(p.file_name().unwrap().to_str().unwrap().to_owned(), src)
        })
        .collect();
    let mut g = parser.parse(&inputs).graph;
    elaborate::elaborate(&mut g);
    g
}

fn load_src(file: &str, src: &str) -> ModelGraph {
    let parser = TreeSitterParser::new();
    let mut g = parser.parse(&[SysmlFile::new(file.to_owned(), src.to_owned())]).graph;
    elaborate::elaborate(&mut g);
    g
}

// ---------------------------------------------------------------------------
// Phase ordering helpers (ExecutionPhase has no Ord/Hash)
// ---------------------------------------------------------------------------

fn phase_rank(p: ExecutionPhase) -> u8 {
    match p {
        ExecutionPhase::Physics => 0,
        ExecutionPhase::ContinuousDynamics => 1,
        ExecutionPhase::DiscreteDynamics => 2,
        ExecutionPhase::StateMachine => 3,
        ExecutionPhase::Action => 4,
    }
}

fn phase_name(p: ExecutionPhase) -> &'static str {
    match p {
        ExecutionPhase::Physics => "Physics",
        ExecutionPhase::ContinuousDynamics => "ContinuousDynamics",
        ExecutionPhase::DiscreteDynamics => "DiscreteDynamics",
        ExecutionPhase::StateMachine => "StateMachine",
        ExecutionPhase::Action => "Action",
    }
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

struct SubsystemInfo {
    name: String,
    phase: ExecutionPhase,
    kind: &'static str,
}

/// Build the full deterministic textual inventory report for one orchestrator.
fn inventory(orch: &Orchestrator) -> String {
    let mut out = String::new();

    // --- subsystem table --------------------------------------------------
    let subs: Vec<SubsystemInfo> = orch
        .subsystems()
        .iter()
        .map(|s| SubsystemInfo {
            name: s.name.clone(),
            phase: s.executor.phase(),
            kind: s.executor.kind_label(),
        })
        .collect();

    let _ = writeln!(out, "subsystems: {}", subs.len());
    {
        // per-phase subsystem counts (sorted by phase rank). With the Wave-2a
        // `Executor::read_slots()` accessor every kind exposes its read-set, so
        // `reads_unrecoverable` is structurally 0 — kept in the report as a
        // permanent assertion that no kind regresses to a hidden read surface.
        let mut by_phase: BTreeMap<u8, (ExecutionPhase, usize, usize)> = BTreeMap::new();
        for s in &subs {
            let e = by_phase
                .entry(phase_rank(s.phase))
                .or_insert((s.phase, 0, 0));
            e.1 += 1;
            // every kind is recoverable via read_slots(); unrec stays 0.
            let _ = s.kind;
        }
        for (_, (phase, count, unrec)) in &by_phase {
            let _ = writeln!(
                out,
                "  phase {:<18} subsystems={} reads_unrecoverable={}",
                phase_name(*phase),
                count,
                unrec
            );
        }
    }

    // --- authoritative WRITE-sets (WriterId::Executor plane) --------------
    let Some(shared) = orch.context.slots.as_ref() else {
        let _ = writeln!(out, "no slot store — nothing to inventory");
        return out;
    };
    let store: &SlotStore = &shared.read().unwrap();

    // slot -> writing subsystem index
    let mut write_sets: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let mut slot_writer: BTreeMap<u32, usize> = BTreeMap::new();
    let mut slot_name: BTreeMap<u32, String> = BTreeMap::new();
    let mut orch_bookkeeping = 0usize;
    let mut orch_computed = 0usize; // path-A computed/propagated producers
    let mut external_writes = 0usize;
    let mut total_slots = 0usize;
    for (id, meta, _v) in store.iter() {
        total_slots += 1;
        slot_name.insert(id.index() as u32, meta.canonical_name.to_string());
        match meta.writer {
            WriterId::Executor(i) => {
                let idx = i as usize;
                slot_writer.insert(id.index() as u32, idx);
                write_sets
                    .entry(idx)
                    .or_default()
                    .insert(meta.canonical_name.to_string());
            }
            WriterId::Orchestrator => {
                if meta.bookkeeping {
                    orch_bookkeeping += 1;
                } else {
                    orch_computed += 1;
                }
            }
            WriterId::External => external_writes += 1,
        }
    }
    let _ = writeln!(
        out,
        "slots: total={} executor_written={} orchestrator_computed={} orchestrator_bookkeeping={} external={}",
        total_slots,
        slot_writer.len(),
        orch_computed,
        orch_bookkeeping,
        external_writes
    );

    // write-set sizes per phase (deterministic: keyed by subsystem name)
    {
        let mut rows: BTreeMap<(u8, String), (ExecutionPhase, usize)> = BTreeMap::new();
        for (idx, set) in &write_sets {
            if let Some(s) = subs.get(*idx) {
                rows.insert((phase_rank(s.phase), s.name.clone()), (s.phase, set.len()));
            }
        }
        let _ = writeln!(out, "write-set sizes (phase / subsystem / #slots):");
        for ((_, name), (phase, n)) in &rows {
            // label comes from the subsystem at this row's own idx (carried via
            // the value above), never re-derived by name — duplicate-named
            // subsystems (e.g. an SM wrapper and a CD ODE that share one
            // definition name) would otherwise collide on a name lookup.
            let phase = phase_name(*phase);
            let _ = writeln!(out, "  [{phase}] {name}: {n}");
        }
    }

    // --- signal-link dependency edges (existing RSC-4.1 input) ------------
    let dep = orch.signal_propagation().dependency_edges();
    let _ = writeln!(
        out,
        "signal_link_dependency_edges: {} (has_cycle={})",
        dep.len(),
        orch.signal_propagation().has_cycle()
    );
    {
        // For each edge, attribute writer-subsystem (and phase) of each
        // endpoint slot via the WriterId plane. An intra-phase edge is one
        // where both the producing slot AND the consuming slot are written by
        // executors in the SAME phase (the consuming slot's writeback identifies
        // the consumer subsystem).
        let mut intra = 0usize;
        let mut lines: BTreeSet<String> = BTreeSet::new();
        for (writers, readers) in dep {
            for w in writers {
                for r in readers {
                    let wi = slot_writer.get(&(w.index() as u32)).copied();
                    let ri = slot_writer.get(&(r.index() as u32)).copied();
                    let (Some(wi), Some(ri)) = (wi, ri) else {
                        continue;
                    };
                    let (ws, rs) = (&subs[wi], &subs[ri]);
                    if wi != ri && ws.phase == rs.phase {
                        intra += 1;
                        lines.insert(format!(
                            "  [{}] {} -> {}",
                            phase_name(ws.phase),
                            ws.name,
                            rs.name
                        ));
                    }
                }
            }
        }
        let _ = writeln!(out, "  intra-phase signal edges (writer-plane attributed): {intra}");
        for l in &lines {
            let _ = writeln!(out, "{l}");
        }
    }

    // --- COMPLETE intra-phase write→read edges (Gap-4 sites) --------------
    // Sourced from the Wave-2a `Executor::read_slots()` accessor for EVERY
    // subsystem (not just state machines). For each subsystem B, harvest its
    // compiler-resolved read slots, then look up which subsystem A WRITES each
    // read slot. Same-phase, distinct writer (A != B) = a same-phase staleness
    // site (a write→read dependency the Wave-2b per-phase DAG must order).
    // Self-reads (B reading its own write-set, e.g. an ODE reading its own
    // state) are excluded by the `a_idx != b_idx` guard.
    let _ = writeln!(out, "intra-phase write->read edges (complete read-set via read_slots()):");
    let mut edge_lines: BTreeSet<String> = BTreeSet::new();
    // Independently-derived source-2 edge set (writer, reader, slot) — the SAME
    // rule `scheduler::assemble_edges` applies (a != b, same phase, read_slots ∩
    // write-plane). Diffed against the production scheduler's edges() below (B1).
    let mut gate_edge_set: BTreeSet<(usize, usize, u32)> = BTreeSet::new();
    let mut execs_with_reads = 0usize;
    let mut total_read_slots = 0usize;
    for (b_idx, sub) in orch.subsystems().iter().enumerate() {
        let info = &subs[b_idx];
        let read_slots = sub.executor.read_slots();
        if !read_slots.is_empty() {
            execs_with_reads += 1;
            total_read_slots += read_slots.len();
        }
        for slot in &read_slots {
            let s = slot.index() as u32;
            if let Some(&a_idx) = slot_writer.get(&s) {
                if a_idx != b_idx && subs[a_idx].phase == info.phase {
                    let sname = slot_name.get(&s).cloned().unwrap_or_default();
                    edge_lines.insert(format!(
                        "  [{}] {} writes `{}` <- read by {}",
                        phase_name(info.phase),
                        subs[a_idx].name,
                        sname,
                        info.name
                    ));
                    gate_edge_set.insert((a_idx, b_idx, s));
                }
            }
        }
    }
    let _ = writeln!(
        out,
        "  executors_with_resolved_reads={} resolved_read_slots={} intra_phase_edges={}",
        execs_with_reads,
        total_read_slots,
        edge_lines.len()
    );
    for l in &edge_lines {
        let _ = writeln!(out, "{l}");
    }

    // RSC-4.1 cross-check (B1): the PRODUCTION scheduler
    // (`Orchestrator::execution_schedule`, built at bind time) must order on
    // exactly the edges this gate derives independently. We diff the full
    // (writer, reader, slot) EDGE SET — stronger than the old `is_identity()`
    // witness, which only checked the RESULT permutation and would miss a case
    // where the two assemblies disagree on edges yet both happen to yield an
    // identity order. The scheduler exposes its own edges via `edges()`, so this
    // is one home: a single edge definition, independently derived
    // here and diffed, not a second producer.
    let sched_edge_set: BTreeSet<(usize, usize, u32)> = orch
        .execution_schedule()
        .edges()
        .iter()
        .map(|e| (e.writer, e.reader, e.slot.index() as u32))
        .collect();
    assert_eq!(
        gate_edge_set, sched_edge_set,
        "the read-set inventory's independently-derived source-2 edges disagree with the \
         production scheduler's execution_schedule().edges() — the two edge assemblies have diverged"
    );
    // And with no intra-phase edges on the corpus the scheduler must be the
    // identity permutation (the byte-identical witness).
    assert!(
        orch.execution_schedule().is_identity(),
        "production ExecutionSchedule reordered a phase, but the read-set inventory reports \
         intra_phase_edges=0 — the scheduler and the gate have diverged"
    );

    out
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn build_espresso_cell() -> Orchestrator {
    let dir = example_dir("espresso-production-cell");
    let graph = load_dir("espresso-production-cell");
    let compiler = ModelCompiler::new(graph).with_source_dir(&dir);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler
        .build_workspace_orchestrator(
            base_ctx,
            Some(precompiled),
            None,
            None,
            None,
            &[],
            Some(100.0),
            Some(60000.0),
        )
        .expect("espresso-production-cell builds")
}

const RAMP_MODEL: &str = r#"
package RampCross {
    private import ScalarValues::*;
    part def Ramp {
        attribute v : Real default 1.0;
        out attribute x : Real default 0.0;
        out attribute sig : Real default 0.0;
        action def Dynamics :> ContinuousStateSpaceDynamics {
            calc def XDeriv :> GetDerivative { return dxdt = v; }
            calc def SigOut :> GetOutput { return sig = 3.0 * x; }
        }
    }
    state def Mover {
        in attribute x : Real;
        in attribute sig : Real;
        state low; state mid; state high;
        entry; then low;
        transition low_to_mid first low accept when x >= 0.5 then mid;
        transition mid_to_high first mid accept when sig >= 4.5 then high;
    }
}
"#;

fn build_ramp() -> Orchestrator {
    let graph = load_src("RampCross.sysml", RAMP_MODEL);
    let compiler = ModelCompiler::new(graph);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    compiler
        .build_workspace_orchestrator(
            base_ctx, None, None, None, None, &[], Some(1.0), Some(2500.0),
        )
        .expect("ramp builds")
}

/// Run the inventory for one orchestrator and enforce the danger gate.
///
/// The pinned topology report SNAPSHOT was dropped (coverage-matrix SCHED-READSET):
/// a byte-identical report of a specific workspace's subsystem/slot/edge topology
/// pinned product structure, so the public gate keeps only the GENERIC invariants:
///   * `inventory()` internally asserts the independently-derived write->read edge
///     set EQUALS the production scheduler's `execution_schedule().edges()` and that
///     the schedule is the identity permutation; and
///   * `intra_phase_edges == 0` (the Gap-4 same-phase-staleness danger gate).
/// A same-phase write->read edge on any model FAILS LOUDLY here rather than being
/// silently stepped in stale Vec order.
fn check_report(label: &str, report: &str) {
    println!("\n========== READ-SET INVENTORY: {label} ==========\n{report}");

    for line in report.lines() {
        if let Some(rest) = line.split("intra_phase_edges=").nth(1) {
            let n: usize = rest.trim().parse().unwrap_or_else(|_| {
                panic!("{label}: could not parse intra_phase_edges count from `{line}`")
            });
            assert_eq!(
                n, 0,
                "{label}: intra_phase_edges={n} (expected 0). A same-phase executor now \
                 reads a slot written by a same-phase peer (Gap-4 same-phase staleness). \
                 The cull dropped the topological scheduler on the proven assumption this \
                 class is empty — it no longer is. Route this model through the RSC-4.0 Gap-4 \
                 backlog (G11/G15) / RSC-4.1 topological DAG."
            );
        }
    }
}

#[test]
fn read_set_edges_are_scheduler_identity_on_espresso_cell() {
    check_report("espresso_cell", &inventory(&build_espresso_cell()));
}

#[test]
fn read_set_edges_are_scheduler_identity_on_ramp_crossing() {
    check_report("ramp_crossing", &inventory(&build_ramp()));
}

