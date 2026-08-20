# Obligation matrix — Requirements

**Area:** requirement satisfaction semantics.
**Gate:** `tests/requirement_spec_conformance.rs`.
**Status:** fan-out area (format approved at pilot).

Spec sources: SysML §7.21 *Requirements* (`SysML-spec-r2025-04_REF.html`);
`sysml.library/Systems Library/Requirements.sysml`. Citations verified
`2026-06-21`.

Current-behavior anchor (read-only): `crates/lang/sysml-runtime/src/constraints.rs`
`evaluate_requirement` / `RequirementConstraintIR { assumptions, constraints, is_negated }`.

> **Subject binding is the dividing line.** The requirement *satisfaction logic*
> (assumption⇒required implication, negation) is implemented and gated here. The
> requirement *subject* (subject parameter, subrequirement subject inheritance,
> conformance-to-subject-type, satisfy-binding) is the **in-flight verification /
> requirement subject-binding build** (see project memory, Inc1c). Those rows are
> **DEFERRED** to that build to avoid collision — not new gaps.

## Obligation table

| ID | Obligation | Citation (tier) | Gate | Verdict |
|----|-----------|-----------------|------|---------|
| `requirement-is-constraint-satisfied-iff-true` | A requirement is a kind of constraint, satisfied iff it evaluates to true. | §7.21.1 *"a requirement is satisfied when it evaluates to true."* (GOSPEL) | `req_satisfied_iff_required_constraint_true` | **CONFORMS** |
| `requirement-result-is-assumption-implies-required` | Effective result = `allTrue(assumptions) implies allTrue(constraints)`; required constraints checked only when assumptions hold. | §7.21.2 + `Requirements.sysml:41` `return result = allTrue(assumptions()) implies allTrue(constraints())` (GOSPEL+LIBRARY) | `req_required_checked_only_when_assumptions_hold` | **CONFORMS** |
| `assumption-false-required-vacuously-satisfied` | A false assumption ⇒ requirement vacuously satisfied. | `Requirements.sysml:41` (LIBRARY, necessary consequence of `implies`) | `req_vacuously_satisfied_when_assumption_false` | **CONFORMS** |
| `negated-satisfy-requires-not-satisfied` | A negated satisfy usage asserts the requirement evaluates to false; verdict inverts. | §7.21.4 + `Requirements.sysml:180` `notSatisfiedRequirementChecks :> negatedConstraintChecks` (GOSPEL+LIBRARY) | `req_negation_inverts_verdict` | **CONFORMS** |
| `requirement-check-result-is-boolean` | Every RequirementCheck result is Boolean. | §8.4.17.1 (RequirementDefinition is-a ConstraintDefinition, Boolean[1] result) (GOSPEL) | `req_inconclusive_propagates_from_unbound_required_constraint` | **CONFORMS** (unbound required ⇒ inconclusive, not coerced) |
| `require-constraint-membership-kind` | Assumed vs required constraints partitioned by `RequirementConstraintMembership.kind`. | §8.3.21.6/7 (STRUCTURAL) | _(transitive)_ | **CONFORMS** — the runtime partitions `assumptions` vs `constraints`; exercised by the implication test. |
| `satisfy-requirement-usage-asserts-true` | A non-negated satisfy usage asserts the requirement is always true; a false result is a logical inconsistency the tool flags. | §7.21.4 (GOSPEL) | — | **DEFERRED** — verdict-meaning (assert⇒Fail) is the per-instance check/verdict layer, same family as the constraints-area GAP-2. |
| `satisfy-requirement-usage-requires-binding-connector` | A satisfy usage has one BindingConnector binding the subject to the satisfying feature. | §8.3.21.10 (STRUCTURAL / subject) | — | **DEFERRED** — subject-binding build. |
| `subrequirement-inherits-subject-when-undeclared` | A subrequirement with no declared subject inherits the container's subject. | §7.21.2 (GOSPEL / subject) | — | **DEFERRED** — subject-binding build. |
| `satisfy-without-explicit-by-uses-containing-feature` | A nested satisfy usage with no `by` binds the satisfying feature to the containing usage (`Base::things::that`). | §7.21.4 (GOSPEL / subject) | — | **DEFERRED** — subject-binding build. |
| `requirement-usage-entity-must-conform-to-subject-type` | A requirement can only be satisfied by an entity conforming to the subject's definition. | §7.21.1 (GOSPEL / typing) | — | **DEFERRED** — needs subject type + a type-conformance check; subject-binding build. |
| `subrequirement-is-required-constraint` | A nested composite requirement usage is automatically a required constraint of its container. | §7.21.2 (GOSPEL) | — | **DOCUMENTED (gap candidate)** — `evaluate_requirement` aggregates child assume/require/constraint props; whether it recurses into nested *requirement usages* as sub-checks is unverified. Needs a probe before gating. |
| `framed-concern-is-required-constraint` | A framed concern usage is a subrequirement and thus a required constraint. | §7.21.3 + `Requirements.sysml:97` (GOSPEL+LIBRARY) | — | **DOCUMENTED** — depends on the same subrequirement-aggregation path. |
| `requirement-subject-must-be-first-parameter` | The subject parameter must be the requirement's first input. | §8.3.21 `validateRequirementDefinitionSubjectParameterPosition` (GOSPEL OCL: `input->notEmpty() and input->first() = subjectParameter`) | `requirement_subject_not_first_parameter_is_flagged` | **CONFORMS** (`2026-06-21`) — S141/S142 (`subject_is_first_parameter`, semantic_checks/requirements.rs) flag a directed `in`/`inout` input declared before the subject. KerML `/input` = direction `in`/`inout` features; a directionless `attribute` is not an input, so only a *directed* param before the subject violates. |
| `requirement-subject-default-is-anything` | An undeclared subject is implicitly typed `Anything[1]`. | §7.21.2 + `Requirements.sysml:58` (GOSPEL+LIBRARY / typing) | — | **STRUCTURAL** — elaboration/typing concern; cross-ref. |
| `at-most-one-subject` | A requirement has at most one subject (a `SubjectMembership` is `[0..1]`). | §8.3.21 `SubjectMembership` subject cardinality `[0..1]` (GOSPEL, STRUCTURAL) | `structural_spec_conformance::req_two_subjects_is_flagged` | **CONFORMS** — S060/S061 (`cardinality::at_most_one_subject`); two subjects on a requirement def raise S060, a single subject is clean. |

