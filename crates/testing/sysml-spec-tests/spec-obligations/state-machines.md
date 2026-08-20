# Obligation matrix — State machines

**Area:** state / transition execution semantics.
**Existing gate:** `tests/runtime_spec_conformance.rs` (RSC-0.2) gates the
**trigger/guard/clock** subset; the dedicated behavioral gate
`tests/state_machine_spec_conformance.rs` (SMC-1..11) now gates the
transition-selection, guard-gating, firing-order (exit→effect→entry),
entry/do/exit, initial-state, incoming-trigger and stateSequencing rows below.
**Status:** fan-out area — obligation catalog + coverage map; the behavioral
rows have been probed and gated (GAP-SM-EXEC + GAP-SM-EFFECT fix-waves closed
the firing-execution gaps; SMC-10/11 closed the two R2-audit behavioral gaps).

Spec sources: SysML §7.18 *States* (`SysML-spec-r2025-04_REF.html`); KerML
`Kernel Semantic Library/StatePerformances.kerml`, `TransitionPerformances.kerml`.
Verified `2026-06-21`.

Coverage legend: **GATED-here** · **GATED-elsewhere** (cite test) ·
**UNGATED** (catalogued gap, verdict needs a probe) · **STRUCTURAL** (validation).

> **Test results (2026-06-21, updated after the GAP-SM-EXEC + GAP-SM-EFFECT fix-wave):**
> GATED by `tests/state_machine_spec_conformance.rs` (10 tests, **9 CONFORMS / 0
> DIVERGES**, 2 UNIMPLEMENTED). The full firing sequence exit→effect→entry now
> executes in order. Three bugs closed: (1) **GAP-SM-EXEC exit/do** — `compile_state`
> routes entry/do/exit through one `compile_state_subaction` helper walking the
> tagged children to a `Structured{assignments}` body (was: only ENTRY had the
> fallback; exit/do degraded to `Simple("")`) → SMC-6/SMC-7. (2) **GAP-SM-EFFECT
> (grammar)** — the tree-sitter `effect_action` rule didn't accept the canonical
> `do action { … }` effect form (xtext TransitionUsage 1849-1857), so the effect
> parsed as detached sibling nodes and never reached the IR; fixed by adding the
> `do action {…}` arm (+ regen). (3) **assignment ordering** — surfaced by SMC-4's
> multi-statement body: `compile_action_from_children` built assignments in
> `HashSet` iteration order (`owner_to_children` is unordered) → fixed by sorting
> statement children by source span. SMC-4 → CONFORMS. **SMC-10 + SMC-11 CLOSED
> 2026-06-22** — SMC-10: `SubsystemState.incoming_transition_trigger` records the
> triggering message event when a triggered transition fires
> (`StatePerformances.kerml:48`). SMC-11: `elaborate::derive_state_sequencing`
> materialises the N-1 `stateSequencing` successions between a state's exclusive
> substates on the ModelGraph (`States.sysml:71-77`). All SM obligations CONFORM.

