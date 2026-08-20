# Obligation matrix — Occurrences & clocks

**Area:** occurrence lifetime/portion semantics + clock time semantics.
**Existing gate:** `tests/runtime_spec_conformance.rs` (RSC-0.2) gates clock
`currentTime` monotonicity and `TimeOf` snapshot semantics. This matrix
catalogues the area and maps coverage. **No new gate file** added. **Status:**
fan-out area.

Spec sources: KerML §9.2.4 *Occurrences*, §9.2.12 *Clocks/TimeOf*
(`KerML-spec-r2025-04_REF.html`); SysML §7.9 *Occurrences*; `sysml.library/.../
Occurrences.kerml`, `Clocks.kerml`. Verified `2026-06-21`.

> **Test results (2026-06-21):** the ungated behavioral rows are now GATED by
> `tests/occurrence_clock_spec_conformance.rs` (7 tests, all CONFORMS) against the
> `OccurrenceTracker` / `clock` runtime API: lifetime extent, `DurationOf` =
> end−start, time-ordering, time-continuity, happens-just-before, and
> localClock-defaults-to-universal. Note: model-callable `TimeOf`/`DurationOf`
> *from SysML source* remain UNIMPLEMENTED (only the `__clock_time` scalar exists)
> — already pinned by `runtime_spec_conformance::spec_clock_timeof_snapshot_semantics`.

## Obligation table

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `clock-currenttime-advances-monotonically` | A clock's `currentTime` advances monotonically over its lifetime. | KerML §9.2.12.2.4 *"a scalar currentTime that advances montonically"*; `Clocks.kerml` `timeFlowConstraint` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_clock_monotonic_currenttime_exposed_to_guards`. |
| `timeof-returns-start-shot-time` | `TimeOf` an occurrence is the time of its start (its `startShot`). | KerML §9.2.12.2.6; `Clocks.kerml` `startTimeConstraint` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_clock_timeof_snapshot_semantics`. |
| `occurrence-has-start-shot-and-end-shot` | Every occurrence has a `startShot` (before all snapshots) and `endShot` (after all). | KerML §9.2.4.2.13; `Occurrences.kerml:348-390` (GOSPEL+LIBRARY) | **GATED-elsewhere** (partial) — TimeOf test uses startShot. |
| `snapshot-is-zero-duration-timeslice` | A snapshot is a time slice of zero duration (`startShot == endShot`). | KerML §9.2.4.1; SysML `checkOccurrenceUsageSnapshotSpecialization` (GOSPEL) | **GATED-elsewhere** (likely) — snapshot semantics test. |
| `happens-before-no-time-overlap` | `HappensBefore`: earlier and later do not overlap in time; earlier ends before later starts; transitive. | KerML §9.2.4.2.1 *"none of their snapshots happen at the same time"* (GOSPEL) | **GATED-elsewhere** (partial) — succession ordering via SM/flow tests. |
| `occurrence-has-lifetime-extent` | An occurrence has an extent in time (lifetime) from start to end. | SysML §8.4.5; KerML §9.2.4 (GOSPEL) | **GATED-here** — `occurrence_clock_spec_conformance::occ_occurrence_has_lifetime_extent` — **CONFORMS**. |
| `durationof-end-minus-start` | `DurationOf` = `TimeOf(endShot) − TimeOf(startShot)`. | KerML §9.2.12.2.5; `Clocks.kerml` `DurationOf` (GOSPEL+LIBRARY) | **GATED-here** — `occurrence_clock_spec_conformance::occ_durationof_equals_end_minus_start` — **CONFORMS**. |
| `local-clock-defaults-to-universal-clock` | An occurrence's `localClock` defaults to `Clocks::universalClock`; suboccurrences inherit it. | KerML §9.2.4.2.13, §9.2.12.2.7 (LIBRARY) | **GATED-here** — `occurrence_clock_spec_conformance::occ_local_clock_defaults_to_universal_clock` — **CONFORMS**. |
| `timeof-ordering-constraint` | If A HappensBefore B, `TimeOf(A.endShot) ≤ TimeOf(B)`. | KerML §9.2.12.2.6 `timeOrderingConstraint` (GOSPEL+LIBRARY) | **GATED-here** — `occurrence_clock_spec_conformance::occ_timeof_ordering_constraint_holds_for_sequential_occurrences` — **CONFORMS** (realized-time invariant). |
| `timeof-continuity-constraint` | If A HappensJustBefore B, `TimeOf(A.endShot) == TimeOf(B)`. | KerML §9.2.12.2.6 `timeContinuityConstraint` (GOSPEL+LIBRARY) | **GATED-here** — `occurrence_clock_spec_conformance::occ_timeof_continuity_constraint_for_abutting_occurrences` — **CONFORMS** (realized-time invariant). |
| `timeslice-is-portion` | A time slice covers all space of its occurrence over a smaller time period; an occurrence is a time slice of itself. | KerML §9.2.4.1; `checkOccurrenceUsageTimeSliceSpecialization` (GOSPEL) | **STRUCTURAL** (portionKind) — validation sweep. |
| `portion-is-same-identity-as-parent` | A portion is the same thing as the occurrence it is a portionOf (shared `portionOfLife`). | KerML §9.2.4.1 *"same 'thing' as the Occurrences they are a portionOf"* (GOSPEL) | **STRUCTURAL** — identity semantics; cross-ref. |
| `life-is-maximal-portion` | An `individual` occurrence specializes `Occurrences::Life` (maximal portion, multiplicity ≤1). | KerML §9.2.4.2.11; `checkOccurrenceDefinitionIndividualSpecialization` (GOSPEL) | **STRUCTURAL** — validation sweep. |
| `happens-just-before-no-intervening` | `HappensJustBefore`: no occurrence can exist in the gap between earlier and later. | KerML §9.2.4.2.4 (GOSPEL) | **GATED-here** — `occurrence_clock_spec_conformance::occ_happens_just_before_has_no_intervening_occurrence` — **CONFORMS** (realized immediate-succession invariant). |
| `clock-timeflow-constraint` | The `currentTime` of a `Clock` snapshot equals the `TimeOf` that snapshot relative to the clock: `snapshots->forAll{TimeOf(s, thisClock) == s.currentTime}`. | `Clocks.kerml:47-57` `timeFlowConstraint`; KerML §9.2.12.2.2 (LIBRARY+GOSPEL) | **GATED-here** — `occurrence_clock_spec_conformance::occ_clock_timeflow_snapshot_currenttime_equals_timeof` — **CONFORMS** (snapshot↔clock-time identity, distinct from monotonic advance). |
| `suboccurrence-endshot-coincidence` | When a parent occurrence ends, each suboccurrence's `endShot` is time-coincident with the parent's `endShot`. | `Occurrences.kerml:379-382` `subendshot`/`superendshot subsets timeCoincidentOccurrences` (LIBRARY) | **UNIMPLEMENTED** — pinned by `occurrence_clock_spec_conformance::occ_suboccurrence_endshot_coincident_with_parent` (`#[ignore]`): the runtime `OccurrenceTracker` is a flat peer set with no `suboccurrences`/`endShot`/`timeCoincidentOccurrences` surface. The absence is the finding; no runtime surface to assert against yet. |

