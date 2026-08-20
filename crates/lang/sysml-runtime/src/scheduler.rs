//! RSC-4.1 — per-phase topological scheduler (design doc
//!
//! Orders same-phase executors by slot data dependency so a same-phase producer
//! steps before its consumer, closing the Gap-4 same-phase staleness class
//! (G11/G15). The DAG has two edge sources, joined per `ExecutionPhase`:
//!
//! 1. **Signal-link edges** — `SignalPropagation::dependency_edges()`
//!    (`[writer slots] → [reader slots]`), projected onto subsystem nodes via
//!    each slot's `WriterId::Executor(i)` (RSC-2.0 D-2.0.2).
//! 2. **Cross-subsystem read/write edges** — for same-phase A, B: if A's
//!    write-set (slots it owns as `WriterId::Executor`) intersects B's read-set
//!    (`Executor::read_slots()`), edge A → B.
//!
//! A Kahn topological sort per phase yields the order. Ties break by the node's
//! `WriterId::Executor(i)` identity — the executor's canonical slot-plane
//! identity, which is minted in subsystem-registration order, so the order is
//! **byte-identical to `Vec` insertion order whenever a phase has no intra-phase
//! edges** (`edges = 0` today, corpus-wide). The spec permits any deterministic
//! order for unsequenced-concurrent performances (Performances.kerml:47-81), so
//! this tie-break is spec-clean.
//!
//! Cycles (algebraic loops) cannot be ordered: the members are appended in
//! insertion order (current same-tick-stale behaviour, handed to the RSC-4.2
//! convergence loop) and a **RS011** diagnostic is emitted (sibling to the
//! RS010 signal-link-cycle diagnostic).
//!
//! The schedule is consumed as a **positional within-phase permutation**
//! ([`ExecutionSchedule::remap`]): each phase's subsystems keep the `Vec`
//! positions they occupy, reordered only among themselves. At `edges = 0` the
//! remap is the identity, so every consumer (the tick loop's per-phase passes
//! and the convergence loop) is byte-identical; cross-phase interleaving is
//! preserved.

use std::collections::{BTreeMap, BTreeSet};

use sysml_span::Diagnostic;

use crate::orchestrator::{ExecutionPhase, Subsystem};
use crate::slots::{SlotId, SlotStore, WriterId};

/// Deterministic rank for phase ordering (Physics first … Action last), matching
/// the tick loop's phase pass order in `Orchestrator::step_inner`.
pub fn phase_rank(phase: ExecutionPhase) -> u8 {
    match phase {
        ExecutionPhase::Physics => 0,
        ExecutionPhase::ContinuousDynamics => 1,
        ExecutionPhase::DiscreteDynamics => 2,
        ExecutionPhase::StateMachine => 3,
        ExecutionPhase::Action => 4,
    }
}

/// A within-phase write→read dependency between two subsystems (indices into
/// `Orchestrator::subsystems`). Both endpoints are in the SAME phase — cross-
/// phase coupling is handled by phase-rank ordering, not the DAG.
#[derive(Debug, Clone)]
pub struct ScheduleEdge {
    /// Subsystem index that WRITES the shared slot (must run first).
    pub writer: usize,
    /// Subsystem index that READS it (must run after).
    pub reader: usize,
    /// The slot carrying the dependency (for diagnostics / the inventory gate).
    /// The phase is derivable from either endpoint's subsystem, so not stored.
    pub slot: SlotId,
}

/// Map `slot index → writing subsystem index`, restricted to the
/// `WriterId::Executor` plane (the only writers the scheduler orders on;
/// Orchestrator/External writers are not subsystems).
fn slot_writer_map(store: &SlotStore) -> BTreeMap<u32, usize> {
    let mut m = BTreeMap::new();
    for (id, meta, _v) in store.iter() {
        if let WriterId::Executor(i) = meta.writer {
            m.insert(id.index() as u32, i as usize);
        }
    }
    m
}

