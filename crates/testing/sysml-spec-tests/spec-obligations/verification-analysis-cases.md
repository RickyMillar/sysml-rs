# Obligation matrix — Verification & analysis cases (OBLIGATIONS ONLY)

**Area:** verification-case verdicts + analysis-case results.
**Gate:** **NONE authored by this sweep — fixtures are DEFERRED to the in-flight
verification-case build (Inc1/Inc2).** This matrix lists obligations and maps
them to that build so the two efforts stay aligned and don't collide. **Status:**
fan-out area, obligations-only (per the sweep charter).

Spec sources: SysML §7.24 *Verification cases*, §7.23 *Analysis cases*, §8.3.22-24
(`SysML-spec-r2025-04_REF.html`); `sysml.library/Systems Library/VerificationCases.sysml`,
`Cases.sysml`, `AnalysisCases.sysml`. Verified `2026-06-21`.

> **Test results (2026-06-21):** the STABLE verdict semantics are now GATED by
> `tests/verification_verdict_spec_conformance.rs` (10 tests, all CONFORMS):
> `VerdictKind` enum + Display (`pass`/`fail`/`inconclusive`/`error`) + worst-wins
> `aggregate` (Error>Fail>Inconclusive>Pass). The case-evaluation / subject-binding
> / simulation pipeline remains DEFERRED to the in-flight build (not tested here to
> avoid churn on `cases/mod.rs`).