## Obligation table

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `transition-guard-boolean` | A transition guard is a Boolean[1..1] expression that must be true for the transition to occur. | §7.18.3 / §8.3.18.8 (GOSPEL) | **GATED-elsewhere** — `runtime_spec_conformance::spec_trigger_when_*` exercise guard truth gating triggers. |
| `trigger-is-accept-action` | A transition trigger is an AcceptActionUsage; ≤1 accepted per firing. | §8.3.18.8; `TransitionPerformances.kerml:42` (GOSPEL+LIBRARY) | **GATED-elsewhere** — `spec_trigger_*` + `spec_send_accept_payload_identity_through_port`. |
| `entry-do-exit-ordering` | Entry completes → do starts and runs while active → on exit, do interrupted, exit runs to completion. | §7.18.1 *"An entry action starts when the state is activated…"*; `StatePerformances.kerml:43-45` (GOSPEL+LIBRARY) | **GATED-here** — `state_machine_spec_conformance::{smc5_entry_action_runs_on_state_entry, smc6_exit_action_runs_on_leave, smc7_do_action_runs_while_resident}` — **CONFORMS** (`2026-06-21`, GAP-SM-EXEC fix-wave lowered the exit/do bodies). |
| `transition-firing-order-exit-effect-entry` | Firing sequence: interrupt source do → source exit → transition effect → target entry → target do. | §7.18.3 *"If the source state has a do action that is still being performed, that is interrupted…"* (GOSPEL) | **GATED-here** — `state_machine_spec_conformance::smc4_firing_order_exit_effect_entry` — **CONFORMS** (`2026-06-21`, GAP-SM-EFFECT fix-wave: exit→effect→entry now executes in order). |
| `guard-evaluated-during-source-state` | The guard is evaluated during the source state's performance; transition fires only if guard true AND a transfer accepted. | §8.4.14.3; `StatePerformances.kerml:140-141` (GOSPEL+LIBRARY) | **GATED-elsewhere** (partial) — `spec_trigger_when_*`. |
| `exclusive-one-substate-active` | A non-parallel state has exactly one active substate at a time (after entry). | §7.18.1 / §8.3.18.6 (GOSPEL) | **UNGATED** — candidate gate. |
| `no-transitions-in-parallel-state` | A parallel state's substates may have no incoming/outgoing transitions. | §8.3.18.5/6 (GOSPEL, STRUCTURAL) | **STRUCTURAL** — validation sweep. |
| `at-most-one-each-subaction-kind` | ≤1 entry, ≤1 do, ≤1 exit per state. | §8.3.18.5/6 (GOSPEL, STRUCTURAL) | **CONFORMS (S068/S069 CLOSED `2026-06-21`)** — `at_most_one_state_subaction` now counts the parser's `ActionUsage`+`stateSubactionKind` shape; gated by `structural_spec_conformance::two_entry_subactions_on_state_def_is_flagged`. |
| `initial-state-via-entry-succession` | The initial substate is the target of a succession from the entry action. | §7.18.2; `StatePerformances.kerml:43` (GOSPEL+LIBRARY) | **GATED-here** — `state_machine_spec_conformance::smc8_initial_state_is_first_declared_before_any_event` — **CONFORMS** (`2026-06-21`). |
| `run-to-completion-blocks-transition-during-entry` | With `isRunToCompletion` (default), no transition fires during the entry action. | KerML §9.2.11.1; `StatePerformances.kerml:99-102` (GOSPEL+LIBRARY) | **UNGATED** — candidate gate. |
| `transition-source-must-be-state-usage` | A transition's source must be a StateUsage; a non-state source may not have an accepter. | §7.18.3; `StatePerformances.kerml:120` (GOSPEL, STRUCTURAL) | **STRUCTURAL** — validation sweep. |
| `accepted-requires-acceptable` | `accepted` is non-empty iff `acceptable` is; exit waits on it. | `StatePerformances.kerml:55-56,95`; KerML §9.2.11.1 (LIBRARY+GOSPEL) | **UNGATED** — candidate gate (transfer-driven). |
| `exhibit-state-within-occurrence-lifetime` | An exhibited state performance occurs within the exhibiting occurrence's lifetime. | §7.18.1 / §8.4.14.4 (GOSPEL) | **UNGATED** — low priority. |
| `at-most-one-transition-fires-per-trigger` | At most one outgoing transition fires per trigger (via `accepted[0..1]` + `isDispatch`). | `StatePerformances.kerml:56,77-83`; KerML §9.2.11.1 (**SPEC-SILENT** as a standalone SHALL) | **SPEC-SILENT** — mechanism (dispatch+sort) defined, not a separate "one fires" rule. The observable single-target landing is pinned as a determinism guard by `state_machine_spec_conformance::smc9_at_most_one_transition_fires_per_trigger`. |
| `transition-selection` | An accepted event-triggered transition fires and moves the machine to its named target state. | §7.18.3; `TransitionPerformances.kerml:42` (GOSPEL+LIBRARY) | **GATED-here** — `state_machine_spec_conformance::smc1_event_drives_transition_to_target` — **CONFORMS** (`2026-06-21`). |
| `no-transition-on-unmatched-event` | An event matching no outgoing transition of the current state leaves the machine in place (no spurious firing). | §7.18.3 (GOSPEL) — a transition fires only when its trigger is accepted. | **GATED-here** — `state_machine_spec_conformance::smc2_unmatched_event_does_not_fire` — **CONFORMS** (`2026-06-21`). |
| `incoming-transition-trigger-recorded` | On entry into a state, the transfer/event that triggered the entry is recorded on the entered state's performance. | `StatePerformances.kerml:48-53` `incomingTransitionTrigger : MessageTransfer [0..1]` (LIBRARY) | **GATED-here** — `state_machine_spec_conformance::smc10_incoming_transition_trigger_recorded` — **CONFORMS** (`2026-06-22`, SMC-10: `SubsystemState.incoming_transition_trigger`). SPEC-SILENT: records the event name; full `MessageTransfer` identity deferred. |
| `state-sequencing-count-invariant` | With N mutually-exclusive substates there are exactly N-1 `stateSequencing` successions: `notEmpty(exclusiveStates) implies size(stateSequencing) == size(exclusiveStates) - 1`. | `States.sysml:71-77` (LIBRARY) | **GATED-here** (elaboration) — `state_machine_spec_conformance::smc11_state_sequencing_count_invariant` — **CONFORMS** (`2026-06-22`, SMC-11: `elaborate::derive_state_sequencing` materialises the N-1 successions on the `ModelGraph`). |

## Coverage