/// Assemble every intra-phase subsystem dependency edge that intra-phase
/// ordering can actually make fresh-vs-stale: a **cross-subsystem write→read**
/// where subsystem B's `read_slots()` contains a slot written by same-phase
/// subsystem A. Shared by the scheduler (below) and the RSC-4.B0 read-set
/// inventory gate so the gate validates the SAME edges the scheduler orders on
/// (CLAUDE.md #4 — one home). Edges are de-duplicated on `(writer, reader, slot)`.
///
/// **Why only read-set edges (D-4.1.1, corrected 2026-07-03).** The design
/// originally also projected signal-link `dependency_edges()` onto subsystem
/// nodes ("source 1"). Those edges were dropped: signal delivery is the single
/// trailing `propagate_port_values()` pass (`orchestrator.rs:3406`), which runs
/// AFTER every phase pass that consumes the schedule — so reordering subsystems
/// within a phase cannot change what propagation later delivers. Signal-link
/// edges therefore bought no freshness, and combined with a read-set edge in the
/// opposite direction could fabricate a spurious RS011 2-cycle on a model with
/// no real algebraic loop. Genuine same-phase reads are already captured here
/// via `read_slots()`. (`dependency_edges()` remains load-bearing for
/// `order_pairs`/RS010 in `links.rs` — a different, slot-pair-granularity
/// concern, unaffected.)
pub fn assemble_edges(subsystems: &[Subsystem], store: &SlotStore) -> Vec<ScheduleEdge> {
    let slot_writer = slot_writer_map(store);
    let phase_of = |i: usize| subsystems[i].executor.phase();
    let mut edges: Vec<ScheduleEdge> = Vec::new();
    let mut seen: BTreeSet<(usize, usize, u32)> = BTreeSet::new();

    // Cross-subsystem write→read edges via `read_slots()`: subsystem B reads a
    // slot the same-phase subsystem A writes, so A must step before B.
    for (b, sub) in subsystems.iter().enumerate() {
        for slot in sub.executor.read_slots() {
            if let Some(&a) = slot_writer.get(&(slot.index() as u32)) {
                if a != b
                    && phase_of(a) == phase_of(b)
                    && seen.insert((a, b, slot.index() as u32))
                {
                    edges.push(ScheduleEdge {
                        writer: a,
                        reader: b,
                        slot,
                    });
                }
            }
        }
    }

    edges
}

/// `true` iff `start` lies on a directed cycle within `node_set` — i.e. it can
/// reach itself following `succs` edges (path length ≥ 1) without leaving the
/// phase. Used to separate genuine cycle members from nodes merely stranded
/// downstream of a cycle. Phase node-counts are tiny, so a plain DFS is fine.
fn node_on_cycle(
    start: usize,
    succs: &BTreeMap<usize, BTreeSet<usize>>,
    node_set: &BTreeSet<usize>,
) -> bool {
    let mut stack: Vec<usize> = succs
        .get(&start)
        .map(|s| s.iter().copied().filter(|x| node_set.contains(x)).collect())
        .unwrap_or_default();
    let mut visited: BTreeSet<usize> = BTreeSet::new();
    while let Some(n) = stack.pop() {
        if n == start {
            return true;
        }
        if !visited.insert(n) {
            continue;
        }
        if let Some(ss) = succs.get(&n) {
            for &s in ss {
                if node_set.contains(&s) {
                    stack.push(s);
                }
            }
        }
    }
    false
}

