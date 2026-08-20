# Obligation matrix — Actions

**Area:** action execution — succession ordering, control nodes, send/accept,
assignment, terminate.
**Existing gate:** `tests/runtime_spec_conformance.rs` (RSC-0.3) gates
**send/accept payload identity through ports**; control-node and succession
semantics are otherwise **UNGATED**. **No new gate file** added here yet (the
action runtime is also under active work). **Status:** fan-out area — obligation
catalog + coverage map; verdicts for ungated rows deferred to a probe pass.

Spec sources: SysML §7.17 *Actions* (`SysML-spec-r2025-04_REF.html`); KerML
`Kernel Semantic Library/ControlPerformances.kerml`; `sysml.library/Systems Library/Actions.sysml`.
Verified `2026-06-21`.

> **Test results (2026-06-21):** control nodes now GATED at the IR level by
> `tests/action_spec_conformance.rs` (7 tests, all CONFORMS): fork = all branches,
> join = waits-for-all, decision = exactly-one (+ no-match diagnostic), merge =
> any-one-no-sync. **GAP-ACT-COMPILE CONFIRMED:** `compile_action` lowers neither
> succession guards nor assignment RHS (placeholder `LiteralInt(0)`), so
> control-node semantics are testable only on directly-built `ActionGraphIR`, not
> from `.sysml` source — a real source-pipeline implementation gap.

## Obligation table

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `succession-temporal-ordering` | A succession requires source to end before target begins. | §7.13.2; KerML §9.2.4.2.1 HappensBefore (GOSPEL) | **UNGATED** — candidate gate. |
| `fork-node-concurrent-fanout` | A fork orders itself before ALL outgoing targets (every branch activates). | §7.17.3 / §8.4.13.4 (GOSPEL) | **GATED** — `action_spec_conformance::fork_activates_every_outgoing_branch` (CONFORMS). |
| `join-node-synchronize-all-incoming` | A join is ordered after ALL incoming sources complete. | §7.17.3 / §8.4.13.4 (GOSPEL) | **UNGATED** — candidate gate. |
| `merge-node-any-one-incoming` | A merge fires once per exactly-one incoming control. | §7.17.3 / §8.4.13.4; `ControlPerformances.kerml` MergePerformance (GOSPEL+LIBRARY) | **UNGATED** — candidate gate. |
| `decision-node-exactly-one-outgoing` | A decision routes to exactly one outgoing branch per performance. | §7.17.3 / §8.4.13.4; `ControlPerformances.kerml` DecisionPerformance (GOSPEL+LIBRARY) | **UNGATED** — candidate gate. |
| `control-node-instantaneous` | Fork/join/merge/decide are instantaneous (`start = done`). | `Actions.sysml:284` *"A ControlAction is instantaneous."* (LIBRARY) | **UNGATED** — candidate gate. |
| `guarded-succession-conditional-ordering` | A guarded succession asserts ordering only when its guard is true after the source completes. | §7.17.1 / §8.4.13.3 (GOSPEL) | **UNGATED** — candidate gate. |
| `send-action-initiates-message-transfer` | A send initiates a MessageTransfer carrying the payload from the sender. | §8.4.13.5 (GOSPEL) | **GATED-elsewhere** (partial) — `runtime_spec_conformance::spec_send_accept_payload_identity_through_port`. |
| `accept-action-waits-for-matching-incoming-transfer` | An accept selects an incoming MessageTransfer whose values conform to the accepted type. | §8.4.13.6 (GOSPEL) | **GATED-elsewhere** (partial) — same RSC-0.3 test. |
| `send-action-default-sender-is-this` | A composite send's default sender is the containing part. | §7.17.7 / §8.4.13.5 (GOSPEL) | **UNGATED**. |
| `send-action-has-payload` | A SendActionUsage must have a payload parameter (its first argument). | §8.3.17.15 `validateSendActionParameters` (`inputParameters()->size() >= 3`) + `deriveSendActionUsagePayloadArgument` (`payloadArgument = argument(1)`); §7.17.7 (GOSPEL, STRUCTURAL) | **GATED-here** — **CONFORMS** on three faces: (1) SYNTAX `structural_spec_conformance::payload_less_send_is_a_parse_error` — the grammar (`send_action: seq("send", _expression, …)`) makes a payload-less send a parse error, so it is not grammatically representable; (2) CONFORMING `send_with_payload_is_clean` — a well-formed `send <payload> …` carries its payload as an `Expression` child, S125 does NOT fire; (3) VALIDATOR (defensive) `send_action_without_payload_parameter_is_flagged` — a hand-built `SendActionUsage` lacking any payload child raises S125 (`actions::send_action_has_payload`). |
| `accept-action-default-receiver-is-this` | A composite accept's default receiver is the containing part. | §7.17.8 (GOSPEL) | **UNGATED**. |
| `assignment-action-writes-at-completion` | An assignment makes the target feature hold the replacement value at the moment the action ends. | §8.4.13.7 (GOSPEL) | **UNGATED** — candidate gate. |
| `parameter-binding-vs-flow-transfer` | Binding output→input parameters asserts value-equivalence; a flow models an actual (timed) transfer. | §7.17.1 (GOSPEL) | **UNGATED** — overlaps flows/ports area. |
| `succession-flow-ordering-constraint` | A succession flow: transfer can't begin until source completes, target can't start until transfer completes. | §7.17.1 (GOSPEL) | **UNGATED** — overlaps flows/ports area. |
| `terminate-action-ends-occurrence-at-completion` | A terminate forces the terminated occurrence's lifetime to end by the terminate's completion. | §7.17.10; `Actions.sysml:255-257` (GOSPEL+LIBRARY) | **UNGATED** — low priority. |