## Coverage

- Gated / behaviorally-gateable = **6 / 8** (the 5 CONFORMS + the partition row, vs + `subrequirement-is-required-constraint` and `framed-concern`) = **75%** of the *satisfaction-logic* obligations.
- The 5 subject-binding rows are **DEFERRED to the in-flight build**, not counted as this sweep's gaps.
- All obligations (incl. 2 structural + 5 deferred): **6 / 15 = 40%**.

## Ranked findings

1. **No new GAP at the satisfaction-logic layer** — implication + vacuous-truth + negation all CONFORM.
2. **`subrequirement-is-required-constraint` / `framed-concern` (DOCUMENTED)** — verify whether `evaluate_requirement` recurses into nested requirement usages; if not, that is a real aggregation gap. One read-only probe needed before it can be gated. Candidate follow-up.
3. **Subject binding (5 rows, DEFERRED)** — owned by the in-flight verification/requirement subject-binding build; this sweep should gate them once that build lands, not duplicate it now.

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `requirement-is-constraint-satisfied-iff-true` | `$SYS` §7.21.1 | `grep -n -i "can be evaluated to be true or false" /tmp/sys.txt` |
| `requirement-result-is-assumption-implies-required` | `$SYS` §7.21.2 | `grep -n -i "effective constraint for the requirement" /tmp/sys.txt` |
| same (library formula) | `sysml.library/Systems Library/Requirements.sysml` | `grep -n "allTrue(assumptions())" "<file>"` |
| `negated-satisfy-requires-not-satisfied` | `$SYS` §7.21.4 | `grep -n -i "negated satisfy requirement usage asserts" /tmp/sys.txt` |
| `subrequirement-is-required-constraint` | `$SYS` §7.21.2 | `grep -n -i "nested composite requirement usage is automatically considered" /tmp/sys.txt` |
| `requirement-subject-default-is-anything` | `$SYS` §7.21.2 / library | `grep -n -i "implicitly assumed to be defined as Anything" /tmp/sys.txt`; `grep -n "subject subj : Anything" "<Requirements.sysml>"` |