/// Pure per-phase Kahn topological sort — the scheduler's core, separated from
/// the `Subsystem`/`SlotStore` plumbing so it is directly unit-testable.
///
/// `rank_of[i]` = the phase rank of node `i` (`0..n`); `pairs` = the distinct
/// same-phase `(writer → reader)` dependency pairs. Returns, per phase rank
/// present, the topologically-ordered node list, plus the set of nodes that
/// lie **on** an unbreakable cycle. All un-ordered nodes (both true cycle
/// members and their innocent downstream readers) are appended to their phase
/// order in ascending index order — same-tick-stale, handed to the RSC-4.2
/// convergence loop — but only the nodes actually on a cycle are returned as
/// `cycle_members`, so the RS011 message names the real loop, not the nodes
/// merely blocked behind it.
///
/// Ties among ready nodes break by ascending node index — the executor's
/// `WriterId::Executor(i)` identity — so the order is the identity permutation
/// (== `Vec` insertion order) whenever a phase has no intra-phase pairs.
fn kahn_by_phase(
    rank_of: &[u8],
    pairs: &BTreeSet<(usize, usize)>,
) -> (BTreeMap<u8, Vec<usize>>, BTreeSet<usize>) {
    let mut nodes_by_rank: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, &r) in rank_of.iter().enumerate() {
        nodes_by_rank.entry(r).or_default().push(i);
    }

    let mut succs: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut preds: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(w, r) in pairs {
        if succs.entry(w).or_default().insert(r) {
            preds.entry(r).or_default().insert(w);
        }
    }

    let mut order_by_rank: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    let mut cycle_members: BTreeSet<usize> = BTreeSet::new();

    for (rank, nodes) in &nodes_by_rank {
        let node_set: BTreeSet<usize> = nodes.iter().copied().collect();
        let mut indeg: BTreeMap<usize, usize> = BTreeMap::new();
        for &n in nodes {
            let d = preds
                .get(&n)
                .map(|p| p.iter().filter(|x| node_set.contains(x)).count())
                .unwrap_or(0);
            indeg.insert(n, d);
        }
        // BTreeSet ready-set → `.next()` pops the smallest index (the tie-break).
        let mut ready: BTreeSet<usize> =
            nodes.iter().copied().filter(|n| indeg[n] == 0).collect();
        let mut order: Vec<usize> = Vec::with_capacity(nodes.len());
        while let Some(&n) = ready.iter().next() {
            ready.remove(&n);
            order.push(n);
            if let Some(ss) = succs.get(&n) {
                for &s in ss {
                    if !node_set.contains(&s) {
                        continue;
                    }
                    if let Some(d) = indeg.get_mut(&s) {
                        *d -= 1;
                        if *d == 0 {
                            ready.insert(s);
                        }
                    }
                }
            }
        }

        if order.len() < nodes.len() {
            let ordered: BTreeSet<usize> = order.iter().copied().collect();
            for &n in nodes {
                if !ordered.contains(&n) {
                    // A node is a true cycle member iff it can reach itself
                    // through same-phase successors; a node merely downstream of
                    // a cycle is un-ordered too (its predecessor never settled)
                    // but is NOT on the loop, so it must not be named in RS011.
                    if node_on_cycle(n, &succs, &node_set) {
                        cycle_members.insert(n);
                    }
                    order.push(n);
                }
            }
        }

        order_by_rank.insert(*rank, order);
    }

    (order_by_rank, cycle_members)
}

/// The compiled per-phase execution order (built once at orchestrator-build
/// time, consulted by index at tick — no per-tick sort).
#[derive(Debug, Clone, Default)]
pub struct ExecutionSchedule {
    /// Per phase (keyed by [`phase_rank`]): `(phase, subsystem indices in
    /// topological order)`.
    order_by_rank: BTreeMap<u8, (ExecutionPhase, Vec<usize>)>,
    /// The positional within-phase permutation of `0..n`, precomputed once at
    /// [`build`](Self::build) time (B4) and handed out as a slice at tick — the
    /// permutation is fixed for the life of the schedule, so recomputing it every
    /// `step_inner` was pure waste.
    remap: Vec<usize>,
    /// RS011 within-phase-cycle diagnostics (unified with RS010 at the
    /// orchestrator's reporting surface).
    diagnostics: Vec<Diagnostic>,
    /// The assembled edges (exposed so the RSC-4.B0 inventory gate validates the
    /// scheduler's own edges).
    edges: Vec<ScheduleEdge>,
}

