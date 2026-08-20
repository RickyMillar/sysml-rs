# Spec-derived semantic-conformance obligation tracker

This directory holds the **obligation → gating-test coverage matrix** for the
runtime's *semantic* conformance to the SysML v2 / KerML specification, one
markdown file per semantic area.

It **complements, does not duplicate**, the crate's existing coverage:

| Existing coverage (this crate) | What it measures |
|---|---|
| `src/element_coverage.rs` | which **ElementKind** variants the parser produces |
| `src/corpus.rs`, `tests/corpus_tests.rs` | whether real corpus `.sysml` files **parse** |
| `src/operators.rs`, `src/treesitter_validation.rs` | tree-sitter node/operator coverage vs spec |
| **this tracker + `tests/*_spec_conformance.rs`** | whether runtime **behavior** matches spec **obligations** |

Parsing coverage answers "can we read it?"; this tracker answers "do we mean
the right thing when we run it?".

## Source precedence (spec-doc-first)

Obligations are derived in the order mandated by the root `CLAUDE.md`
("How to Research → Source precedence"):

1. OMG spec **DOCUMENT** (`SysML-spec-r2025-04_REF.html`,
   `KerML-spec-r2025-04_REF.html`) — **gospel**.
2. Normative standard **model library** (`sysml.library/.../*.sysml`,
   `.../*.kerml`) — these models *are* the semantics of library constructs.
3. Metamodel TTL → 4. Xtext grammar.
5. Pilot examples and our `examples/` corpus are **fallible illustration** —
   never the source of an obligation.

Every obligation row cites the **highest** source that establishes it, and the
citation prose is **verified against the spec HTML** (tag-stripped + grepped)
before it is written down. "Byte-identical on corpus" is a no-regression
signal, **not** conformance proof.

### Citation reproducibility (required)

A quoted sentence in a matrix is only trustworthy if anyone can re-derive it.
So every area file ends with a **"Reproducing the citations"** block listing,
per source, the exact file path and the grep term used to locate each quote, plus
the one-time tag-strip recipe:

```bash
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" <spec.html> > /tmp/spec.txt
grep -n -i "<grep-term>" /tmp/spec.txt
```

The raw tag-stripped text is intentionally **not** committed (it is an 8–12 MB
derivative); the file path + grep term is the durable, reproducible pointer.

## How an obligation is recorded

Each area file is a table of obligations. For each obligation:

- **ID** — kebab-case, stable; the gating test references it in an `// OBL:` line.
- **Obligation** — one normative sentence.
- **Citation** — clause + the verified quoted sentence, or `library file:construct`.
- **Tier** — `GOSPEL` (spec prose) · `LIBRARY` (normative model) · `STRUCTURAL`
  (well-formedness only) · `SPEC-SILENT` (spec defines structure but not the
  behavioral/runtime protocol — *not a gateable conformance obligation*).
- **Gate** — the test that gates it (`tests/<file>::<fn>`), or `—` if ungated.
- **Verdict** — `CONFORMS` · `DIVERGES — <why>` · `UNIMPLEMENTED` ·
  `DEFERRED — <where>` (gated elsewhere / another area / in-flight build).

## Coverage metric

Coverage is reported two ways, for honesty:

- **Behavioral coverage** = gated ÷ (behaviorally-gateable obligations).
  Behaviorally-gateable excludes `SPEC-SILENT` and pure-`STRUCTURAL`
  obligations (there is no normative behavior to assert).
- **All-obligation coverage** = gated ÷ (all obligations in the area), counting
  the silent/structural ones in the denominator.

Silent/structural obligations are **never silently dropped** — they are listed
with their tier so the gap between "we have a test" and "the spec says
something" is visible.

## Gating-test convention

Gating tests live in `tests/<area>_spec_conformance.rs` and follow the
`runtime_spec_conformance.rs` convention: pure-runtime, one obligation per test,
a `// VERDICT:` marker line per case, an `// OBL:` line tying it to this tracker,
and a self-scanning `*_matrix_summary` test that prints the verdict counts.

## Status

| Area | Tracker | Gate | Status |
|---|---|---|---|
| Constraints & expressions | `constraints-expressions.md` | `constraint_*` + `assert_constraint_spec_conformance.rs` | **DONE+GATED** — GAP-1 CLOSED (negation now applied; 4 ex-DIVERGES → CONFORMS) |
| Requirements | `requirements.md` | `requirement_spec_conformance.rs` | **DONE** — satisfaction logic gated; subject binding DEFERRED to in-flight build |
| Calculations | `calculations.md` | `calculation_spec_conformance.rs` | **DONE** — eval obligations gated |
| State machines | `state-machines.md` | `state_machine_spec_conformance.rs` (+RSC-0.2) | **DONE+GATED** — GAP-SM-EXEC exit/do CLOSED `2026-06-21` (2 ex-DIVERGES → CONFORMS); GAP-SM-EFFECT (grammar) remains (1 DIVERGES) |
| Actions | `actions.md` | `action_spec_conformance.rs` (+RSC-0.3) | **DONE+GATED** — control-nodes CONFORM (IR-level); GAP-ACT-COMPILE |
| Flows / ports | `flows-ports.md` | `runtime_spec_conformance.rs` (RSC-0.1/0.3) | **DONE** — 90% behavioral via cross-ref; GAP-FLOW-1 |
| Verification / analysis cases | `verification-analysis-cases.md` | `verification_verdict_spec_conformance.rs` | **DONE (verdict semantics)** — case pipeline DEFERRED to in-flight build |
| Occurrences / clocks | `occurrences-clocks.md` | `occurrence_clock_spec_conformance.rs` (+RSC-0.2) | **DONE+GATED** — all CONFORM |
| ODE / physics | `ode-physics.md` | `quantity_spec_conformance.rs` | **DONE+GATED** — dim-consistency ENFORCED; 2 DIVERGES |
| **Structural well-formedness** | _(cross-area)_ | `structural_spec_conformance.rs` | **DONE+GATED** — validator exists but base-specialization unowned; 7 UNIMPLEMENTED |

