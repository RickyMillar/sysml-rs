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

Obligations are derived in this source-precedence order:

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