## Completeness audit — clauses reviewed (2026-06-21)

**Sections reviewed:** §7.21.1–7.21.4 (prose), §8.3.21.1–8.3.21.12 (abstract syntax), §8.4.17.1–8.4.17.5 (semantics), `Requirements.sysml` (normative library, all features).

**Methodology:** stripped HTML → grepped section markers; read all normative prose, named abstract-syntax constraints/operations, and library feature declarations. Each was classified against the existing obligation table.

| Unit | Clause | Classification | ID or reason |
|------|--------|---------------|--------------|
| Requirement satisfied iff evaluates to true | §7.21.1 | CAPTURED | `requirement-is-constraint-satisfied-iff-true` |
| Entity must conform to subject's definition | §7.21.1 | CAPTURED (DEFERRED) | `requirement-usage-entity-must-conform-to-subject-type` |
| `allTrue(assumptions) implies allTrue(constraints)` formula | §7.21.2 + library:41 | CAPTURED | `requirement-result-is-assumption-implies-required` |
| False assumption ⇒ vacuous satisfaction | library:41 | CAPTURED | `assumption-false-required-vacuously-satisfied` |
| Nested composite requirement = required constraint | §7.21.2 | CAPTURED (DOCUMENTED) | `subrequirement-is-required-constraint` |
| Subrequirement without explicit subject inherits container subject | §7.21.2 | CAPTURED (DEFERRED) | `subrequirement-inherits-subject-when-undeclared` |
| Undeclared subject ⇒ `Anything[1]` | §7.21.2 + library:58 | CAPTURED (STRUCTURAL) | `requirement-subject-default-is-anything` |
| Subject must be first parameter | §7.21.2 / §8.3.21.8–9 | CAPTURED (STRUCTURAL) | `requirement-subject-must-be-first-parameter` |
| Actor/stakeholder params declared with `actor`/`stakeholder` | §7.21.2 | STRUCTURAL | model well-formedness; no evaluation impact |
| Framed concern = subrequirement = required constraint | §7.21.3 + library:97 | CAPTURED (DOCUMENTED) | `framed-concern-is-required-constraint` |
| Non-negated satisfy asserts requirement always true | §7.21.4 | CAPTURED (DEFERRED) | `satisfy-requirement-usage-asserts-true` |
| Negated satisfy asserts requirement false | §7.21.4 + library:180 | CAPTURED | `negated-satisfy-requires-not-satisfied` |
| No explicit `by` → satisfying feature = `Base::things::that` | §7.21.4 / §8.4.17.3 | CAPTURED (DEFERRED) | `satisfy-without-explicit-by-uses-containing-feature` |
| SatisfyRequirementUsage needs BindingConnector to subject | §8.3.21.10 / §8.4.17.3 | CAPTURED (DEFERRED) | `satisfy-requirement-usage-requires-binding-connector` |
| `ActorMembership.validateActorMembershipOwningType` | §8.3.21.2 | STRUCTURAL | owning-type well-formedness |
| `ConcernDefinition.checkConcernDefinitionSpecialization` | §8.3.21.3 | STRUCTURAL | library specialization chain |
| `ConcernUsage.checkConcernUsageSpecialization` | §8.3.21.4 | STRUCTURAL | library specialization chain |
| `ConcernUsage.checkConcernUsageFramedConcernSpecialization` | §8.3.21.4 | STRUCTURAL | library specialization chain |
| `FramedConcernMembership.validateFramedConcernMembershipConstraintKind` | §8.3.21.5 | STRUCTURAL | model well-formedness |
| `RequirementConstraintKind` enum (assumption / requirement) | §8.3.21.6 | CAPTURED | `require-constraint-membership-kind` |
| `RequirementConstraintMembership` derive/validate constraints (3) | §8.3.21.7 | STRUCTURAL | derive formulae + well-formedness |
| `RequirementDefinition` derive/validate constraints (6, incl. subjectParameter position) | §8.3.21.8 | STRUCTURAL (with 1 CAPTURED) | `requirement-subject-must-be-first-parameter`; rest are model well-formedness |
| `RequirementUsage.checkRequirementUsageObjectiveRedefinition` | §8.3.21.9 | OUT-OF-SCOPE | Cases / ObjectiveMembership domain; not a satisfaction-logic obligation |
| `RequirementUsage.checkRequirementUsageRequirementVerificationSpecialization` | §8.3.21.9 | OUT-OF-SCOPE | VerificationCases domain |
| `RequirementUsage` remaining derive/validate constraints (5) | §8.3.21.9 | STRUCTURAL | library specialization + well-formedness |
| `SatisfyRequirementUsage.checkSatisfyRequirementUsageBindingConnector` | §8.3.21.10 | CAPTURED (DEFERRED) | `satisfy-requirement-usage-requires-binding-connector` |
| `SatisfyRequirementUsage.checkSatisfyRequirementUsageSpecialization` | §8.3.21.10 | STRUCTURAL | library specialization chain |
| `SatisfyRequirementUsage.deriveSatisfyRequirementUsageSatisfyingFeature` | §8.3.21.10 | CAPTURED (DEFERRED) | subject-binding build |
| `SatisfyRequirementUsage.validateSatisfyRequirementUsageReference` | §8.3.21.10 | STRUCTURAL | model well-formedness |
| `SubjectMembership.validateSubjectMembershipOwningType` | §8.3.21.11 | STRUCTURAL | owning-type well-formedness |
| `StakeholderMembership.validateStakeholderMembershipOwningType` | §8.3.21.12 | STRUCTURAL | owning-type well-formedness |
| §8.4.17.1: `subrequirements` + `concerns` feed into `allTrue(constraints())` | §8.4.17.1 + library:90,97 | CAPTURED (DOCUMENTED) | same path as `subrequirement-is-required-constraint` / `framed-concern-is-required-constraint` |
| §8.4.17.1: actors/stakeholders collected into `RequirementCheck` features | §8.4.17.1 + library:65,74 | OUT-OF-SCOPE | no behavioral evaluation impact on satisfaction verdict |
| §8.4.17.2: composite RequirementUsage subsets `subrequirements` (library specialization) | §8.4.17.2 | STRUCTURAL | library specialization constraint |
| §8.4.17.3: no-`by` SatisfyRequirementUsage binds `Base::things::that` | §8.4.17.3 | CAPTURED (DEFERRED) | `satisfy-without-explicit-by-uses-containing-feature` |
| §8.4.17.4–5: ConcernDefinition / ConcernUsage semantics (specialization chains) | §8.4.17.4–5 | STRUCTURAL | library specialization chain |
| `RequirementConstraintCheck` private base (library:19–47) | library | STRUCTURAL | implements the `implies` formula directly; exercised by CONFORMS rows |
| `FunctionalRequirementCheck`, `InterfaceRequirementCheck`, `PerformanceRequirementCheck`, `PhysicalRequirementCheck`, `DesignConstraintCheck` (library:106–153) | library | OUT-OF-SCOPE | specialised requirement flavours; subject-type typing only, no distinct eval logic |

**Counts:** CAPTURED = 14 (of which 5 CONFORMS gated, 2 DOCUMENTED gap candidate, 5 DEFERRED, 2 STRUCTURAL) · STRUCTURAL = 17 · OUT-OF-SCOPE = 6 · **MISSED = 0**.

**Denominator assessment:** Denominator closed for §7.21.1–7.21.4 prose, §8.3.21.2–8.3.21.12 named constraints/operations, §8.4.17.1–8.4.17.5 semantics prose, and all `Requirements.sysml` library feature declarations. No behavioral obligation found outside the existing matrix rows.