**Fan-out COMPLETE + GATED** (all 9 areas + structural). 11 gate suites
(`{constraint,requirement,calculation,assert_constraint,state_machine,action,
occurrence_clock,quantity,verification_verdict,verification_case,structural}_
spec_conformance.rs`) plus cross-reference to `runtime_spec_conformance.rs`.

## ⚠ Conformance tally (headline) — `2026-06-21`

Conformance gaps are encoded as `#[ignore]`d tests asserting the **spec-correct**
expectation: a plain `cargo test` is green and shows them **ignored/pending**;
`cargo test -- --ignored` runs them and they **fail against current behavior**
(the proof a gap is real); closing a gap = delete the `#[ignore]`.

| Verdict | Count | Meaning |
|---|---|---|
| **CONFORMS** (green) | **84** | obligation gated and the engine satisfies it (+4 GAP-1, +2 GAP-SM-EXEC, +1 GAP-PHYS Q5, +1 GAP-SM-EFFECT, +1 GAP-PHYS Q4, +2 GAP-VER no-criteria, +1 GAP-STRUCT S068/S069, +1 GAP-STRUCT S064/S065, +1 GAP-STRUCT subject-first-param S141-S144, +1 GAP-STRUCT port-referential S145/S146 — all `2026-06-21`; +1 SMC-10 incomingTransitionTrigger, +1 SMC-11 stateSequencing, +1 GAP-VER-ANALYSIS analysis-case objective→result FORM-B Tier-1 — `2026-06-22`) |
| **DIVERGES** (ignored) | **0** | every gated obligation whose behavior was *wrong* vs spec has been fixed this session |
| **UNIMPLEMENTED** (ignored) | **3** | no engine surface — all 3 are no-surface/spec-silent documented absences that PASS (ODE numerical solving, perform-referential, suboccurrence containment). The analysis-case objective→result gap (the last `--ignored` failure) is CLOSED `2026-06-22` (FORM-B Tier-1). |
| **SATISFIED-BY-CONSTRUCTION** | **3** | met by elaboration, gated in `sysml-core` not the spec-tests suite (the 3 base-specialization obligations — reclassified `2026-06-21`) |
| _(self-scan summaries)_ | 11 | one infra test per suite |

