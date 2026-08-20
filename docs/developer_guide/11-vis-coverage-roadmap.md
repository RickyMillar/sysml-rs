# Visualization Pipeline Coverage Roadmap

> **Status: FROZEN SNAPSHOT of the 2026-03 coverage push.** Every phase item and
> decision below is dated 2026-03-16 or 2026-03-20 and describes the pipeline as
> it stood then. This document is the *record* of that push, not a description of
> the current pipeline.
>
> **For the current pipeline, read
> [`13-vis-pipeline-architecture.md`](13-vis-pipeline-architecture.md).**
>
> Things referenced below that no longer exist: the TypeScript rendering stack the
> `ts_reg` axis measures (`editors/diagram/`, `sysml-module.ts`, `link-view.ts` —
> all deleted with that renderer), the `Requirements` view generator (retired
> 2026-06; see the Phase 6 note), and `12-vis-architecture-standard.md` /
> tracker `crates/lang/sysml-diagram/src/pipeline_coverage.toml` is still in the
> tree and still read by `build.rs`, but it has drifted from the source enums —
> see "Coverage Summary".

## Current Phase

Nothing is being tracked here any more. As last assessed (2026-03-16) all core
phases 1–8 were complete, with these left open: orthogonal state regions (needs
IR changes), history pseudo-states (quick win), browser relationship edges
(nice-to-have). Phases 9–10 below were never closed out.

---

## Coverage Summary

Re-measured 2026-08-13 against source. **Source** is the authority — the
`VisualKind` / `CompartmentKind` enums in
`crates/lang/sysml-diagram/src/visual_kind.rs` and `RelationshipKind` in
`crates/lang/sysml-core/src/relationship.rs`. **Tracked** is what
`pipeline_coverage.toml` carries. Where the two differ, the tracker is stale.

| Metric | Source | Tracked in TOML | Drift |
|--------|--------|-----------------|-------|
| Node types (`VisualKind`) | 38 | 39 | tracker carries a `NaryDot` entry that is not a `VisualKind` variant |
| Edge types (`RelationshipKind`) | 38 | 37 | `Refine` is untracked |
| Compartment types (`CompartmentKind`) | 69 | 68 | `Redefinitions` is untracked |
| Generator (node × view) pairs | — | 76 / 91 done, excluding `n/a` | recorded here as 77 / 95 |
| `#[test]` fns under `ir/generators/` | 159 | — | recorded here as 153 |

The original version of this table claimed 100% node and edge coverage. Both
claims are now false, and the `ts_reg` axis those percentages were computed from
recorded registration in a TypeScript module that has since been deleted — read
it as meaningless, not as green. `build.rs` does still validate the tracker, but
only for nodes and edges and only against hardcoded name lists inside `build.rs`
itself (which carry the same stale `NaryDot` entry); compartment names are counted
but never checked. So none of the drift above surfaces as a build warning.

---

## Phase Checklist

### Phase 0: Tracking Infrastructure
- [x] Create pipeline_coverage.toml (2026-03-16)
- [x] Create this roadmap document (2026-03-16)
- [x] Create vis coverage agent instructions (2026-03-16)
- [x] Add build.rs coverage validation to sysml-diagram (2026-03-16)

### Phase 1: Fix Runtime Crashes — Missing Edge & Node Registrations
**Priority: CRITICAL**

All items completed 2026-03-16:
- [x] Register 16 missing edge types in sysml-module.ts
- [x] Register `node:sqProxy` and `node:naryDot` in sysml-module.ts
- [x] Fix `Actor` node_type: "node:block" → "node:actor" in visual_kind.rs
- [x] Add marker mappings for new edge types in link-view.ts
- [x] Update pipeline_coverage.toml (39/39 nodes, 37/37 edges, 0 crash risks)