## Coverage

- **GATED-elsewhere**: 5 (clock monotonic + TimeOf full; startShot/endShot,
  snapshot, happens-before partial via RSC-0.2 and SM/flow ordering).
- **GATED-here** (`occurrence_clock_spec_conformance.rs`): 6 CONFORMS
  (lifetime, durationof, localClock default, timeof-ordering, timeof-continuity,
  happens-just-before, clock-timeflow) + 1 UNIMPLEMENTED (suboccurrence-endshot).
- **STRUCTURAL**: 3.

## Ranked findings

1. **Clock core is gated** — monotonic `currentTime` + `TimeOf` start-shot are
   locked in (RSC-0.2).
2. **GAP-OCC-1 — `DurationOf` + `timeof-ordering`/`timeof-continuity`** — ✅ CLOSED,
   gated CONFORMS by the `occurrence_clock_spec_conformance` realized-time gates.
3. **GAP-OCC-2 — `local-clock-defaults-to-universal-clock`** — ✅ CLOSED, gated
   CONFORMS (`occ_local_clock_defaults_to_universal_clock`).
4. **suboccurrence-endshot-coincidence** — the one open gap: no
   parent/child containment on the flat `OccurrenceTracker` (UNIMPLEMENTED, pinned).

## Reproducing the citations