**BACKLOG: 0 buildable conformance gaps remain** (0 DIVERGES; every behavioral
divergence and every buildable structural/runtime obligation is closed). **SMC-11
stateSequencing-count was CLOSED `2026-06-22`** (director-approved): elaboration
(`derive_state_sequencing`) materialises the N-1 successions on the ModelGraph.
The single remaining `#[ignore]`d gate is director-gated:
- **analysis-case objective→result (§7.23.2)** — **DEFERRED-TO-DIRECTOR**: it is
  the base-`Case` `subject subj default Case::result` binding (`Cases.sysml:46`),
  i.e. the same result-binding machinery as the director-gated **Inc2b**; building
  it standalone would duplicate that path (CLAUDE #3/#5). Design plan for the
  eventual FORM-B Tier-3 implementation is in progress (director-requested).

The 3 base-specialization obligations are **SATISFIED-BY-CONSTRUCTION**
(elaboration's implicit-generalization derives the library base; gated by
`implicit_generalization.rs`, not a validator). "All suites green" does NOT mean
conformant. Run `cargo test -p sysml-spec-tests -- --ignored` to see the 1
director-gated gate fail.
**Closed `2026-06-21`:** GAP-1 (negated-assert — `ConstraintIR.is_negated`,
SysML §7.20); GAP-SM-EXEC exit+do halves (`compile_state` walks the tagged
exit/do children to Structured bodies, SysML §7.18.1); GAP-PHYS Q5 (Interpolate
null-OOB); **GAP-SM-EFFECT** — the full state-machine firing sequence
exit→effect→entry now executes (grammar `effect_action` accepts `do action {…}`
so the transition effect attaches; `compile_action_from_children` sorts
statement children by source span since `children_of` is unordered), SysML
§7.18.3. Landing it required regenerating `parser.c`, which surfaced PRE-EXISTING
grammar-source-vs-baseline drift (commits `13b6886f` quantity `unit`-prop +
`0d0850bf` accept-action) — director-authorized + reconciled separately (all
improvements: 7 pilot `ts_only_paths` ↓, `unit` grounded, CoreODE spurious-import
removed).

## Completeness audit (R2) — is the denominator closed?

Each area matrix has a `## Completeness audit — clauses reviewed` section that
enumerates the spec subclauses examined and classifies every normative unit
(CAPTURED / STRUCTURAL / OUT-OF-SCOPE / **MISSED**), so the catalogue is provably
complete rather than self-defined.

| Area | Denominator | Newly-found behavioral obligations (were MISSED) |
|---|---|---|
| Constraints & expressions | **CLOSED** (MISSED 0) | — |
| Requirements | **CLOSED** (MISSED 0) | — |
| Calculations | **CLOSED** (MISSED 0) | — |
| Flows / ports | **CLOSED** (MISSED 0) | — |
| Verification / analysis | **CLOSED** (MISSED 0) | — |
| ODE / physics | **CLOSED** (MISSED 0) | — |
| **Actions** | was incomplete | **+4**: IfThenAction, WhileLoopAction, ForLoopAction, PerformActionUsage-referential (§8.4.13.9–11) |
| **State machines** | was incomplete | **+2**: incomingTransitionTrigger, stateSequencing count invariant |
| **Occurrences / clocks** | was incomplete | **+2**: Clock.timeFlowConstraint, suboccurrence endShot-coincidence |

**8 newly-found behavioral obligations — now GATED** (commit `16fb53c3`):
- **4 CONFORM**: `if-action`, `while-loop`, `for-loop` (the runtime HAS
  `ActionNodeIR::If/WhileLoop/ForLoop` — control flow works at the IR level), and
  `clock-timeflow-constraint`.
- **2 real UNIMPLEMENTED gaps** (fail under `--ignored`): SM `incoming-transition-trigger`
  (no surface records the triggering transfer on entry) and `state-sequencing-count`
  (no `stateSequencing` structure in `StateMachineIR`).
- **2 no-surface documented absences**: `perform-action-referential` (structural,
  no action-runtime flag), `suboccurrence-endshot-coincidence` (tracker has no
  parent/child containment).

So the audit closed the catalogue AND the new obligations are all gated: the
actions control-flow hole turned out to be CONFORMANT, and 2 new real gaps were
added to the backlog (now 21).

## Roll-up (all areas) — empirical, from running the gates

| Area | Gate suite | Tests | Result |
|---|---|---|---|
| Constraints & expressions | constraint + assert_constraint | 9+8 | 6 + (**7 CONFORMS, 0 DIVERGES** — GAP-1 CLOSED `2026-06-21`) |
| Requirements | requirement | 6 | all CONFORMS; subject binding deferred |
| Calculations | calculation | 5 | all CONFORMS |
| State machines | state_machine | 11 | **11 CONFORMS, 0 DIVERGES, 0 UNIMPL** — all SM obligations conform (GAP-SM-EXEC + GAP-SM-EFFECT `2026-06-21`; SMC-10 incomingTransitionTrigger + SMC-11 stateSequencing-count `2026-06-22`) |
| Actions | action | 7 | 6 CONFORMS (control-nodes); GAP-ACT-COMPILE documented |
| Flows / ports | runtime_spec_conformance | (RSC-0.1/0.3) | ~90% gated; GAP-FLOW-1 ungated |
| Verification / analysis | verification_verdict + verification_case | 10+7 | verdict semantics CONFORM; case-level honest-Inconclusive CONFORMS; **verdict-criteria-modeled-explicitly CLOSED `2026-06-21`** (no-criteria → Inconclusive, was vacuous Pass); **analysis-case objective→result CLOSED `2026-06-22`** (FORM-B Tier-1: literal result bound to objective subject, verdict via the one engine) |
| Occurrences / clocks | occurrence_clock | 7 | all CONFORMS |
| ODE / physics | quantity | 11 | **9 CONFORMS, 0 DIVERGES** (Q5 Interpolate-null + Q4 decreasing-domain both CLOSED `2026-06-21`), 1 UNIMPL (solving SPEC-SILENT) |
| Structural well-formedness | structural | 11 | **9 CONFORMS, 0 UNIMPLEMENTED** — backlog fully closed (S068/S069 + S064/S065 + subject-first-param/S141-S144 + port-referential/S145-S146 CLOSED; 3 base-specialization reclassified SATISFIED-BY-CONSTRUCTION & removed — all `2026-06-21`) |

### Confirmed implementation gaps (found by running the gates)

1. **GAP-1 (constraints) — ✅ CLOSED `2026-06-21`.** `assert not constraint`
   negation was dropped (`ConstraintIR` had no `isNegated`). Fixed: added
   `ConstraintIR.is_negated`, set from the `AssertConstraintUsage` element's
   `isNegated` at both extraction paths, and inverted the decided verdict at the
   `evaluate_expr` chokepoint (so the eval layer AND the per-instance check path
   are corrected). The 4 ex-DIVERGES tests in
   `assert_constraint_spec_conformance.rs` now CONFORM. SysML §7.20 (GOSPEL).
2. **GAP-SM-EXEC + GAP-SM-EFFECT (state machines) — ✅ BOTH CLOSED `2026-06-21`.**
   Exit/do action *bodies* were dropped (compiled to `Simple("")`, only ENTRY
   executed): fixed by routing all three subactions through one
   `compile_state_subaction` helper that walks the tagged children to a
   `Structured` body (SMC-6/7). The transition EFFECT was a tree-sitter grammar
   gap — `effect_action` didn't accept the canonical `do action { … }` form
   (xtext TransitionUsage 1849-1857), so the effect parsed as detached sibling
   nodes and never reached the IR; fixed by adding the `do action {…}` arm to
   `effect_action` (+ regen). A third bug surfaced via SMC-4's multi-statement
   body: `compile_action_from_children` built assignments in `HashSet` order
   (`owner_to_children` is unordered) — fixed by sorting children by source span.
   SMC-4/6/7 → CONFORMS; the full firing sequence exit→effect→entry now executes
   in order. SysML §7.18.1/§7.18.3.
3. **GAP-ACT-COMPILE (actions)** — `compile_action` drops succession guards and
   assignment RHS (placeholder `LiteralInt(0)`); decision routing from `.sysml`
   source can't work. (Control-node *runtime* semantics CONFORM at IR level.)
4. **GAP-PHYS (quantities)** — **Q5 ✅ CLOSED `2026-06-21`:** the spec function
   `Interpolate` now returns null out of bounds (SampledFunctions.sysml:80-84 /
   §9.4.3.2.2); the internal `interpolateLinear` ODE helper still clamps for
   integration edge-continuity (a flagged tool divergence — note
   `interpolateLinear : Interpolate`). **Q4 ✅ CLOSED `2026-06-21`:**
   `build_sampled_function_from_pairs` no longer re-sorts — it validates the given
   domain is strictly monotonic (increasing OR decreasing per §9.4.3.2.6),
   preserves the order, and rejects non-monotonic/duplicate domains;
   `interpolate_linear_impl` is now direction-aware (ascending path byte-identical;
   descending mirrored). Baseline byte-identical (ODE lookups are ascending).
   (Good news: dimensional consistency IS enforced.)
6. **GAP-VER-ANALYSIS (verification/analysis cases)** — `VerificationRunner::verify`
   honest-Inconclusive CONFORMS. **no-criteria vacuous-Pass ✅ CLOSED `2026-06-21`:**
   a case with no modeled requirements, or a requirement with no constraints and
   no subrequirements, now yields Inconclusive (a determination cannot be made),
   not a vacuous Pass — SysML §8.4.20.1 (criteria must be modeled explicitly) /
   §7.24.1 (Inconclusive = determination could not be made). **analysis-case
   `objective→result` binding ✅ CLOSED `2026-06-22` (FORM-B Tier-1):** an analysis
   case's objective subject is bound to its RESULT (§7.23.2 / Cases.sysml:46, not
   overridden by AnalysisCase, vs verification's case-subject binding); the literal
   result is bound to the objective's verified-requirement subject via the shared
   `discover_objective_requirements` path (pluggable `ObjectiveSubjectSource`), and
   `AnalysisCaseIR::verify_objective` produces the verdict through the ONE engine
   (VerificationRunner). FORM-B Tier-2 (static expression/calc) and Tier-3
   (SOLVER-EXECUTED result via the analysis case's OWN `execute`/`run_and_verify`,
   `@ToolExecution`-selected bisection/ODE) also CLOSED `2026-06-22` — covering
   §7.23.1 (a/b/c) result sources. Still-unbuilt: the STATE-MACHINE-sim-coupled path
   (demote `verify_with_simulation`, complete the STUB `IECCompliance` corpus —
   director-gated) + requirement-invocation `PassIf(req(args))` (no fixture without
   the sim); a separate Inconclusive→Fail flatten in
   `evaluate_constraints_with_context` (LSP path).
5. **GAP-STRUCT (well-formedness)** — a validator exists (~86 S-rules) but the
   spec's "must-specialize-library-base" obligations are UNOWNED (elaboration
   silently auto-adds the base type rather than validating it). **S068/S069
   (at-most-one entry/do/exit subaction) ✅ CLOSED `2026-06-21`:** the check
   counted `StateSubactionMembership` (never minted); now counts the parser's
   `ActionUsage` + `stateSubactionKind` shape, so >1 of a kind raises S068/S069.
   **S064/S065 (at-most-one objective) ✅ CLOSED `2026-06-21`:** the rule is
   registered for `CaseDefinition`/`CaseUsage`, but a `use case def` is a
   `UseCaseDefinition` *subtype* and the generated dispatcher routed by exact
   kind. Fixed by making the dispatcher HIERARCHY-AWARE (steward-ruled): each
   element type's match arm now also emits the rules registered on its transitive
   supertypes (`semantic_validation_generator.rs`, deduped by check; `build.rs`
   passes the TypeHierarchy). So a rule on `CaseDefinition` fires on every case
   subtype. Full-corpus blast-radius audit clean (check guards hold → no spurious
   diagnostics); guarded by codegen test `subtype_arm_inherits_supertype_rules`.
   The **3 base-specialization obligations** (calc-def→Calculation §8.3.19.2,
   constraint-def→ConstraintCheck §7.20, verification-case-def→VerificationCase
   §8.3.24) are **✅ RECLASSIFIED SATISFIED-BY-CONSTRUCTION `2026-06-21`** (steward
   ruling): SysML derives these via implicit generalization, which the engine
   faithfully ports (`elaborate::implicit_generalization`), so the base is present
   by construction — there is no validator obligation. Gated by
   `implicit_generalization.rs`; the mis-framed no-library gates were removed.
   **0 UNIMPLEMENTED remain** in `structural_spec_conformance.rs` — the structural
   well-formedness backlog is fully closed (subject-first-param CLOSED via
   S141-S144 and port-nested-referential CLOSED via S145/S146, both `2026-06-21`).