impl ExecutionSchedule {
    /// Build the schedule from the current subsystem set and slot store (the
    /// writer plane + each executor's read-set).
    pub fn build(subsystems: &[Subsystem], store: &SlotStore) -> Self {
        let edges = assemble_edges(subsystems, store);

        let rank_of: Vec<u8> = subsystems
            .iter()
            .map(|s| phase_rank(s.executor.phase()))
            .collect();
        // Distinct dependency pairs (collapse multi-slot edges between a pair).
        let pairs: BTreeSet<(usize, usize)> =
            edges.iter().map(|e| (e.writer, e.reader)).collect();

        let (orders, cycle_members) = kahn_by_phase(&rank_of, &pairs);

        // Attach the phase to each rank + render one RS011 per phase with a cycle.
        let rank_to_phase: BTreeMap<u8, ExecutionPhase> = subsystems
            .iter()
            .map(|s| (phase_rank(s.executor.phase()), s.executor.phase()))
            .collect();
        let mut order_by_rank: BTreeMap<u8, (ExecutionPhase, Vec<usize>)> = BTreeMap::new();
        for (rank, order) in orders {
            let phase = rank_to_phase[&rank];
            order_by_rank.insert(rank, (phase, order));
        }
        let mut diagnostics = Vec::new();
        let mut cyc_by_rank: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
        for &m in &cycle_members {
            cyc_by_rank.entry(rank_of[m]).or_default().push(m);
        }
        for members in cyc_by_rank.values() {
            let names: Vec<String> =
                members.iter().map(|&i| subsystems[i].name.clone()).collect();
            diagnostics.push(
                Diagnostic::warning(format!(
                    "within-phase dependency cycle ({}); the loop's members step in \
                     ascending-index order (same-tick-stale values within the cycle)",
                    names.join(" -> ")
                ))
                .with_code("RS011")
                .with_note(
                    "an algebraic loop of same-phase executors cannot be topologically \
                     ordered; values inside the cycle use the previous sub-step's value. \
                     Break the loop or enable the convergence iteration (RSC-4.2) to settle it. \
                     (Complementary to RS010: RS010 names a signal-link SLOT-chain cycle; \
                     RS011 names the SUBSYSTEM feedback loop — a chain cycle surfaces as both \
                     on the shared compile-warning surface, and RS011 also catches mutual \
                     subsystem dependencies that form no slot chain.)",
                ),
            );
        }

        let remap = Self::compute_remap(&order_by_rank, subsystems.len());
        ExecutionSchedule {
            order_by_rank,
            remap,
            diagnostics,
            edges,
        }
    }

    /// The **positional within-phase permutation** of `0..n`, precomputed at
    /// [`build`](Self::build) time: each phase's subsystems keep the `Vec`
    /// positions they occupy, reordered among themselves into topological order.
    /// The identity when every phase has no intra-phase edges — so consumers that
    /// iterate this instead of raw `Vec` order stay byte-identical at `edges = 0`
    /// while gaining topo order when edges exist.
    pub fn remap_order(&self) -> &[usize] {
        &self.remap
    }

    /// Compute the positional within-phase permutation of `0..n` from the
    /// per-phase topological orders (private; `build` calls it once). Each
    /// phase's members occupy exactly the `Vec` positions equal to their own
    /// (ascending) indices; the topo-ordered members are dropped into those
    /// positions, leaving every other phase untouched.
    fn compute_remap(
        order_by_rank: &BTreeMap<u8, (ExecutionPhase, Vec<usize>)>,
        n: usize,
    ) -> Vec<usize> {
        let mut remap: Vec<usize> = (0..n).collect();
        for (_, (_, order)) in order_by_rank {
            let mut positions: Vec<usize> = order.clone();
            positions.sort_unstable();
            for (pos, &sched_idx) in positions.iter().zip(order.iter()) {
                remap[*pos] = sched_idx;
            }
        }
        remap
    }

    /// RS011 diagnostics (empty when no within-phase cycle).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The assembled intra-phase edges (for the RSC-4.B0 inventory gate).
    pub fn edges(&self) -> &[ScheduleEdge] {
        &self.edges
    }

    /// `true` when the schedule is the identity permutation (every phase in
    /// ascending-index order) — i.e. no intra-phase edges reordered anything.
    /// The corpus invariant today; a parity witness for the byte-identical gate.
    pub fn is_identity(&self) -> bool {
        self.order_by_rank
            .values()
            .all(|(_, ord)| ord.windows(2).all(|w| w[0] < w[1]))
    }