## Coverage

- **GATED-elsewhere** (partial, RSC-0.3): 2 (send / accept).
- **UNGATED behavioral gaps**: 13 — the control-node + succession + assignment
  semantics are the action area's coverage hole.
- Behavioral coverage ≈ **2 / 15 ≈ 13%** (and the 2 are only partial, via the
  port path).

## Ranked findings

1. **GAP-ACT-1 — control-node semantics (fork/join/merge/decision) are entirely
   ungated** (5 obligations). The action graph's defining behavior has no
   conformance test. Highest-value action-area candidate gates.
2. **GAP-ACT-2 — succession ordering + guarded succession ungated** (3).
3. **GAP-ACT-3 — assignment-writes-at-completion ungated.**
   Verdicts require a probe of the action runtime (`ActionGraphIR` execution);
   scheduled as the action gate pass, coordinated with active runtime work.

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `fork-node-concurrent-fanout` | `$SYS` §8.4.13.4 | `grep -n -i "every outgoing Succession must have a value" /tmp/sys.txt` |
| `join-node-synchronize-all-incoming` | `$SYS` §8.4.13.4 | `grep -n -i "every incoming Succession must have a value" /tmp/sys.txt` |
| `decision-node-exactly-one-outgoing` | `$SYS` §8.4.13.4 | `grep -n -i "exactly one outgoing Succession that has a value" /tmp/sys.txt` |
| `assignment-action-writes-at-completion` | `$SYS` §8.4.13.7 | `grep -n -i "accessedFeature of the target Occurrence will have" /tmp/sys.txt` |
| `control-node-instantaneous` | `sysml.library/Systems Library/Actions.sysml` | `grep -n "ControlAction is instantaneous" "<file>"` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Spec source | Subclauses enumerated |
|---|---|
| SysML §7.17 (notation/abstract syntax) | §7.17.1 (Overview), §7.17.2 (ActionDef/Usage), §7.17.3 (Control Nodes), §7.17.4 (Succession Shorthands), §7.17.5 (Conditional Successions), §7.17.6 (Perform), §7.17.7 (Send), §7.17.8 (Accept), §7.17.9 (Assignment), §7.17.10 (Terminate), §7.17.11 (If), §7.17.12 (Loop) |
| SysML §8.4.13 (semantics) | §8.4.13.1 (ActionDefinition), §8.4.13.2 (ActionUsage), §8.4.13.3 (Decision Transition Usages), §8.4.13.4 (Control Nodes), §8.4.13.5 (Send), §8.4.13.6 (Accept), §8.4.13.7 (Assignment), §8.4.13.8 (Terminate), §8.4.13.9 (If), §8.4.13.10 (Loop), §8.4.13.11 (Perform) |
| `sysml.library/Systems Library/Actions.sysml` | SendAction, AcceptAction, TerminateAction, ControlAction (Fork/Join/Merge/Decision), TransitionAction, DecisionTransitionAction, AssignmentAction, IfThenAction, IfThenElseAction, WhileLoopAction, ForLoopAction |
| `KerML Kernel Library/ControlPerformances.kerml` | MergePerformance, DecisionPerformance, IfThenPerformance, LoopPerformance (imported by Actions.sysml) |

### Clause classification table