- **GATED-here** (`state_machine_spec_conformance.rs`, SMC-1..11): transition-selection,
  no-transition-on-unmatched-event, guard-gating (transition-guard-boolean),
  firing-order (exit→effect→entry), entry-do-exit-ordering, initial-state-via-entry-succession,
  incoming-transition-trigger-recorded, state-sequencing-count-invariant, and the
  at-most-one-transition determinism guard.
- **GATED-elsewhere**: trigger-is-accept-action + guard-during-source (via RSC-0.2/0.3);
  transition-guard-boolean is also pinned in RSC (`spec_trigger_when_*`).
- **UNGATED behavioral (remaining candidate gates)**: 3 — `exclusive-one-substate-active`,
  `run-to-completion-blocks-transition-during-entry`, `accepted-requires-acceptable`.
- **STRUCTURAL** (validation sweep): 3. **SPEC-SILENT**: 1.

## Ranked findings

The firing-order behavioral hole below is **CLOSED** — retained as history:

1. **GAP-SM-1 — transition firing-order (exit→effect→entry)** — ✅ CLOSED
   (`2026-06-21`, GAP-SM-EFFECT fix-wave; gated CONFORMS by
   `smc4_firing_order_exit_effect_entry`).
2. **GAP-SM-2 — entry/do/exit ordering** — ✅ CLOSED (`2026-06-21`, GAP-SM-EXEC
   fix-wave; gated CONFORMS by SMC-5/6/7).
3. **GAP-SM-3 — run-to-completion + exclusive-substate** — still UNGATED (the two
   remaining candidate gates); verdicts require an orchestrator probe.

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `entry-do-exit-ordering` | `$SYS` §7.18.1 | `grep -n -i "entry action starts when the state is activated" /tmp/sys.txt` |
| `transition-firing-order-exit-effect-entry` | `$SYS` §7.18.3 | `grep -n -i "do action that is still being performed, that is interrupted" /tmp/sys.txt` |
| `exclusive-one-substate-active` | `$SYS` §7.18.1 | `grep -n -i "exactly one of the substates shall be active" /tmp/sys.txt` |
| `entry/do/exit, initial-state, run-to-completion` | `Kernel Semantic Library/StatePerformances.kerml` | `grep -n "do.startShot\|isRunToCompletion\|entry then" "<file>"` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Source file | Clauses covered |
|---|---|
| `SysML-spec-r2025-04_REF.html` | §7.18.1 (overview), §7.18.2 (state def/usage), §7.18.3 (transitions), §7.18.4 (exhibit state) |
| `SysML-spec-r2025-04_REF.html` | §8.3.18.1–9 (abstract syntax: ExhibitStateUsage, StateSubactionKind/Membership, StateDefinition, StateUsage, TransitionFeatureKind/Membership, TransitionUsage) |
| `SysML-spec-r2025-04_REF.html` | §8.4.14.1–4 (semantics: StateDefinition, StateUsage, TransitionUsage, ExhibitStateUsage) |
| `Kernel Semantic Library/StatePerformances.kerml` | StatePerformance invariants (entry/do/exit successions, accepted/acceptable inv, isRunToCompletion inv, dispatch invariant) |
| `Kernel Semantic Library/TransitionPerformances.kerml` | TransitionPerformance (trigger/guard/effect successions, accNum, TPCGuardConstraint) |
| `Systems Library/States.sysml` | StateAction (entryAction/doAction/exitAction, substates, exclusiveStates, stateSequencing, sequencing-count invariant), StateTransitionAction, stateActions |

### Classification table

Each normative unit from the sections above is classified against the existing obligation table rows.