    /// Test-only constructor for the pure `remap`/order layer (bypasses the
    /// `Subsystem`/`SlotStore` plumbing that `build` needs). Derives `n` from the
    /// dense `0..n` node indices (every node appears in exactly one phase order).
    #[cfg(test)]
    fn from_orders(order_by_rank: BTreeMap<u8, (ExecutionPhase, Vec<usize>)>) -> Self {
        let n = order_by_rank
            .values()
            .flat_map(|(_, o)| o.iter().copied())
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        let remap = Self::compute_remap(&order_by_rank, n);
        ExecutionSchedule {
            order_by_rank,
            remap,
            diagnostics: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A single phase (rank 0) with a producer→consumer edge where the producer
    // is registered AFTER the consumer: Vec order [0=consumer, 1=producer], but
    // the edge 1→0 forces the producer (1) to step first. Proves the scheduler
    // actually topologically reorders (not a no-op).
    #[test]
    fn producer_registered_after_consumer_is_reordered_first() {
        let rank_of = [0u8, 0u8]; // both in the same phase
        let pairs: BTreeSet<(usize, usize)> = [(1usize, 0usize)].into_iter().collect(); // 1 writes, 0 reads
        let (orders, cycles) = kahn_by_phase(&rank_of, &pairs);
        assert!(cycles.is_empty());
        assert_eq!(orders[&0], vec![1, 0], "producer (1) must precede consumer (0)");
    }

    // No edges → identity order (== Vec insertion order), the byte-identical
    // guarantee at intra_phase_edges=0.
    #[test]
    fn no_edges_is_identity_order() {
        let rank_of = [0u8, 0u8, 0u8];
        let pairs = BTreeSet::new();
        let (orders, cycles) = kahn_by_phase(&rank_of, &pairs);
        assert!(cycles.is_empty());
        assert_eq!(orders[&0], vec![0, 1, 2]);
    }

    // Ties (independent nodes) break by ascending index (WriterId::Executor
    // identity): with only 2→3, nodes 0/1 stay ahead in index order and 2
    // precedes 3.
    #[test]
    fn ties_break_by_ascending_index() {
        let rank_of = [0u8, 0u8, 0u8, 0u8];
        let pairs: BTreeSet<(usize, usize)> = [(2usize, 3usize)].into_iter().collect();
        let (orders, _) = kahn_by_phase(&rank_of, &pairs);
        assert_eq!(orders[&0], vec![0, 1, 2, 3]);
    }

    // Edges only order WITHIN a phase; two phases are ordered independently.
    #[test]
    fn phases_are_ordered_independently() {
        let rank_of = [0u8, 1u8, 0u8]; // nodes 0,2 in phase 0; node 1 in phase 1
        let pairs: BTreeSet<(usize, usize)> = [(2usize, 0usize)].into_iter().collect();
        let (orders, _) = kahn_by_phase(&rank_of, &pairs);
        assert_eq!(orders[&0], vec![2, 0], "phase 0 reordered by its edge");
        assert_eq!(orders[&1], vec![1], "phase 1 untouched");
    }

    // A 2-cycle within a phase: unorderable → both members flagged, appended in
    // ascending (insertion) order, no panic.
    #[test]
    fn within_phase_cycle_flags_members_insertion_order() {
        let rank_of = [0u8, 0u8];
        let pairs: BTreeSet<(usize, usize)> =
            [(0usize, 1usize), (1usize, 0usize)].into_iter().collect();
        let (orders, cycles) = kahn_by_phase(&rank_of, &pairs);
        assert_eq!(cycles, [0usize, 1usize].into_iter().collect());
        assert_eq!(orders[&0], vec![0, 1], "cycle members kept in insertion order");
    }

    // remap is a POSITIONAL within-phase permutation: a phase's members keep the
    // Vec slots they occupy, reordered among themselves; other phases untouched.
    #[test]
    fn remap_is_positional_within_phase() {
        // 4 subsystems: indices 0,2 in phase P (rank 0), 1,3 in phase Q (rank 1).
        // Phase P scheduled as [2, 0] (reordered); phase Q as [1, 3] (identity).
        let mut obr: BTreeMap<u8, (ExecutionPhase, Vec<usize>)> = BTreeMap::new();
        obr.insert(0, (ExecutionPhase::Physics, vec![2, 0]));
        obr.insert(1, (ExecutionPhase::StateMachine, vec![1, 3]));
        let sched = ExecutionSchedule::from_orders(obr);
        // Phase P occupies positions {0,2}: pos0←2, pos2←0. Phase Q positions
        // {1,3} unchanged. So remap = [2, 1, 0, 3].
        assert_eq!(sched.remap_order(), &[2, 1, 0, 3]);
    }

    #[test]
    fn remap_identity_when_all_phases_ascending() {
        let mut obr: BTreeMap<u8, (ExecutionPhase, Vec<usize>)> = BTreeMap::new();
        obr.insert(0, (ExecutionPhase::Physics, vec![0, 1]));
        obr.insert(1, (ExecutionPhase::StateMachine, vec![2, 3]));
        let sched = ExecutionSchedule::from_orders(obr);
        assert!(sched.is_identity());
        assert_eq!(sched.remap_order(), &[0, 1, 2, 3]);
    }

    // A6: a 2-cycle (0↔1) with a node (2) merely downstream of it (1→2). Node 2
    // is un-orderable too (its predecessor 1 never settles), but it is NOT on the
    // loop — only 0 and 1 may be flagged as cycle members / named in RS011.
    #[test]
    fn downstream_of_cycle_is_not_flagged_as_member() {
        let rank_of = [0u8, 0u8, 0u8];
        let pairs: BTreeSet<(usize, usize)> = [(0usize, 1usize), (1usize, 0usize), (1usize, 2usize)]
            .into_iter()
            .collect();
        let (orders, cycles) = kahn_by_phase(&rank_of, &pairs);
        assert_eq!(
            cycles,
            [0usize, 1usize].into_iter().collect(),
            "only the true loop members (0,1) are flagged; the downstream reader (2) is not"
        );
        // all three still execute, appended in ascending index order
        assert_eq!(orders[&0], vec![0, 1, 2]);
    }

    // A5: the full `build` path end-to-end — a real SlotStore + Subsystems where
    // subsystem 0 (consumer) READS a slot written by subsystem 1 (producer), both
    // in the same phase. Because the producer is registered AFTER the consumer,
    // `Vec` order would step the consumer on a stale value; the scheduler must
    // detect the read_slots∩write-plane edge (assemble_edges), reorder via Kahn,
    // and yield a NON-identity remap placing the producer first. This is the only
    // test that exercises assemble_edges → kahn → remap through `build` (the 7
    // above use synthetic pairs / from_orders), i.e. the proof the Gap-4 fix acts.
    #[derive(Clone)]
    struct StubExec {
        phase: ExecutionPhase,
        reads: Vec<SlotId>,
    }
    impl crate::orchestrator::Executor for StubExec {
        fn phase(&self) -> ExecutionPhase {
            self.phase
        }
        fn kind_label(&self) -> &'static str {
            "stub"
        }
        fn tick(&mut self, _ctx: &crate::orchestrator::TickContext<'_>) -> crate::orchestrator::TickOutput {
            crate::orchestrator::TickOutput::solver(String::new(), Vec::new())
        }
        fn reset_executor(&mut self) {}
        fn is_completed(&self) -> bool {
            false
        }
        fn clone_boxed(&self) -> Box<dyn crate::orchestrator::Executor> {
            Box::new(self.clone())
        }
        fn read_slots(&self) -> Vec<SlotId> {
            self.reads.clone()
        }
    }

    #[test]
    fn build_reorders_producer_before_consumer_from_read_slots() {
        use crate::slots::{RuntimeId, SlotMeta, Variability};
        use sysml_core::Value;

        let mut store = SlotStore::new();
        // The shared slot is written by subsystem index 1 (the producer).
        let slot0 = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Executor(1),
                "x",
                "x",
            ),
            Value::Float(0.0),
        );

        let mk = |name: &str, reads: Vec<SlotId>| Subsystem {
            name: name.to_string(),
            executor: Box::new(StubExec {
                phase: ExecutionPhase::ContinuousDynamics,
                reads,
            }),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        };
        // index 0 = consumer (reads slot0), index 1 = producer (writes slot0).
        let subs = vec![mk("consumer", vec![slot0]), mk("producer", vec![])];

        let sched = ExecutionSchedule::build(&subs, &store);

        // Exactly one edge: producer(1) → consumer(0).
        assert_eq!(sched.edges().len(), 1, "one read_slots∩write-plane edge");
        assert_eq!(
            (sched.edges()[0].writer, sched.edges()[0].reader),
            (1, 0),
            "edge is producer(1) → consumer(0)"
        );
        assert!(sched.diagnostics().is_empty(), "no cycle → no RS011");
        // The remap must reorder: producer (1) steps before consumer (0).
        assert!(!sched.is_identity(), "a real same-phase edge reorders the phase");
        assert_eq!(
            sched.remap_order(),
            &[1, 0],
            "producer is visited before the consumer that reads its slot"
        );
    }
}