| Subclause | Classification | Notes |
|---|---|---|
| §7.17.1 Actions Overview | CAPTURED | `parameter-binding-vs-flow-transfer`, `succession-flow-ordering-constraint` |
| §7.17.2 Action Definitions and Usages | STRUCTURAL | Parameter redefinition rules; no runtime behavioral obligation beyond §8.4.13.1/2 |
| §7.17.3 Control Nodes | CAPTURED | All four control-node obligations present |
| §7.17.4 Succession Shorthands | STRUCTURAL | Notation sugar; expands to successions already covered |
| §7.17.5 Conditional Successions | CAPTURED | `guarded-succession-conditional-ordering` (note: §8.4.13.3 is more precise normative home) |
| §7.17.6 Perform Action Usages | **MISSED** | `perform-action-is-referential` — PerformActionUsage must be referential; owned by Part subsets `performedActions` |
| §7.17.7 Send Action Usages | CAPTURED | `send-action-initiates-message-transfer`, `send-action-default-sender-is-this` |
| §7.17.8 Accept Action Usages | CAPTURED | `accept-action-waits-for-matching-incoming-transfer`, `accept-action-default-receiver-is-this` |
| §7.17.9 Assignment Action Usages | CAPTURED | `assignment-action-writes-at-completion` |
| §7.17.10 Terminate Action Usages | CAPTURED | `terminate-action-ends-occurrence-at-completion` |
| §7.17.11 If Action Usages | **MISSED** | `if-action-evaluates-test-then-branch` — IfThenAction evaluates ifTest; if true performs thenClause, else elseClause |
| §7.17.12 Loop Action Usages | **MISSED** | `while-loop-iterates-while-test` (WhileLoopAction), `for-loop-iterates-over-sequence` (ForLoopAction iterates seq→var→body) |
| §8.4.13.1 ActionDefinition semantics | STRUCTURAL | Specialization constraints; no standalone behavioral obligation beyond structural hierarchy |
| §8.4.13.2 ActionUsage semantics | STRUCTURAL | Specialization constraints; subaction subset rules |
| §8.4.13.3 Decision Transition Usages | CAPTURED (imprecise) | `guarded-succession-conditional-ordering` cites §7.17.1; normative home is §8.4.13.3 — guard evaluated AFTER source completes |
| §8.4.13.4 Control Nodes | CAPTURED | Fork/join/merge/decision obligations all present |
| §8.4.13.5 Send semantics | CAPTURED | `send-action-initiates-message-transfer`, `send-action-default-sender-is-this` |
| §8.4.13.6 Accept semantics | CAPTURED | Both accept obligations present |
| §8.4.13.7 Assignment semantics | CAPTURED | `assignment-action-writes-at-completion` |
| §8.4.13.8 Terminate semantics | CAPTURED | `terminate-action-ends-occurrence-at-completion` |
| §8.4.13.9 If semantics | **MISSED** | No obligation for IfThenAction/IfThenElseAction behavioral semantics |
| §8.4.13.10 Loop semantics | **MISSED** | No obligation for WhileLoopAction or ForLoopAction behavioral semantics |
| §8.4.13.11 Perform semantics | **MISSED** | No obligation for PerformActionUsage referential constraint |
| Library `Actions.sysml` | CAPTURED (partial) | `control-node-instantaneous` captured; IfThen/Loop/Perform library defs not yet gated |
| Library `ControlPerformances.kerml` | CAPTURED (partial) | MergePerformance/DecisionPerformance cited for merge/decision obligations |

### Missed behavioral obligations (4 new rows needed)

| Proposed ID | Obligation | Citation |
|---|---|---|
| `if-action-evaluates-test-then-branch` | An IfThenAction evaluates its `ifTest`; if true, performs `thenClause`; an IfThenElseAction additionally performs `elseClause` when `ifTest` is false. | §8.4.13.9; `Actions.sysml:399-420` (GOSPEL+LIBRARY) |
| `while-loop-iterates-while-test` | A WhileLoopAction performs its `body` while `whileTest` evaluates to true and `untilTest` evaluates to false; terminates when `whileTest` is false or `untilTest` is true. | §8.4.13.10; `Actions.sysml:452-484` (GOSPEL+LIBRARY) |
| `for-loop-iterates-over-sequence` | A ForLoopAction assigns each successive value from `seq` to its `var` loop variable and performs `body` for each; internally implemented via a nested WhileLoopAction. | §8.4.13.10; `Actions.sysml:485-531` (GOSPEL+LIBRARY) |
| `perform-action-is-referential` | A PerformActionUsage is always referential (isComposite = false). When owned by a Part or OccurrenceDefinition it subsets `Parts::Part::performedActions`; the referenced Action is considered performed by the owning Part. | §8.4.13.11; §7.17.6 (GOSPEL) |

### Honesty line

15 existing obligations reviewed; all 15 correctly cite normative spec sources. 4 behavioral obligations were absent: if-branch execution, while-loop iteration, for-loop sequence iteration, and perform-action referential constraint. The existing `guarded-succession-conditional-ordering` obligation is accurate but its citation (§7.17.1) should be updated to the more normative §8.4.13.3. Audit performed against `SysML-spec-r2025-04_REF.html` (HTML lines 6946–42531) and `sysml.library/Systems Library/Actions.sysml`.