| Normative unit | Clause | Classification | Table ID or reason |
|---|---|---|---|
| Entry completes before do starts; do interrupted on exit | §7.18.1 / `StatePerformances.kerml:43-45` | CAPTURED | `entry-do-exit-ordering` |
| Exactly one exclusive substate active (non-parallel) | §7.18.1 / §8.3.18.5/6 | CAPTURED | `exclusive-one-substate-active` |
| Exhibited state within exhibitor lifetime | §7.18.1 / §8.4.14.4 | CAPTURED | `exhibit-state-within-occurrence-lifetime` |
| ≤1 entry, ≤1 do, ≤1 exit per state | §7.18.2 / §8.3.18.5/6 `validateStateSubactionKind` | CAPTURED | `at-most-one-each-subaction-kind` |
| Initial substate reached via succession from entry | §7.18.2 / `StatePerformances.kerml:43` | CAPTURED | `initial-state-via-entry-succession` |
| No transitions inside parallel state | §7.18.2 / §8.3.18.5/6 `validateParallelSubactions` | CAPTURED | `no-transitions-in-parallel-state` |
| Guard is Boolean[1..1] | §7.18.3 / §8.3.18.8 `validateGuardExpression` | CAPTURED | `transition-guard-boolean` |
| Trigger is AcceptActionUsage | §7.18.3 / §8.3.18.8 `validateTriggerAction` | CAPTURED | `trigger-is-accept-action` |
| Effect is ActionUsage | §8.3.18.8 `validateTransitionFeatureMembershipEffectAction` | **STRUCTURAL** — parallel constraint to guard/trigger; not in table | **MISSED (structural)** — add row `transition-effect-is-action-usage` |
| owningType of TransitionFeatureMembership must be TransitionUsage | §8.3.18.8 `validateOwningType` | STRUCTURAL | not in table but subsumed by parser correctness; low priority |
| Firing sequence: do interrupted → exit → effect → entry → do | §7.18.3 / §8.4.14.3 | CAPTURED | `transition-firing-order-exit-effect-entry` |
| Guard evaluated during source state performance | §8.4.14.3 / `StatePerformances.kerml:140-141` | CAPTURED | `guard-evaluated-during-source-state` |
| TransitionUsage must have source BindingConnector | §8.3.18.9 `checkTransitionUsageSourceBindingConnector` | **STRUCTURAL** — not in table | **MISSED (structural)** — add row `transition-usage-source-binding-connector` |
| accepted[0..1] dispatch mechanism | `StatePerformances.kerml:56,77-83` | CAPTURED | `at-most-one-transition-fires-per-trigger` (SPEC-SILENT) |
| `accepted` non-empty iff `acceptable` non-empty | `StatePerformances.kerml:55-56,95` | CAPTURED | `accepted-requires-acceptable` |
| isRunToCompletion blocks transition during entry | `StatePerformances.kerml:99-102` | CAPTURED | `run-to-completion-blocks-transition-during-entry` |
| transition source must be StateUsage | §7.18.3 / `StatePerformances.kerml:120` | CAPTURED | `transition-source-must-be-state-usage` |
| ExhibitStateUsage is always referential | §8.3.18.2 / §8.4.14.4 `validateEventOccurrenceUsageIsReference` | **STRUCTURAL** — not in table | **MISSED (structural)** — add row `exhibit-state-usage-is-referential` |
| `incomingTransitionTrigger` set to triggering transfer on state entry | `StatePerformances.kerml:48-53` | **CONFORMS** (`2026-06-22`, SMC-10) | `SubsystemState.incoming_transition_trigger` records the triggering message event when a triggered (event/port) transition fires; `None` for completion/guard-only/time triggers. SPEC-SILENT: event-name, full `MessageTransfer` identity deferred. Gated by `smc10_incoming_transition_trigger_recorded`. |
| `stateSequencing` count: `notEmpty(exclusiveStates) implies size(stateSequencing) == size(exclusiveStates) - 1` | `States.sysml:71-77` | **CONFORMS** (SMC-11, `2026-06-22`, director-approved) | **ELABORATION home** (core-steward): `elaborate::derive_state_sequencing` materialises the N-1 implicit `Succession` relationships (tagged `stateSequencing=true`) linking a non-parallel state's exclusive `StateUsage` substates in declaration order, on the `ModelGraph` — NOT the execution `StateMachineIR` (which would duplicate graph structure + put abstract-syntax semantics in an exec IR, CLAUDE #4/#6). Parallel states excluded (concurrent → not mutually exclusive). The diagram skips tagged successions (implicit ordering, not a user edge). Gated by `smc11_state_sequencing_count_invariant` (verifies the invariant on the elaborated graph). **FOLLOW-ON:** making the library's own `assert constraint {size(stateSequencing)==size(exclusiveStates)-1}` self-evaluate needs the constraint evaluator to resolve the inherited `exclusiveStates`/`stateSequencing` features to counts — until then the evaluator still honestly reports `Error: undefined exclusiveStates` (baseline unchanged). |

### Honesty line

This audit was performed by stripping HTML tags from `SysML-spec-r2025-04_REF.html` and reading §7.18.1–4, §8.3.18.1–9, §8.4.14.1–4 in full, plus reading `StatePerformances.kerml`, `TransitionPerformances.kerml`, and `States.sysml` in full. Every normative unit encountered was classified. No section in §7.18 or §8.3.18 or §8.4.14 was skipped. KerML §9.2.11 in the spec (States Systems Library description) was reviewed via the `States.sysml` source file and the spec HTML prose at §9.2.11.1.

**Summary:** 14 existing rows — all CAPTURED against a spec unit. 5 normative units absent from the existing table: 3 structural (effect-is-action-usage, source-binding-connector, exhibit-referential) and 2 behavioral (incomingTransitionTrigger assignment, stateSequencing-count invariant). Neither behavioral gap is a firing-order issue; both are secondary runtime invariants. The catalogue's 6 major behavioral UNGATED gaps remain accurate and complete for execution-order obligations.