**Cross-cutting:** the structural validator is real but partial — recommend a
focused fix-wave for the unowned base-specialization checks + the two latent
rules, separate from this runtime-semantic sweep.

## Change ledger (append-only)

A running, append-only record of each increment of the sweep — what shipped,
which gaps it opened, and the next gate. Newest last. (Sibling to, not a
*incidental architectural debt*; this ledger tracks *this sweep's progress*.
Correctness divergences found here live as ranked gaps in the per-area matrix,
not in the debt ledger.)

| # | Date | Commit | Increment | Gaps opened | Next gate |
|---|------|--------|-----------|-------------|-----------|
| 1 | 2026-06-21 | `656c4de0` | **D1** — baked spec-doc-first source precedence into root `CLAUDE.md` + `sysml-research-agent.md`. | — | — |
| 2 | 2026-06-21 | `1af741ed` | **D2 pilot** — constraints & expressions: tracker dir (this README + `constraints-expressions.md`, 11 obligations, spec-HTML-verified) + gate `tests/constraint_spec_conformance.rs` (8 gated, all CONFORMS, 9 tests green). Behavioral 6/8, all-obl 6/11. | **GAP-1** (DIVERGES) `assert not constraint` negation not applied at eval layer; **GAP-2** (DEFERRED) asserted-false→Fail verdict mapping in in-flight check layer. | **Director review of obligation-list + matrix format before fan-out.** |
| 3 | 2026-06-21 | _(this commit)_ | **Fan-out: requirements** — `requirements.md` (15 obligations) + gate `tests/requirement_spec_conformance.rs` (5 CONFORMS, 6 tests green). Satisfaction logic (implication, vacuous-truth, negation) all conform. | None new at logic layer. Subject-binding rows (×5) DEFERRED to in-flight build; `subrequirement-is-required-constraint` DOCUMENTED (needs probe). | Probe subrequirement aggregation before gating it. |
| 4 | 2026-06-21 | _(this commit)_ | **Fan-out: calculations** — `calculations.md` (13 obligations) + gate `tests/calculation_spec_conformance.rs` (4 CONFORMS, 5 tests green). All runtime-eval obligations conform. | No eval gap. ~6 STRUCTURAL well-formedness obligations are UNOWNED (no gate anywhere). | Director: decide whether structural/validation well-formedness gets its own sweep. |
| 5 | 2026-06-21 | _(this commit)_ | **Fan-out catalogues: state-machines + actions** — `state-machines.md` (15 obl), `actions.md` (15 obl). Trigger/guard (SM) and send/accept (actions) gated via RSC-0.2/0.3; entry/do/exit, transition firing-order, control-nodes, succession are UNGATED. | **GAP-SM-1..3** (firing-order, entry/do/exit, run-to-completion ungated); **GAP-ACT-1..3** (control-nodes, succession, assignment ungated). Verdicts need a probe pass. | Author SM + action gates (coordinate with active runtime work); probe for verdicts. |
| 6 | 2026-06-21 | _(this commit)_ | **Fan-out: flows/ports + verification (obligations)** — `flows-ports.md` (15 obl, 90% gated via RSC-0.1/0.3), `verification-analysis-cases.md` (13 obl, fixtures deferred to in-flight build). | **GAP-FLOW-1** succession-flow ordering ungated. Verification: 0 gated by charter (deferred). | After verification build lands, author `verification_spec_conformance.rs`. |
| 7 | 2026-06-21 | _(prior commit)_ | **Fan-out COMPLETE: occurrences/clocks + ODE/physics** — `occurrences-clocks.md` (14 obl; clock core gated via RSC-0.2), `ode-physics.md` (~14 obl). | **GAP-OCC-1..2** (DurationOf, time-ordering, localClock default ungated); **GAP-PHYS-1** (dimensional consistency ungated). KEY FINDING: ODE numerical solving is SPEC-SILENT/tool-territory. | Director: (1) schedule SM/action/occ gate passes coordinated with active runtime work; (2) decide on a separate STRUCTURAL/validation conformance sweep. |
| 8 | 2026-06-21 | `92441ad3` | **GATING PASS (parallel): 6 suites, 53 tests, all green** — SM(10), actions(7), assert-constraint/GAP-1(8), occurrences(7), quantities(11), verification-verdict(10). Tests assert ACTUAL behavior; DIVERGES rows pin gaps. | **GAP-1 CONFIRMED** (4 DIVERGES); **GAP-SM-EXEC** found — exit/do/effect bodies dropped (3 DIVERGES); **GAP-ACT-COMPILE** documented; **GAP-PHYS** — Interpolate clamps not null + SampledFunction re-sorts decreasing domain (2 DIVERGES). dim-consistency ENFORCED (good). | Schedule fix-waves for GAP-1 + GAP-SM-EXEC (coordinate w/ active runtime work). |
| 9 | 2026-06-21 | `9b2d684d` | **Structural well-formedness: 13 tests** (5 CONFORMS, 7 UNIMPLEMENTED). Probes `validate_semantic` + `validate_structure` (~86 S-rules). | **GAP-STRUCT** — "must-specialize-library-base" checks UNOWNED (elaboration silently auto-adds base type); S068/S069 + S064/S065 are latent dead code (parser-shape / dispatcher-subtype mismatch). | Director: fix-wave for unowned base-specialization checks + 2 latent rules. |
| 10 | 2026-06-21 | `49e32784` | **Verification CASE pipeline (now settled): 7 tests** complementing teammate's `cases_pipeline.rs`. 3 CONFORMS, 3 DIVERGES, 1 UNIMPL. | **GAP-VER-ANALYSIS** — analysis-case objective→result binding absent (no verdict from analysis case); verdict-criteria default = vacuous Pass. Honest-Inconclusive at case level CONFIRMED-CONFORMS. Still unbuilt: Inc2b sim-coupling; Inconclusive→Fail flatten in evaluate_constraints_with_context (LSP path). | Director: schedule analysis-case binding + Inc2b + Inconclusive-flatten fix-waves. |
| 11 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-1 CLOSED** — added `ConstraintIR.is_negated` (`lib.rs`), set from `AssertConstraintUsage.isNegated` at both extraction paths (`extract_from_element` + `TypedConstraint::from_element`), inverted the decided verdict at the `evaluate_expr` chokepoint (eval layer + per-instance check both corrected). 4 ex-DIVERGES → CONFORMS; `gap_repros::gap1_*` flipped to a regression guard; runtime `e2e_negated_assert_constraint` updated to spec-correct expectation. Baseline-neutral (no corpus fixture uses `assert not`). | None. DIVERGES 11→7; backlog 21→17. | Next fix-wave: **GAP-SM-EXEC** (exit/do/effect bodies dropped at SM compile). |
| 13 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-PHYS Q5 CLOSED.** `Interpolate` (stdlib.rs) now returns null out of bounds (direction-agnostic min/max bounds check) per SampledFunctions.sysml:80-84 / §9.4.3.2.2; `interpolateLinear` keeps clamping for ODE edge-continuity (flagged divergence: `interpolateLinear : Interpolate`). q5 gate DIVERGES→CONFORMS; 2 runtime stdlib unit tests repointed to interpolateLinear + a new Interpolate-null unit test. Runtime lib 1193 green; `Interpolate` not used in the ODE/corpus path → baseline-safe by inspection. | None. DIVERGES 5→4; backlog 15→14. | Q4 (decreasing domain) deferred — needs ODE baseline un-drifted (see GAP-SM-EFFECT). |
| 12 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-SM-EXEC exit/do CLOSED (effect re-scoped).** `compile_state` now routes entry/do/exit through one `compile_state_subaction` helper that walks the tagged exit/do children to a `Structured` body (was: only entry had the children-fallback; exit/do degraded to `Simple("")`). SMC-6/SMC-7 → CONFORMS; `gap_repros` updated to regression guards; SM matrix counts re-pinned (CONFORMS 8 / DIVERGES 1). Runtime statemachine 128 + SM gate 9 green. Baseline-neutral. | **GAP-SM-EFFECT (grammar)** discovered: inline transition `do action {…}` effect is not attached to the transition by the tree-sitter grammar (detached sibling nodes in any position) → effect body never reaches the IR. SMC-4 stays DIVERGES. DIVERGES 7→5; backlog 17→15. | **Director decision:** GAP-SM-EFFECT needs grammar work + regen (~57 min, batched per CLAUDE.md) — out of the runtime fix-loop's mechanical scope. Next mechanical fix-wave: **GAP-ACT-COMPILE** or **GAP-PHYS**. |
| 14 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-SM-EFFECT CLOSED + grammar-drift reconciled (director-authorized).** Grammar: `effect_action` accepts `do action {…}` (regen, no new conflict). Runtime: `compile_action_from_children` sorts statement children by source span (root cause: `owner_to_children` is a `HashSet` → unordered). SMC-4 → CONFORMS (full exit→effect→entry firing). **Disentangle (director-requested):** regen WITHOUT my edit reproduced the same 10 corpus/conformance failures → PROVEN pre-existing drift from `13b6886f` (quantity `unit`) + `0d0850bf` (accept-action), independent of GAP-SM-EFFECT; all 10 are improvements (7 pilot `ts_only_paths` ↓, CoreODE spurious-import removed, `unit` is a TS-internal encoding). Reconciled: `unit` + `value`@SubjectMembership grounded in spec_property_conformance allowlists; 7 pilot `fixture_baseline` counts re-baselined (+ block rationale); CoreODE cross_transport snapshot + 3 parity fixtures re-blessed. | None new. DIVERGES 4→3; backlog 14→13. **Pre-existing (NOT fixed):** `no_legacy_string_readers` red from `39b5e889` (cases/mod.rs:2474 reads `unresolved_value`). | **GAP-STRUCT** (base-specialization = elaboration-vs-validate design call) + **GAP-VER-ANALYSIS** + smc10/11 remain — all need director steer. |
| 15 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-PHYS Q4 CLOSED.** `build_sampled_function_from_pairs` (stdlib.rs) no longer sorts — validates the given domain is strictly monotonic (increasing OR decreasing, §9.4.3.2.6), preserves order, rejects non-monotonic/duplicate; `interpolate_linear_impl` is direction-aware (ascending path byte-identical, descending mirrored). q4 gate DIVERGES→CONFORMS; 3 stdlib unit tests that pinned the old auto-sort updated to spec-correct (reject-unsorted / preserve-order / monotonic-message) + a new descending-interpolation test. Runtime lib 1194; baseline byte-identical (ODE lookups ascending). | None. DIVERGES 3→2; backlog 13→12. | GAP-STRUCT / GAP-VER-ANALYSIS / smc10/11 remain (design-gated). |
| 16 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-VER no-criteria vacuous-Pass CLOSED (director-authorized).** `VerificationRunner::verify` (cases/mod.rs): a case with NO modeled requirements → Inconclusive (was vacuous Pass); `check_requirement`: a requirement with NO constraints AND no subrequirements → Inconclusive. SysML §8.4.20.1 (criteria modeled explicitly) / §7.24.1 (Inconclusive = determination could not be made); consistent with the existing honest-Inconclusive design and distinct from unmet-assumption vacuous satisfaction. 2 verification_case gates DIVERGES→CONFORMS; runtime `cases_pipeline::verification_case_no_requirements_*` updated to assert Inconclusive. Runtime lib 1194 + cases_pipeline 9 + service contract_evaluate green; baseline verified. | None. **DIVERGES 2→0** (no behavioral divergences left); backlog 12→10. | Remaining: all UNIMPLEMENTED (GAP-STRUCT, smc10/11, analysis-case objective→result) — design-gated, need director steer. |
| 17 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-STRUCT S068/S069 CLOSED (S064/S065 stopped — codegen).** `at_most_one_state_subaction` (cardinality.rs) counted `StateSubactionMembership` (the spec wrapper our parser never mints); now counts the parser's `ActionUsage` + `stateSubactionKind` shape too, so >1 entry/do/exit on a state def/usage raises S068/S069. Hand-written check fix, no codegen. two_entry_subactions gate UNIMPLEMENTED→CONFORMS; sysml-core 556 green; baseline checked. **S064/S065 (at-most-one objective) STOPPED per director gate:** the fix needs `semantic_rules.toml`/dispatcher changes (route the rule to `UseCaseDefinition`/other CaseDefinition subtypes — the dispatcher keys on exact kind) — a codegen/design call, flagged not built. | None. UNIMPLEMENTED 13→12; backlog 10→9. | Remaining UNIMPLEMENTED: S064/S065 (codegen), 3 base-specialization + subject-first-param + port-nested-referential (GAP-STRUCT), smc10/11, analysis-case objective→result — all need director steer. |
| 18 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-STRUCT S064/S065 CLOSED (codegen — steward-ruled, director-authorized "clean path / consult steward").** Made the generated semantic-validation dispatcher HIERARCHY-AWARE: each element type's match arm now also emits the rules registered on its transitive supertypes (`semantic_validation_generator.rs::generate_entry_point`, deduped by (category,check); `sysml-core/build.rs` passes the codegen TypeHierarchy). Root cause: a subtype with its own explicit arm (UseCaseDefinition has S139) never reached the `_ =>` subtype catch-all, so a CaseDefinition rule never fired on it. Now at-most-one-objective fires on all case subtypes. two_objectives_on_use_case_def gate UNIMPLEMENTED→CONFORMS; codegen regression test `subtype_arm_inherits_supertype_rules` added. **Full-corpus blast-radius audit CLEAN** (0 new diagnostics — every supertype-registered rule now reaches subtypes, but the check fns self-guard so none misfire); sysml-core 556 + full spec-tests green. (6 codegen integration tests fail on a pre-existing spec-file-path issue, unrelated.) | None. UNIMPLEMENTED 12→11; backlog 9→8. | Remaining UNIMPLEMENTED: 3 base-specialization (Decision 2 — reclassify satisfied-by-construction), subject-first-param + port-nested-referential (validator rules), smc10/11 + analysis-case (IR surfaces). |
| 19 | 2026-06-21 | _(reclassify)_ | **RECLASSIFY: 3 base-specialization obligations → SATISFIED-BY-CONSTRUCTION (steward-ruled, no production code).** calc-def→Calculation (§8.3.19.2), constraint-def→ConstraintCheck (§7.20), verification-case-def→VerificationCase (§8.3.24) are NOT validator obligations: SysML derives the library-base specialization via implicit generalization, which `elaborate::implicit_generalization` faithfully ports (mapping table :102/110/142), so the base is present by construction after elaboration-with-library — nothing to flag. The 3 mis-framed no-library `#[ignore]`d gates (which expected a diagnostic the no-library `check()` path can never observe — `resolve_base` needs the library) were REMOVED from structural_spec_conformance.rs; the minting mechanism is gated by `implicit_generalization.rs` unit tests. Matrix guard updated (unimpl 5→2). Area trackers reclassified. | None. UNIMPLEMENTED 11→8; backlog 8→5 (3 now satisfied-by-construction). | Remaining UNIMPLEMENTED: subject-first-param + port-nested-referential (validator rules), smc10/11 + analysis-case (IR surfaces). |
| 24 | 2026-06-22 | _(fix-wave)_ | **FIX-WAVE: SMC-11 stateSequencing-count CLOSED (elaboration — director-approved, was SPEC-SILENT in row 23).** New `elaborate::derive_state_sequencing` (sysml-core elaborate/state_machines.rs) materialises the N-1 implicit `Succession` relationships (tagged `stateSequencing=true`) between a non-parallel state's exclusive `StateUsage` substates in declaration order, on the `ModelGraph` — the spec's library-derived `stateSequencing` succession feature (States.sysml:71-77), NOT an execution-IR field (CLAUDE #4/#6). Parallel states excluded; idempotent. Diagram General view skips tagged successions (implicit ordering ≠ user edge). New `ElaborationReport.state_sequencing_created`. Gate `smc11_state_sequencing_count_invariant` rewritten to elaborate + assert `size(stateSequencing)==size(exclusiveStates)-1` on the graph; un-ignored → CONFORMS. SM matrix guard 10→11 CONFORMS / 1→0 UNIMPL. 4 new elaboration unit tests (N-1 / parallel-excluded / single / idempotent). **Blast-radius:** runtime SM 128 (compiler reads `Transition` not `Succession`), elaborate 89, reparse-identity + elaborate-equivalence + property-conformance + diagram all green; service_command_baseline byte-identical (see below). **FOLLOW-ON:** the library's own `assert constraint` self-evaluating needs the constraint evaluator to resolve inherited `exclusiveStates`/`stateSequencing` to counts (evaluator still honestly reports `Error: undefined exclusiveStates`). | None. UNIMPLEMENTED 5→4; backlog: SMC-11 closed, only director-gated analysis-case remains. | Remaining: analysis-case objective→result (FORM-B Tier-3, design plan in progress; build is director/Inc2b-gated). |
| 23 | 2026-06-22 | _(triage)_ | **TRIAGE: the final 2 backlog gaps dispositioned NO-BUILD (core-steward) — buildable backlog now 0.** (1) **SMC-11 stateSequencing-count** reclassified **SPEC-SILENT (design-undecided)**: `stateSequencing` is a library-derived succession on the abstract `StateDefinition` (`States.sysml:71-77`); its honest home is ELABORATION (generate N-1 `succession` relationships between `exclusiveStates` on the ModelGraph), NOT the execution `StateMachineIR` — adding an IR field would duplicate graph structure + put abstract-syntax semantics in an execution IR (CLAUDE #4/#6). Current `Error: undefined exclusiveStates` from the constraint evaluator is CORRECT (not a false green); whether to elaborate the successions is undecided. Gate reframed to the elaboration layer, no IR surface, no re-bless. (2) **analysis-case objective→result (§7.23.2)** **DEFERRED-TO-DIRECTOR**: it is the base-`Case` `subject subj default Case::result` binding (`Cases.sysml:46`) — the same result-binding machinery as the director-gated **Inc2b**, NOT a standalone analysis feature; building standalone would duplicate the result-binding path. Gate reframed to defer; documented as CONFORMS-REQUIRED blocked on Inc2b. No production code, no baseline change. | None. | **Buildable backlog = 0.** Open (non-buildable): SMC-11 elaboration (SPEC-SILENT, needs design decision) + analysis-case result-binding (Inc2b, director-gated). |
| 22 | 2026-06-22 | _(fix-wave)_ | **FIX-WAVE: SMC-10 incomingTransitionTrigger CLOSED (steward-ruled IR surface).** New field `SubsystemState.incoming_transition_trigger: Option<String>` (lib.rs) mirrors `StatePerformances.kerml:48` `incomingTransitionTrigger : MessageTransfer [0..1]`. The SM runner records the triggering event at the firing site when a *message* trigger (`TriggerKind::Event`/`PortMessage`, or legacy non-completion/non-guard-only event transition) fires — NOT for time/`when`/completion/guard-only (those aren't MessageTransfers). Threaded `StepResult.incoming_trigger` → `TickOutput.incoming_trigger` → `SubsystemState` at both orchestrator insertion sites; forwarded through the hybrid executor and the service snapshot→StepResult conversion. Gate un-ignored, assertion rewritten to read the field (`ss.incoming_transition_trigger == Some("go")`); SM matrix guard 9→10 CONFORMS / 2→1 UNIMPL. **SPEC-SILENT** (documented on the field): records the event *name*, full MessageTransfer identity deferred. Additive `Option` field with `skip_serializing_if=is_none` → snapshots churn ONLY on a genuinely-triggered entry; baseline reviewed individually. runtime lib 1194; SM gate 11 (1 ignored = SMC-11). (Pre-existing unrelated red: `no_legacy_string_readers` cases/mod.rs:2503 `unresolved_value`, from 39b5e889 — NOT mine.) | None. UNIMPLEMENTED 6→5; backlog 3→2. | Remaining UNIMPLEMENTED: SMC-11 stateSequencing-count + analysis-case objective→result (IR surfaces). |
| 21 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-STRUCT port-nested-referential CLOSED (S145/S146 — steward-ruled).** New shared primitive `semantic_checks::composite::is_effectively_composite` (occurrence-default per SysML §8.9.2 "composition only applies to occurrences": explicit isComposite wins, else `ref`→referential, else composite iff `is_subtype_of(OccurrenceUsage)` — Attribute/Reference usages fall through to referential for free). New `semantic_checks::ports`: S145 `port_usage_nested_usages_referential` (PortUsage) + S146 `port_definition_owned_usages_referential` (PortDefinition) flag a composite non-port nested/owned usage (OCL `nestedUsage->reject(PortUsage)->forAll(not isComposite)`). New `category="port"` → ports module in codegen. Un-ignored the gate + added a PortDefinition gate + a referential negative-twin (12 structural gates, 0 ignored). **Blast-radius: 0 fires** — corpus+library scan found ZERO composite (or `ref`-prefixed) occurrence usages nested in ports; service_command_baseline byte-identical. sysml-core 570 (10 new composite/ports unit tests). **Latent parser gap documented (not blocking, no corpus instances):** `ref part X` in a port mis-lowers to a prop-less PartUsage + stray ReferenceUsage (drops the `ref` marker) → a `ref` occurrence usage in a port would false-positive; fix belongs in the parser, not the validator. | None. UNIMPLEMENTED 7→6; backlog 4→3. | Remaining UNIMPLEMENTED: smc10/11 + analysis-case objective→result (IR surfaces — design-gated). |
| 20 | 2026-06-21 | _(fix-wave)_ | **FIX-WAVE: GAP-STRUCT subject-first-param CLOSED (S141-S144 — steward-ruled Option A).** New validator `subject_is_first_parameter` (semantic_checks/requirements.rs) + 4 registry rules (S141 RequirementDefinition, S142 RequirementUsage, S143 CaseDefinition, S144 CaseUsage, `category="requirement"`). Faithful to §8.3.21 OCL `input->notEmpty() and input->first() = subjectParameter`: among the owned input features (subject + any KerML `/input` = direction `in`/`inout` feature, ordered by source span) the subject must be first; a directed input declared before the subject flags. **Verify-before-build caught a mis-framed fixture** (same failure mode as ledger 19): the gate fixture `attribute earlier; subject s;` used a *directionless* attribute → not an `input` → well-formed, NOT a violation; steward authorized correcting it to `in earlier; subject s;` (a real directed-param-before-subject violation). Hierarchy-aware dispatch (ledger 18) propagates S143/S144 onto case subtypes (use/verification/analysis). **Blast-radius: 0 real fires** — library scan = 0; the lone corpus "hit" (EveOnline) was a `doc /* in Low Sec */` comment false-positive (subject is first). sysml-core 8 requirements tests + structural gate 9 + cross-transport parity green; service_command_baseline byte-identical. | None. UNIMPLEMENTED 8→7; backlog 5→4. | Remaining UNIMPLEMENTED: port-nested-referential (validator rule), smc10/11 + analysis-case (IR surfaces). |