> **Case-pipeline results (2026-06-21):** the build has SETTLED (Inc1/Inc2
> landed); the teammate's `crates/lang/sysml-runtime/tests/cases_pipeline.rs`
> already covers subject binding / feature-ref RHS / objective discovery.
> Complementary gates added in `tests/verification_case_spec_conformance.rs`
> (7 tests; 7 CONFORMS, 0 DIVERGES, 0 UNIMPLEMENTED — updated `2026-06-22`):
> - **CONFORMS** — `VerificationRunner::verify` returns honest **Inconclusive**
>   (not Fail) for an unbound feature, and aggregates correctly. The runner does
>   NOT flatten Inconclusive.
> - **CONFORMS (was DIVERGES — CLOSED `2026-06-21`)** — a case with no modeled
>   requirements, or a requirement with no constraints and no subrequirements,
>   now returns **Inconclusive**, not a vacuous Pass. §8.4.20.1 (criteria must be
>   modeled explicitly) + §7.24.1 (Inconclusive = a determination could not be
>   made) ⇒ no criteria ⇒ no basis for a verdict ⇒ Inconclusive. This is the
>   principled spec-derived verdict, NOT an invented default.
> - **CONFORMS (GAP-VER-ANALYSIS — CLOSED `2026-06-22`, FORM-B Tier-1)** —
>   analysis-case `objective→result` binding (§7.23.2). `compile_analysis_case`
>   discovers the objective's verified requirement(s) via the SAME
>   `discover_objective_requirements` path verification cases use (pluggable
>   `ObjectiveSubjectSource`: analysis binds the RESULT, verification binds the case
>   subject — Cases.sysml:46 vs VerificationCases.sysml:25), binds the result literal
>   to the objective subject, and `AnalysisCaseIR::verify_objective` produces the
>   verdict through the ONE engine (`VerificationRunner`). Gated by
>   `analysis_case_objective_is_bound_to_result` here + verdict-flip/negative-twins in
>   `cases_pipeline.rs::analysis_objective_result_binding_drives_verdict`.
>
> Remaining pipeline implementation gaps: (1) STATE-MACHINE-sim-coupled objective
> results — demote `verify_with_simulation` (sm-keyed) to feed a case objective +
> complete the STUB corpus `IECCompliance` (never computes `r_ratio_at_test`)
> [director-gated; T1 literal + T2 static-expression + T3 solver-executed via the
> analysis case's OWN `execute` are DONE `2026-06-22`]. **NB the sim-coupled verdict
> needs NET-NEW runtime plumbing: `run_and_verify`/`solver.solve` runs a bare ODE, NOT
> a coupled state machine, and `cases/mod.rs` never touches the orchestrator — so a
> hybrid analysis case would need a measurement primitive + an orchestrator-sourced
> executed result, which is bigger than a binding change;** (2) requirement invocation
> `PassIf(req(args))` (T3b, design plan §5) — no buildable fixture without the
> sm-sim coupling; (3) Inconclusive-flattening in `evaluate_constraints_with_context`
> (evaluation.rs:1353) affecting LSP constraint monitoring [inferred].

> Cross-reference: project memory `project_eval_conformance_holistic_sweep` and
> the constraint-eval conformance entry track the active build. The obligations
> below are the spec contract that build must satisfy; this sweep will gate them
> **after** the build lands (to avoid touching `cases/mod.rs`, `verify_with_simulation`,
> and `cases_pipeline.rs`, which are off-limits during active work).

## Obligation table

| ID | Obligation | Citation (tier) | Build status |
|----|-----------|-----------------|--------------|
| `verdict-kind-enumeration` | `VerdictKind` has exactly `pass`/`fail`/`inconclusive`/`error`. | `VerificationCases.sysml:58-68` (LIBRARY) | **IMPLEMENTED** — `cases::VerdictKind` matches; gate after build. |
| `verdict-semantics` | Pass=subject determined to satisfy; Fail=determined not to; Inconclusive=determination could not be made; Error=error during verification. | §7.24.1 *"Inconclusive indicates that a determination could not be made…"* (GOSPEL) | **IMPLEMENTED (honest-Inconclusive)** — our Inconclusive matches the spec meaning (per memory). Gate after build. |
| `verification-case-result-is-verdict` | A verification case's result is a `VerdictKind`. | `VerificationCases.sysml:22` `return verdict : VerdictKind :>> result` (LIBRARY) | **IMPLEMENTED** — gate after build. |
| `verification-subject-bound-to-objective-subject` | The verified requirement's subject is bound to the objective's subject, itself bound to the case subject. | §7.24.2 *"its subject is bound by default to the subject of the objective…"*; `VerificationCases.sysml:25` (GOSPEL+LIBRARY) | **IN-FLIGHT (Inc1c subject binding)** — the active build's core. Gate after it lands. |
| `verified-requirements-derivation` | `verifiedRequirements` = the verifiedRequirements of the objective's RequirementVerificationMemberships. | §8.3.24.3 `deriveVerificationCaseDefinitionVerifiedRequirement` (GOSPEL) | **IN-FLIGHT (Inc1a/1b objective discovery)** — gate after build. |
| `requirement-verification-membership-in-objective` | A `verify requirement` must be owned by a RequirementUsage under an ObjectiveMembership. | §8.3.24.2 `validateRequirementVerificationMembershipOwningType` (GOSPEL, STRUCTURAL) | STRUCTURAL — validation sweep. |
| `requirement-verification-is-subrequirement` | A `verify requirement R` is automatically a required constraint of its objective. | §7.24.2; `VerificationCases.sysml:27` (GOSPEL+LIBRARY) | **IN-FLIGHT** — objective discovery. |
| `verdict-criteria-modeled-explicitly` | The pass/fail criteria must be modeled explicitly in the case body; no implicit derivation. | §8.4.20.1 *"the criteria for passing must be modeled explicitly."* + §7.24.1 *"Inconclusive … a determination could not be made"* (GOSPEL) | **CONFORMS (CLOSED `2026-06-21`)** — with no criteria modeled, a determination cannot be made, so `verify` returns Inconclusive (not a vacuous Pass). This is spec-DERIVED (§7.24.1), not an invented default. Gated by `case_with_no_criteria_*` + `requirement_with_empty_constraint_list_*`. |
| `analysis-case-objective-bound-to-result` | An analysis case's objective subject is bound to the analysis **result** (not the case subject). | §7.23.2 + `Cases.sysml:40-47` `objective obj : RequirementCheck { subject subj default Case::result }` (GOSPEL) | **CONFORMS (FORM-B Tier-1, CLOSED `2026-06-22`)** — `compile_analysis_case` discovers the objective's verified requirement via the shared `discover_objective_requirements` path with a pluggable `ObjectiveSubjectSource` (analysis = result per Cases.sysml:46, not overridden by AnalysisCase; verification = case subject per VerificationCases.sysml:25 — ONE pipeline, one verdict source). The result binds the objective subject; the verdict routes through the ONE engine (VerificationRunner, B4) — no duplicate result-binding path. Value-less result → honest Inconclusive. **CONFORMS for literal + static-expression + SOLVER-EXECUTED result sources (§7.23.1 a/b/c):** Tier-1 literal + Tier-2 static expression/calc (`verify_objective`, compile-time) + **Tier-3 executed (`run_and_verify`, CLOSED `2026-06-22`):** runs the analysis case's OWN solver/ODE via `AnalysisCaseIR::execute` (selected by `@ToolExecution`, e.g. `builtin:bisection`/`builtin:ode-rk45` — NOT the state-machine `verify_with_simulation` command), seeds a context from the solver outputs, resolves the executed result, binds it to the objective subject (executed value SHADOWS the compile-time static binding), and verdicts via VerificationRunner. Non-convergence/solver-error → honest Inconclusive. Gated by `analysis_case_objective_is_bound_to_result` + `cases_pipeline.rs::{analysis_objective_result_binding_drives_verdict, analysis_objective_expression_result_drives_verdict, analysis_objective_executed_result_drives_verdict}` (the last proves Tier-3 does what the static tiers cannot: same case is Inconclusive under `verify_objective` but decisive under `run_and_verify`). **DEFERRED-TO-DIRECTOR (the STATE-MACHINE-sim-coupled path only):** demoting `verify_with_simulation` (sysml-service/lib.rs:6681, keyed by `sm_name` + flat graph-wide constraints + `overall_satisfied:bool` = a SECOND verdict path, B4) to feed a case objective = a selector + return-contract change; AND the corpus `IECCompliance.sysml::TypeBComplianceSuite` is a STUB (never computes `r_ratio_at_test`; `H_peak` undefined) → completing it = authoring physics (§6.4 corpus-completion = separate director call), not conformance plumbing. Per design plan §5. |
| `case-has-subject-and-objective` | A case has ≤1 subject (first input) and ≤1 objective. | §8.3.22.2 `validateCaseDefinition*` (GOSPEL, STRUCTURAL) | **CONFORMS** — ≤1 subject (S062/S063), ≤1 objective (S064/S065), and subject-is-first-input (S143/S144, `subject_is_first_parameter`, `2026-06-21`; parallel to the requirement-subject rule, §8.3.21 OCL applied to the same model shape). Objective cardinality gated by `structural_spec_conformance::two_objectives_on_use_case_def_is_flagged`. |
| `verification-case-specializes-library-base` | VerificationCaseDefinition specializes `VerificationCases::VerificationCase`. | §8.3.24.3/4 (GOSPEL, STRUCTURAL) | **SATISFIED-BY-CONSTRUCTION (`2026-06-21`)** — not a validator obligation; `elaborate::implicit_generalization` derives the base specialization (mapping :142), present by construction after elaboration-with-library. Gated in sysml-core. (Steward-ruled; mis-framed no-library structural gate removed.) |
| `analysis-case-specializes-library-base` | AnalysisCaseDefinition specializes `AnalysisCases::AnalysisCase`. | §8.3.23.2/3 (GOSPEL, STRUCTURAL) | STRUCTURAL — validation sweep. |
| `passif-boolean-to-verdict` | `PassIf(isPassing)` returns `pass` if true else `fail`. | `VerificationCases.sysml:70-79` (LIBRARY) | **DOCUMENTED** — utility; gate after build. |

## SPEC-SILENT flags (align with the in-flight build's findings)

- **SS-A verdict computation** — no default algorithm when the verdict expression
  is absent; tool-defined.
- **SS-B sim-derived value binding** — the `:>> attr = analysisUsage.result`
  feature-reference-RHS binding is the spec mechanism, but the protocol for
  resolving a RHS that depends on an external simulation is spec-silent. This is
  exactly the Inc2 layer-3 bridge the active build is designing — keep the
  obligation here, let the build own the resolution.

## Coverage

- **Gated by this sweep: 0** (deferred by charter).
- Behavioral obligations awaiting the in-flight build: 6 (verdict ×3, subject
  binding, verified-requirements derivation, requirement-verification-subrequirement).
- STRUCTURAL: 4. SPEC-SILENT: 2.
- **Action:** once the verification-case build lands, author
  `tests/verification_spec_conformance.rs` gating the 6 behavioral obligations
  (verdict semantics incl. honest-Inconclusive, subject binding flips verdict,
  objective-derived verified requirements).

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `verdict-semantics` | `$SYS` §7.24.1 | `grep -n -i "Inconclusive indicates that a determination could not be made" /tmp/sys.txt` |
| `verification-subject-bound-to-objective-subject` | `$SYS` §7.24.2 | `grep -n -i "bound by default to the subject of the objective" /tmp/sys.txt` |
| `analysis-case-objective-bound-to-result` | `$SYS` §7.23.2 | `grep -n -i "subject of the objective is always bound to the result" /tmp/sys.txt` |
| `verdict-criteria-modeled-explicitly` | `$SYS` §8.4.20.1 | `grep -n -i "criteria for passing must be modeled explicitly" /tmp/sys.txt` |
| VerdictKind / PassIf / result | `sysml.library/Systems Library/VerificationCases.sysml` | `grep -n "VerdictKind\|PassIf\|:>> result" "<file>"` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Section | Title | Status |
|---------|-------|--------|
| §7.22 / §7.22.1 | Cases Overview | Reviewed |
| §7.22.2 | Case Definitions and Usages | Reviewed |
| §7.23.1 | Analysis Cases Overview | Reviewed |
| §7.23.2 | Analysis Case Definitions and Usages | Reviewed |
| §7.23.3 | Trade-Off Analyses | Reviewed |
| §7.24.1 | Verification Cases Overview | Reviewed |
| §7.24.2 | Verification Case Definitions and Usages | Reviewed |
| §8.3.22.2 | CaseDefinition abstract syntax | Reviewed |
| §8.3.22.3 | CaseUsage abstract syntax | Reviewed |
| §8.3.22.4 | ObjectiveMembership abstract syntax | Reviewed |
| §8.3.23.2 | AnalysisCaseDefinition abstract syntax | Reviewed |
| §8.3.23.3 | AnalysisCaseUsage abstract syntax | Reviewed |
| §8.3.24.2 | RequirementVerificationMembership abstract syntax | Reviewed |
| §8.3.24.3 | VerificationCaseDefinition abstract syntax | Reviewed |
| §8.3.24.4 | VerificationCaseUsage abstract syntax | Reviewed |
| §8.4.18.1 | Case Definitions semantics | Reviewed |
| §8.4.18.2 | Case Usages semantics | Reviewed |
| §8.4.19.1 | Analysis Case Definitions semantics | Reviewed |
| §8.4.19.2 | Analysis Case Usages semantics | Reviewed |
| §8.4.20.1 | Verification Case Definitions semantics | Reviewed |
| §8.4.20.2 | Verification Case Usages semantics | Reviewed |
| `Cases.sysml` | Library: Case, cases, subj/obj/result/actors/subcases | Reviewed |
| `AnalysisCases.sysml` | Library: AnalysisCase, analysisCases, subAnalysisCases | Reviewed |
| `VerificationCases.sysml` | Library: VerificationCase, VerdictKind, PassIf, VerificationMethod, VerificationMethodKind | Reviewed |

### Clause-to-obligation cross-reference

| Spec unit | Obligation ID | Classification |
|-----------|--------------|----------------|
| §7.22.2 + §8.3.22.2/3 `validateCaseDefinition/UsageOnlyOneObjective` | `case-has-subject-and-objective` | STRUCTURAL — existing |
| §7.22.2 + §8.3.22.2/3 `validateCaseDefinition/UsageSubjectParameterPosition` | `case-has-subject-and-objective` | STRUCTURAL — existing |
| §7.22.2 + §8.3.22.4 `validateObjectiveMembershipOwningType` | `case-has-subject-and-objective` | STRUCTURAL — existing |
| §7.23.2 gospel: *"subject of the objective is always bound to the result"* | `analysis-case-objective-bound-to-result` | CONFORMS (FORM-B Tier-1, CLOSED `2026-06-22`) |
| §7.23.3 + §9.4.5 `TradeStudy` library | **NEW: `tradeoff-analysis-evaluation-function`** | OUT-OF-SCOPE — `TradeStudy` is a higher-level library pattern built on `AnalysisCase`; no separate runtime obligation beyond `analysis-case-objective-bound-to-result`. |
| §7.24.1 + §7.24.2 typical verification actions (collect/process/evaluate data) | **NEW: `verification-action-steps`** | OUT-OF-SCOPE — §7.24.1 states these are *"typical"* steps, not mandated structure; tool-defined. |
| §7.24.2 + §8.3.24.2 `validateRequirementVerificationMembershipKind` | `requirement-verification-membership-in-objective` | STRUCTURAL — existing |
| §7.24.2 + §8.3.24.2 `validateRequirementVerificationMembershipOwningType` | `requirement-verification-membership-in-objective` | STRUCTURAL — existing |
| §8.3.22.2 `deriveCaseDefinitionActorParameter` (ActorMembership) | **NEW: `case-actor-parameter`** | STRUCTURAL — not yet in table; validation sweep gap. |
| §8.3.23.2/3 `resultExpression` / `ResultExpressionMembership` | **NEW: `analysis-case-result-expression`** | CONFORMS (FORM-B Tier-2, CLOSED `2026-06-22`) — `analysis_result_value` evaluates a `return result = <expr>` STATICALLY over the case's input-attribute defaults (`analysis_input_context`, design plan §4.2) and binds the computed result to the objective subject; unbound input → Inconclusive (B1, no fabricated value). Gated by `cases_pipeline.rs::analysis_objective_expression_result_drives_verdict`. Runtime/solver/sim-supplied inputs = Tier-3a (Inc2b). |
| §8.3.24.3/4 `deriveVerificationCaseDefinition/UsageVerifiedRequirement` | `verified-requirements-derivation` | CAPTURED-AS-DEFERRED — existing |
| §8.4.18.1 `checkCaseDefinitionSpecialization` → `Cases::Case` | `case-has-subject-and-objective` (structural) + `verification-case-specializes-library-base` | STRUCTURAL — existing |
| §8.4.19.1 `checkAnalysisCaseDefinitionSpecialization` → `AnalysisCases::AnalysisCase` | `analysis-case-specializes-library-base` | STRUCTURAL — existing |
| §8.4.20.1 *"criteria for passing must be modeled explicitly"* | `verdict-criteria-modeled-explicitly` | SPEC-SILENT — existing |
| §8.4.20.1 + §8.4.20.2 result = `verdict : VerdictKind` | `verification-case-result-is-verdict` | CAPTURED-AS-DEFERRED — existing |
| `VerificationCases.sysml:22` `return verdict : VerdictKind :>> result` | `verification-case-result-is-verdict` | CAPTURED-AS-DEFERRED — existing |
| `VerificationCases.sysml:25` `subject subj = VerificationCase::subj` (obj→subj binding) | `verification-subject-bound-to-objective-subject` | CAPTURED-AS-DEFERRED — existing |
| `VerificationCases.sysml:27` `requirementVerifications :> subrequirements` | `requirement-verification-is-subrequirement` | CAPTURED-AS-DEFERRED — existing |
| `VerificationCases.sysml:58-68` VerdictKind enum | `verdict-kind-enumeration` | IMPLEMENTED — existing |
| `VerificationCases.sysml:70-79` PassIf | `passif-boolean-to-verdict` | DOCUMENTED — existing |
| `VerificationCases.sysml:81-101` VerificationMethod metadata + VerificationMethodKind enum | **NEW: `verification-method-metadata`** | OUT-OF-SCOPE for runtime — this is an annotation/metadata construct; no verdict-computation obligation; tool-defined read/write behavior. |
| `Cases.sysml:46` `subject subj default Case::result` (base Case obj default binding) | **NEW: `case-objective-default-subject-binding`** | OUT-OF-SCOPE — abstract base; the concrete obligation is captured under `analysis-case-objective-bound-to-result` and `verification-subject-bound-to-objective-subject`. |
| `AnalysisCases.sysml:24` `subAnalysisCases` sub-case nesting | **NEW: `sub-analysis-case-nesting`** | STRUCTURAL — not yet in table; sub-case specialization constraint matches `checkAnalysisCaseUsageSubAnalysisCaseSpecialization` §8.3.23.3. Validation sweep gap only; no runtime/behavioral obligation. |
| `VerificationCases.sysml:42` `subVerificationCases` | **NEW: `sub-verification-case-nesting`** | STRUCTURAL — same pattern; §8.3.24.4 `checkVerificationCaseUsageSubAnalysisCaseSpecialization`. Validation sweep gap only. |

### NEW obligations found that are MISSED (behavioral)

None. All newly enumerated units are either already captured, structural validation gaps, or genuinely out-of-scope (tool-defined / annotation / abstract-base).

The two structural additions worth adding to the table on a future sweep:
- `case-actor-parameter` — §8.3.22.2 `deriveCaseDefinitionActorParameter` (ActorMembership); parser stamps actor parameters; no behavioral gap.
- `sub-analysis-case-nesting` / `sub-verification-case-nesting` — §8.3.23.3 / §8.3.24.4 specialization constraints on composite sub-case usages; structural-only.

**Honesty line:** The one confirmed behavioral gap, `analysis-case-objective-bound-to-result` (GAP-VER-ANALYSIS), is now CLOSED (FORM-B Tier-1, `2026-06-22`) — literal-result binding through the shared objective→verdict pipeline; expression/calc and sim/solver result sources (Tier-2/Tier-3) remain on the design-plan ladder. No net-new behavioral MISSED obligations were found. The `verdict-criteria-modeled-explicitly` SPEC-SILENT flag is confirmed correct (§8.4.20.1 mandates explicit modeling but is silent on a default when absent).