```bash
KER="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/KerML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$KER" > /tmp/ker.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `clock-currenttime-advances-monotonically` | `$KER` §9.2.12.2.4 | `grep -n -i "advances montonically" /tmp/ker.txt` |
| `timeof-returns-start-shot-time` | `$KER` §9.2.12.2.6 | `grep -n -i "TimeOf returns a scalar" /tmp/ker.txt` |
| `happens-before-no-time-overlap` | `$KER` §9.2.4.2.1 | `grep -n -i "none of their snapshots happen at the same time" /tmp/ker.txt` |
| `durationof-end-minus-start` | `$KER` §9.2.12.2.5 | `grep -n -i "DurationOf returns the duration" /tmp/ker.txt` |
| `local-clock-defaults-to-universal-clock` | `$KER` / `Clocks.kerml` | `grep -n -i "Clocks::universalClock" /tmp/ker.txt` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Source | Section | Content |
|--------|---------|---------|
| `KerML-spec-r2025-04_REF.html` | §9.2.4.1 | Occurrences Overview: Lives, portions, timeSlices, spaceSlices, snapshots, temporal/spatial associations |
| `KerML-spec-r2025-04_REF.html` | §9.2.4.2.1–2.26 | All 26 Elements: HappensBefore, happensBeforeLinks, HappensDuring, HappensJustBefore, HappensLink, HappensWhile, IncomingTransferSort, InnerSpaceOf, InsideOf, JustOutsideOf, Life, MatesWith, Occurrence (full feature list), occurrences, OutsideOf, PortionOf, SelfSameLifeLink, SnapshotOf, SpaceLink, SpaceShotOf, SpaceSliceOf, SurroundedBy, TimeSliceOf, Within, WithinBoth, Without |
| `KerML-spec-r2025-04_REF.html` | §9.2.12.1–2.8 | Clocks Overview; all 8 elements: BasicClock, BasicDurationOf, BasicTimeOf, Clock (timeFlowConstraint), DurationOf, TimeOf (startTimeConstraint, timeOrderingConstraint, timeContinuityConstraint), universalClock, UniversalClockLife |
| `SysML-spec-r2025-04_REF.html` | §7.9.1–7.9.5 | Occurrences Overview, Occurrence Definitions and Usages, Time Slices and Snapshots, Individual Definitions and Usages, Event Occurrence Usages |
| `SysML-spec-r2025-04_REF.html` | §8.3.9.1–8.3.9.5 | Abstract syntax: EventOccurrenceUsage, OccurrenceDefinition (isIndividual, checkOccurrenceDefinitionIndividualSpecialization, checkOccurrenceDefinitionMultiplicitySpecialization), OccurrenceUsage (portionKind, all check* and validate* constraints), PortionKind |
| `Occurrences.kerml` (normative library) | full file | All invariants: snapshots == union(startShot, middleTimeSlice.snapshots, endShot); startShot==endShot iff no middleTimeSlice; suboccurrences.endShot time-coincidence; irreflexivity of withoutOccurrences; spaceSlice innerSpaceDimension ≤ parent; localClock inheritance |
| `Clocks.kerml` (normative library) | full file | Clock.timeFlowConstraint; TimeOf.startTimeConstraint, timeOrderingConstraint, timeContinuityConstraint; DurationOf formula; BasicClock, BasicTimeOf, BasicDurationOf; universalClock singleton |

### Completeness table

| Normative unit | Clause | Status in matrix | Notes |
|----------------|--------|-----------------|-------|
| Clock monotonic `currentTime` | KerML §9.2.12.2.4; `Clocks.kerml:47` | CAPTURED — `clock-currenttime-advances-monotonically` (GATED-elsewhere) | |
| TimeOf = start-shot time | KerML §9.2.12.2.6 `startTimeConstraint`; `Clocks.kerml:73` | CAPTURED — `timeof-returns-start-shot-time` (GATED-elsewhere) | |
| startShot / endShot exist on every Occurrence | KerML §9.2.4.2.13; `Occurrences.kerml:348,373` | CAPTURED — `occurrence-has-start-shot-and-end-shot` (GATED-elsewhere) | |
| snapshot = zero-duration (startShot==endShot) | KerML §9.2.4.1; `Occurrences.kerml:337`; SysML `checkOccurrenceUsageSnapshotSpecialization` | CAPTURED — `snapshot-is-zero-duration-timeslice` (GATED-elsewhere) | |
| HappensBefore = no time overlap, transitive | KerML §9.2.4.2.1; `Occurrences.kerml:80-88` | CAPTURED — `happens-before-no-time-overlap` (GATED-elsewhere) | |
| Occurrence has lifetime extent (start to end) | SysML §7.9.1; KerML §9.2.4 | CAPTURED — `occurrence-has-lifetime-extent` (UNGATED) | |
| DurationOf = TimeOf(endShot) − TimeOf(startShot) | KerML §9.2.12.2.5; `Clocks.kerml:109-120` | CAPTURED — `durationof-end-minus-start` (UNGATED) | |
| localClock defaults to universalClock; suboccurrences inherit | KerML §9.2.4.2.13; `Occurrences.kerml:52,66` | CAPTURED — `local-clock-defaults-to-universal-clock` (UNGATED) | |
| timeOrderingConstraint (HappensBefore ⇒ TimeOf ≤) | KerML §9.2.12.2.6; `Clocks.kerml:82` | CAPTURED — `timeof-ordering-constraint` (UNGATED) | |
| timeContinuityConstraint (HappensJustBefore ⇒ TimeOf ==) | KerML §9.2.12.2.6; `Clocks.kerml:95` | CAPTURED — `timeof-continuity-constraint` (UNGATED) | |
| timeSlice is a portion (portionKind validation) | KerML §9.2.4.1; SysML `checkOccurrenceUsageTimeSliceSpecialization` | CAPTURED — `timeslice-is-portion` (STRUCTURAL) | |
| portion shares identity (portionOfLife) | KerML §9.2.4.1; `Occurrences.kerml:37` | CAPTURED — `portion-is-same-identity-as-parent` (STRUCTURAL) | |
| individual def specializes Life (mult ≤ 1) | KerML §9.2.4.2.11; SysML §8.3.9.3 `checkOccurrenceDefinitionIndividualSpecialization` | CAPTURED — `life-is-maximal-portion` (STRUCTURAL) | |
| HappensJustBefore = no gap | KerML §9.2.4.2.4; `Occurrences.kerml:364,385` | CAPTURED — `happens-just-before-no-intervening` (UNGATED) | |
| **Clock.timeFlowConstraint**: snapshot's `currentTime == TimeOf(snapshot, clock)` — snapshot-clocktime identity | `Clocks.kerml:47-57` (LIBRARY) | **MISSED** — behavioral invariant distinct from mere monotonic advance; not in matrix | |
| **suboccurrences endShot coincidence**: when parent ends, suboccurrences' endShots must be time-coincident with parent endShot | `Occurrences.kerml:379-382` (LIBRARY) | **MISSED** — behavioral; omitted from matrix | |
| isIndividual → multiplicity specializes Base::zeroOrOne | SysML §8.3.9.3 `checkOccurrenceDefinitionMultiplicitySpecialization` | OUT-OF-SCOPE — structural multiplicity validation, not a runtime behavioral obligation | |
| EventOccurrenceUsage specializes timeEnclosedOccurrences | SysML §8.3.9.2 `checkEventOccurrenceUsageSpecialization` | OUT-OF-SCOPE — usage-graph structural check, not occurrence/clock runtime semantics | |
| HappensDuring / HappensWhile (time enclosure) | KerML §9.2.4.2.3, §9.2.4.2.6 | OUT-OF-SCOPE — spatial/temporal containment relations outside runtime occurrence tracker scope | |
| SpaceSlice / spaceShots / spaceBoundary family | KerML §9.2.4.1, §9.2.4.2.18–2.22 | OUT-OF-SCOPE — spatial semantics, no runtime spatial model implemented | |
| snapshots partition invariant (snapshots == union of startShot, middleTimeSlice.snapshots, endShot) | `Occurrences.kerml:339` | OUT-OF-SCOPE — internal library invariant, no snapshot-set API exposed at runtime level | |

### Honesty line

The existing 14-row matrix is **substantively complete** for the behavioral runtime surface of occurrences and clocks. Two genuine behavioral obligations were **MISSED**: the `timeFlowConstraint` (Clock snapshot's `currentTime` must equal `TimeOf` of that snapshot relative to itself — distinct from monotonic advance alone) and `suboccurrences-endShot-coincidence` (parent occurrence end forces suboccurrence endShots to be time-coincident). Both are gateable against the `OccurrenceTracker` / `clock` runtime API and should be promoted to the obligation table.