### Phase 2: Complete GeneralView Node Coverage
**Priority: HIGH**
- [x] Audit is_bdd_relevant() — correctly filters unnamed control nodes (they're in embedded sub-diagrams, not BDD) (2026-03-16)
- [x] Control nodes (Initial, Final, etc.) already handled via embedded state/action sub-diagrams — not a gap (2026-03-16)
- [x] SendAction/AcceptAction already handled via embedded action sub-diagrams (2026-03-16)
- [x] Actor: add context-aware `effective_visual_kind()` — PartUsage via ActorMembership → Actor stick figure (2026-03-16)
- [x] Create test .sysml files: all-graphical-kinds, use-case-actors, state-action-embedded, all-edge-types (2026-03-16)
- [ ] Update pipeline_coverage.toml

### Phase 3: Compartment Population Audit
**Priority: HIGH** — COMPLETE (2026-03-16)
- [x] Audit compartment_for_element() routing — mostly complete, property-based routing works
- [x] DirectedFeatures — works via `direction` property (parser already extracts)
- [x] Ends — works via `isEnd` property (parser already extracts)
- [x] Variants — works via `isVariation` property (parser already extracts)
- [x] Individuals — added `isIndividual` parser extraction (individual keyword)
- [x] Timeslices/Snapshots — added `portionKind` parser extraction (snapshot/timeslice keywords)
- [x] VariantUsages — documented as needing graph context (future work)
- [x] Update pipeline_coverage.toml

### Phase 4: Requirements View Enrichment
**Priority: MEDIUM** — DONE (2026-03-16)
- [x] Nested requirement hierarchy — already supported via recursive generation
- [x] Concern/VerificationCase — already visually distinct via VisualKind CSS classes
- [x] Subject compartment — added (filters isSubject=true or ReferenceUsage named "subject")
- [x] Assume/require constraint compartments — added (splits by isAssume property)
- [x] reqId rendering — added ("id = <value>" label)

### Phase 5: Interconnection View Enrichment
**Priority: MEDIUM** — DONE (2026-03-16)
- [x] Allocate edges — added to edge filter
- [x] InterfaceConnection edges — added to edge filter
- [ ] Port conjugation visual indicators — future (needs conjugation property extraction)
- [ ] Connection multiplicity labels — future (needs end feature metadata)

### Phase 6: IR Generator Tests
**Priority: MEDIUM** — ALREADY DONE (discovered 2026-03-16)
All 9 generators then in `ir/generators/` already have `#[cfg(test)] mod tests`
blocks. There are **8** generators today: `requirements.rs` was deleted when the
`Requirements` view kind was retired (2026-06,
notation now renders inside the `General` generator.
- [x] ir/generators/general.rs — comprehensive tests (2026-03-16: pre-existing)
- [x] ir/generators/state.rs — comprehensive tests
- [x] ir/generators/action.rs — comprehensive tests
- [x] ir/generators/requirements.rs — comprehensive tests
- [x] ir/generators/interconnection.rs — comprehensive tests
- [x] ir/generators/sequence.rs — comprehensive tests
- [x] ir/generators/browser.rs — comprehensive tests
- [x] ir/generators/grid.rs — comprehensive tests (including extended relationship types)
- [x] ir/generators/geometry.rs — comprehensive tests (position + child positioning)
Total: 153 tests passing.

### Phase 7: State & Action View Completion
**Priority: MEDIUM** — MOSTLY COMPLETE (assessed 2026-03-16)
- [x] Loop/conditional action nodes — already implemented (If, WhileLoop, ForLoop)
- [x] Send/accept action nodes in ActionFlow — already implemented
- [x] Structured action parameters — already implemented (input/output ports)
- [x] Composite state nesting with depth limit (MAX_STATE_DEPTH=20)
- [x] Type expansion (typed usage → definition children)
- [ ] Orthogonal state regions — needs sysml-runtime IR changes (future)
- [ ] History pseudo-states — quick win (~30 lines), low risk

### Phase 8: Browser & Grid View Polish
**Priority: LOW** — ASSESSED (2026-03-16)
Browser and Grid are production-quality for their scope:
- Browser: clean ownership tree with expand/collapse, child counts
- Grid: full traceability matrix with Satisfy/Verify/Allocate/Derive/Trace/Dependency
- [ ] Browser: relationship edge display (nice-to-have)
- [ ] Grid: hierarchical matrix for nested requirements (nice-to-have)

### Phase 9: View Composition & Nesting
**Priority: HIGH** — In progress (2026-03-20)

Container nesting architectural fixes:
- [x] Multi-view additive composition (else-if → independent if blocks) (2026-03-20)
- [x] Top-level duplicate ID fix (expanded owner filtering) (2026-03-20)
- [x] Fix A: Embedded compartment layout: "vbox" → None (2026-03-20)
- [x] Fix B: hasGraphChildren compartment traversal (2026-03-20)
- [ ] Fix C: Action subtree generator rewrite (walks children, not compile by name)
- [ ] Fix D: ELK padding alignment (15px vs 80px mismatch)
- [ ] Fix E: Scoped ID system for GeneralView embedding

describe the retired renderer and are superseded by
[`13-vis-pipeline-architecture.md`](13-vis-pipeline-architecture.md)):
- [x] Create 12-vis-architecture-standard.md (2026-03-20)
- [ ] Update 10-vis-pipeline-audit.md VBox decision — never done, and now moot

### Phase 10: Remaining View Features
**Priority: MEDIUM**

From previous roadmap items still open + new spec-derived items:
- [ ] Orthogonal state regions (needs sysml-runtime IR changes)
- [ ] History pseudo-states (quick win, ~30 lines)
- [ ] Browser relationship edge display
- [ ] Grid hierarchical matrix for nested requirements
- [ ] Port conjugation visual indicators
- [ ] Connection multiplicity labels
- [ ] GeometryView: 2D spatial layout with position extraction
- [ ] Framed views: ViewUsage as usage-node with diagram frame

---

## Decisions Log

| Date | Decision | Reason |
|------|----------|--------|
| 2026-03-16 | Track at VisualKind granularity (37) not ElementKind (266) | ElementKind→VisualKind is already a many-to-one mapping; tracking 266 is redundant |
| 2026-03-16 | TOML format for tracking file | Follows proven semantic_rules.toml pattern; machine-readable + diffable |
| 2026-03-16 | VBox layout is required (not dead code) | Reason as recorded, specific to the retired renderer: tested removal — labels overlap at (0,0); that renderer's VBox handled intra-node stacking. `NodeLayout::VBox` still exists in the IR today, but this justification no longer applies. |
| 2026-03-16 | Prioritize crash fixes (Phase 1) over features | 16 edge types + 2 node types crash at runtime if emitted |
| 2026-03-16 | Control nodes in GeneralView are NOT a gap | They're correctly handled via embedded state/action sub-diagrams when behavioral defs are expanded |
| 2026-03-16 | Actor rendering is context-dependent | No ElementKind maps to Actor — it's PartUsage owned via ActorMembership. Added `effective_visual_kind()` |
| 2026-03-20 | Embedded sub-diagram compartments must use layout: None, not "vbox" | VBox stacks children vertically; ELK with INCLUDE_CHILDREN is needed for proper diagram layout. State view was the reference. |
| 2026-03-20 | Action subtree generator must walk children, not compile by name | `generate_named(graph, "Vehicle")` fails for PartDefinitions. State view pattern (walk children_of) is correct. |
| 2026-03-20 | All subtree generators should follow a uniform contract | `generate()`, `generate_subtree()`, `generate_subtree_for_owner()` with `expanded_ids` param. |
| 2026-03-20 | Frameless views: general, interconnection, action-flow, state-transition, sequence | Per spec BNF. Geometry, grid, browser are framed-only. |

---

## Session Protocol

Each Claude session working on vis coverage should:
1. Read `crates/lang/sysml-diagram/src/pipeline_coverage.toml` → get current status
2. Read this roadmap → find "Current Phase" and next unchecked items
3. Read MEMORY.md vis section → get context from last session
4. Work on unchecked items in current phase
5. Update `pipeline_coverage.toml` + check items here when done
6. Update MEMORY.md if decisions were made
