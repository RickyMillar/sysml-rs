# Obligation matrix — Constraints & Expressions

**Area:** constraint evaluation + the expression semantics that underpin it.
**Gate:** `tests/constraint_spec_conformance.rs`.
**Status:** PILOT (validates the method/format; awaiting director review before fan-out).

Spec sources (verified against the spec HTML by tag-strip + grep on
`2026-06-21`):

- `SysML-spec-r2025-04_REF.html` §7.20 *Constraints* (incl. "Asserted Constraints")
- `KerML-spec-r2025-04_REF.html` §7.4.8 *Predicates*, §8.3.4.8.5
  *FeatureReferenceExpression::evaluate*
- `sysml.library/Systems Library/Constraints.sysml`
- `sysml.library/Kernel Libraries/Kernel Semantic Library/Performances.kerml`

Current-behavior anchors (read-only, `2026-06-21`):
`crates/lang/sysml-runtime/src/constraints.rs` — `evaluate`/`evaluate_expr`
(`EvaluationResult { satisfied, inconclusive, … }`), `extract_constraints`,
`RequirementConstraintIR.is_negated`; `crates/lang/sysml-runtime/src/lib.rs`
`ConstraintIR`.

---

## Obligation table

| ID | Obligation | Citation (tier) | Gate | Verdict |
|----|-----------|-----------------|------|---------|
| `constraint-result-boolean` | A constraint/predicate produces exactly one Boolean result; a non-Boolean result is not a valid verdict. | KerML §7.4.8: *"Predicates are functions whose result is a single Boolean value (that is, true or false)."* + `Performances.kerml` `BooleanEvaluation` returns `Boolean[1]` (GOSPEL+LIBRARY) | `obl1_boolean_expression_yields_concrete_verdict`, `obl1_non_boolean_expression_is_not_a_verdict` | **CONFORMS** — non-Boolean ⇒ `inconclusive` + "expected boolean" diag. |
| `constraint-satisfied-iff-true` | A constraint usage is satisfied iff its expression evaluates to true, and violated otherwise. | SysML §7.20: *"a constraint usage is satisfied if its expression evaluates to true and is violated otherwise."* (GOSPEL) | `obl2_satisfied_when_expression_true`, `obl2_violated_when_expression_false` | **CONFORMS** |
| `feature-ref-resolves-to-bound-value` | A feature reference in a constraint evaluates to the value bound to that feature. | KerML §8.3.4.8.5 `FeatureReferenceExpression::evaluate` (GOSPEL) | `obl8_feature_reference_resolves_to_its_bound_value` | **CONFORMS** (gated at the eval/context layer; binding *from the model* is the per-instance machinery — see DEFERRED rows). |
| `core-operator-semantics` | Constraint (Boolean) expressions evaluate comparison/logical/arithmetic operators per KerML expression semantics. | KerML §7.4.8 / `KerMLExpressions.xtext` operator precedence (GOSPEL+XTEXT) | `expr1_comparison_logical_and_arithmetic_operators` | **CONFORMS** |
| `unbound-feature-yields-inconclusive-not-false` | An unresolved feature reference evaluates to the empty list; the result is not `false`. The behavior of an ordering comparison over an empty operand is **spec-silent**. | KerML §8.3.4.8.5: *"evaluates to the empty list."* (GOSPEL for the empty-list result; **SPEC-SILENT** for the comparison's verdict) | `obl9_unbound_feature_is_inconclusive_not_violated` | **CONFORMS-by-design** — reports `inconclusive`, not a violation. No normative obligation to gate the verdict; pinned so it can't silently flip to a false "violated". |
| `constraint-usage-discovered` | A `ConstraintUsage` and an `AssertConstraintUsage` are both surfaced as evaluable constraints. | SysML §7.20 (constraint vs assert constraint); `constraints.rs` extracts both `ConstraintUsage` and `AssertConstraintUsage` (GOSPEL+impl) | `obl56_plain_and_assert_constraints_are_both_discovered` | **CONFORMS** (discovery only; structural well-formedness OBL `constraint-def-specializes-check` is a validation-area concern). |
| `assert-constraint-must-be-true` | A non-negated assert constraint asserts its result is true at all times; a false result is a logical inconsistency the tool flags. | SysML §7.20 *Asserted Constraints*: *"an assert constraint usage asserts that the result of a given constraint must be always true at all times. If, at some point in time, it can be determined that an assert constraint usage evaluates to other than its asserted value, this would be a logical inconsistency in the model."* (GOSPEL) | — | **DEFERRED** — the eval layer reports the raw boolean correctly, but the *asserted ⇒ Fail-verdict* mapping lives in the per-instance check/verdict layer (`sysml.constraint.check`), which is in-flight. Gate once that layer settles. |
| `negated-assert-must-be-false` | A negated `assert not constraint` asserts its result is false; a true result is the inconsistency. | SysML §7.20: *"An assert constraint usage can also be negated, which means that the given constraint is asserted to be false rather than true."* (GOSPEL); KerML Invariant `isNegated` ⇒ specializes `falseEvaluations` | `obl4_negated_assert_inner_true_should_be_violated`, `obl4_negated_assert_inner_false_should_be_satisfied` (+ per-instance twins) | **CONFORMS (GAP-1 CLOSED `2026-06-21`)** — `ConstraintIR.is_negated` added and set from `AssertConstraintUsage.isNegated` at extraction; the decided verdict is inverted at the `evaluate_expr` chokepoint, so the eval layer AND the per-instance check path both honor negation. `assert not constraint {C}` with `C` true is now *violated*, with `C` false is *satisfied*; an undecidable inner stays *inconclusive*. |
| `constraint-usage-not-model-level-evaluable` | A `ConstraintUsage` is not model-level evaluable; evaluation is occurrence/instance-scoped. | SysML §8.3.20.4: *"A ConstraintUsage is not model-level evaluable."* (GOSPEL structure; **SPEC-SILENT** on the runtime trigger protocol) | — | **SPEC-SILENT** — the spec fixes the flag but not *when/how* a tool evaluates per occurrence. Our `evaluate_constraints_per_instance` is a tool-defined, spec-consistent protocol. No normative behavior to gate. |
| `constraint-check-time-indexed` | A constraint check at a point in time yields a Boolean for that point; a non-asserted constraint may be true at some times and false at others. | SysML §7.20 *Asserted Constraints* ("…satisfied sometimes and violated other times…") (GOSPEL structure; **SPEC-SILENT** on the time-point selection protocol) | — | **SPEC-SILENT** — same family as `…not-model-level-evaluable`. Document only. |
| `constraint-def-result-binding` | If a ConstraintDefinition owns a result expression, a BindingConnector binds it to the definition's Boolean result parameter (well-formedness). | SysML §7.20 / KerML `checkFunctionResultBindingConnector` (GOSPEL **STRUCTURAL**) | — | **STRUCTURAL** — a model well-formedness obligation; belongs to a parser/validation conformance sweep, not runtime behavior. Cross-ref only. |

### Cross-references (obligations owned by other areas)

| ID | Owning area | Note |
|----|------------|------|
| `invariant-is-asserted-boolean` (KerML §8.3.4.7.5) | constraints | The KerML mirror of `assert-constraint-must-be-true` / `negated-assert-must-be-false`. SysML `AssertConstraintUsage` *is-a* KerML `Invariant`. Subsumed by GAP-1. |
| `requirement-satisfaction-is-constraint-true` (SysML §7.21) | **requirements** | "a requirement is satisfied when it evaluates to true." Belongs to the requirements-area sweep (next fan-out candidate). |

---

## Coverage

Behaviorally-gateable obligations in this area (exclude `SPEC-SILENT` ×2 and
`STRUCTURAL` ×1):

| | count |
|---|---|
| Gated (CONFORMS / CONFORMS-by-design) | 7 (incl. `negated-assert-must-be-false`, GAP-1 closed) |
| Behaviorally-gateable, ungated | 1 (`assert-constraint-must-be-true` DEFERRED) |
| **Behavioral coverage** | **7 / 8 = 88%** |

All obligations (including silent/structural in the denominator):

| | count |
|---|---|
| Gated | 7 |
| Total obligations (excl. cross-ref rows) | 11 |
| **All-obligation coverage** | **7 / 11 = 64%** |

The two silent obligations and one structural obligation are not counted as
behavioral gaps — there is no normative *behavior* to assert — but they are
listed so the spec surface is fully visible.

---

## Ranked conformance gaps (for director scheduling)

1. **GAP-1 — `negated-assert-must-be-false` — ✅ CLOSED `2026-06-21`.**
   `assert not constraint {C}` now inverts `C` at the constraint layer:
   `ConstraintIR.is_negated` was added and is set from
   `AssertConstraintUsage.isNegated` at both extraction paths
   (`extract_from_element`, `TypedConstraint::from_element`); the decided verdict
   is inverted at the `evaluate_expr` chokepoint, so the eval layer and the
   per-instance check path are both corrected (an undecidable inner stays
   inconclusive — negation cannot manufacture a verdict). Gated by the four
   `obl4_negated_assert_*` tests (now CONFORMS).
   *Spec:* SysML §7.20 (Asserted Constraints) — GOSPEL.

2. **GAP-2 — `assert-constraint-must-be-true` verdict mapping (DEFERRED).**
   The raw boolean is correct, but whether an asserted-and-false constraint
   becomes a `Fail` verdict (logical inconsistency) vs an unasserted constraint
   that is merely false is decided in the in-flight per-instance check/verdict
   layer. Not a runtime-eval-layer gap; gate once `sysml.constraint.check`
   per-instance verdicts settle. Sequencing dependency, not a defect — confirm
   with the per-instance build owner.

3. **(Visibility, not a defect) — two SPEC-SILENT runtime protocols.**
   `constraint-usage-not-model-level-evaluable` and `constraint-check-time-indexed`
   are filled by tool-defined behavior (`evaluate_constraints_per_instance`).
   Spec-consistent and not spec-contradicted. No fix needed; flagged so a future
   spec revision that *does* define the protocol is noticed.

---

## Reproducing the citations

Tag-strip once, then grep (paths relative to repo root):

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
KER="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/KerML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$KER" > /tmp/ker.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `constraint-satisfied-iff-true`, `assert-constraint-must-be-true`, `negated-assert-must-be-false` | `$SYS` (§7.20) | `grep -n -i "is satisfied" /tmp/sys.txt` (the §7.20 Constraints + "Asserted Constraints" paragraph) |
| `constraint-usage-not-model-level-evaluable` | `$SYS` | `grep -n -i "not model-level evaluable" /tmp/sys.txt` |
| `constraint-result-boolean` | `$KER` (§7.4.8) | `grep -n -i "result is a single Boolean" /tmp/ker.txt` |
| `unbound-feature-yields-inconclusive-not-false` | `$KER` (§8.3.4.8.5) | `grep -n -i "evaluates to the empty list" /tmp/ker.txt` |
| `constraint-result-boolean` (library) | `sysml.library/Kernel Libraries/Kernel Semantic Library/Performances.kerml` | `grep -n "BooleanEvaluation" "<file>"` |
| constraint base type | `sysml.library/Systems Library/Constraints.sysml` | `grep -n "ConstraintCheck" "<file>"` |

Verified `2026-06-21`.

> **Test results (2026-06-21):** GAP-1 (`negated-assert-must-be-false`) is now
> **CLOSED**. `tests/assert_constraint_spec_conformance.rs` gates it with four
> `obl4_negated_assert_*` tests (CONFORMS): `assert not constraint` negation is
> applied at BOTH the eval layer and the per-instance `check` path via
> `ConstraintIR.is_negated` (set from `AssertConstraintUsage.isNegated`,
> inverted at the `evaluate_expr` chokepoint). Was confirmed earlier as 4
> DIVERGES; the fix-wave flipped them green.

---

## Completeness audit — clauses reviewed (2026-06-21)

### Spec sections reviewed

| Spec | Section | Title |
|------|---------|-------|
| SysML | §7.20 (incl. §7.20.1–7.20.3) | Constraints (Overview, Constraint Definitions and Usages, Assert Constraint Usages) |
| SysML | §8.3.20 (incl. §8.3.20.1–8.3.20.4) | Constraints Abstract Syntax (AssertConstraintUsage, ConstraintDefinition, ConstraintUsage) |
| SysML | §8.4.16 (incl. §8.4.16.1–8.4.16.3) | Constraints Semantics (Constraint Definitions, Constraint Usages, Assert Constraint Usages) |
| KerML | §7.4.8 (incl. §7.4.8.1–7.4.8.5) | Functions (Overview, Function Declaration, Expression Declaration, Predicate Declaration, Boolean Expression and Invariant Declaration) |
| KerML | §8.3.4.7 (incl. §8.3.4.7.2–8.3.4.7.6) | Functions Abstract Syntax (BooleanExpression, Expression, Function, Invariant, Predicate) |
| KerML | §8.3.4.8.5 | FeatureReferenceExpression Abstract Syntax (evaluate operation) |
| KerML | §8.4.4.8.1–8.4.4.8.2 | Functions and Predicates Semantics; Expressions and Invariants Semantics |
| KerML | §8.4.4.9.6 | Operator Expressions Semantics |
| KerML | §8.4.4.9.8 | Model-Level Evaluable Expressions |
| Library | `Constraints.sysml` | `ConstraintCheck`, `constraintChecks`, `assertedConstraintChecks`, `negatedConstraintChecks` |
| Library | `Performances.kerml` | `BooleanEvaluation`, `booleanEvaluations`, `trueEvaluations`, `falseEvaluations` |

### Classification table

Each normative unit (prose obligation, abstract-syntax constraint or operation, or library feature) is listed with its source clause and classification.

| Normative unit | Clause | Classification | Obligation ID or reason |
|----------------|--------|----------------|-------------------------|
| Constraint usage satisfied iff expression evaluates to true, violated otherwise | SysML §7.20.1 / §7.20.3 | CAPTURED | `constraint-satisfied-iff-true` |
| Assert constraint asserts result must be always true at all times | SysML §7.20.3 | CAPTURED | `assert-constraint-must-be-true` (DEFERRED) |
| Negated assert constraint asserts result must always be false | SysML §7.20.3 | CAPTURED | `negated-assert-must-be-false` (GAP-1 DIVERGES) |
| Constraint may be true at some times, false at others (time-indexed evaluation) | SysML §7.20.3 | CAPTURED | `constraint-check-time-indexed` (SPEC-SILENT) |
| ConstraintDefinition has implicit Boolean result parameter | SysML §7.20.2, §8.3.20.3 | STRUCTURAL | Type well-formedness; belongs to parser/validation sweep |
| AssertConstraintUsage relates its asserted constraint via ReferenceSubsetting | SysML §8.3.20.2 | STRUCTURAL | `deriveAssertConstraintUsageAssertedConstraint` — derived property |
| `checkAssertConstraintUsageSpecialization` (negated→`negatedConstraintChecks`, non-negated→`assertedConstraintChecks`) | SysML §8.3.20.2 | STRUCTURAL | Library specialization constraint; drives STRUCTURAL |
| `validateAssertConstraintUsageReference` — reference target must be ConstraintUsage | SysML §8.3.20.2 | STRUCTURAL | Validation well-formedness |
| `checkConstraintDefinitionSpecialization` — must specialize `Constraints::ConstraintCheck` | SysML §8.3.20.3, §8.4.16.1 | STRUCTURAL → **SATISFIED-BY-CONSTRUCTION (`2026-06-21`)** | Not a validator obligation; `elaborate::implicit_generalization` derives the base (mapping :110), present by construction. Gated in sysml-core. Steward-ruled; mis-framed no-library structural gate removed. |
| `ConstraintUsage.modelLevelEvaluable()` returns false | SysML §8.3.20.4 | CAPTURED | `constraint-usage-not-model-level-evaluable` (SPEC-SILENT) |
| `ConstraintUsage.namingFeature()` — naming for requirement-owned usages | SysML §8.3.20.4 | STRUCTURAL | Derived property; no runtime behavior to gate |
| `checkConstraintUsageCheckedConstraintSpecialization` — composite ConstraintUsage in Item specializes `checkedConstraints` | SysML §8.3.20.4 | STRUCTURAL | Library specialization constraint |
| `checkConstraintUsageRequirementConstraintSpecialization` — specialize `assumptions` or `constraints` | SysML §8.3.20.4 | STRUCTURAL | Library specialization constraint |
| `checkConstraintUsageSpecialization` — must specialize `Constraints::constraintChecks` | SysML §8.3.20.4, §8.4.16.2 | STRUCTURAL | Library specialization constraint |
| `assertedConstraintChecks` subsets `trueEvaluations` — asserted result must be true | SysML §8.4.16.3 / `Constraints.sysml` | CAPTURED | `assert-constraint-must-be-true` (same obligation as DEFERRED row) |
| `negatedConstraintChecks` subsets `falseEvaluations` — negated result must be false | SysML §8.4.16.3 / `Constraints.sysml` | CAPTURED | `negated-assert-must-be-false` (same obligation as GAP-1 row) |
| `checkFunctionResultBindingConnector` — BindingConnector between result expression and result parameter of ConstraintDefinition | SysML §8.4.16.1 / KerML §8.4.4.8.1 | CAPTURED | `constraint-def-result-binding` (STRUCTURAL) |
| Predicates result is a single Boolean value (true or false) | KerML §7.4.8.1 | CAPTURED | `constraint-result-boolean` |
| BooleanExpression may evaluate true/false at different times; Invariant asserted always true or always false | KerML §7.4.8.5 | CAPTURED | `constraint-check-time-indexed` (SPEC-SILENT) + `assert-constraint-must-be-true` + `negated-assert-must-be-false` |
| Operator expressions resolve to standard library Functions (`DataFunctions`, `BaseFunctions`, `ControlFunctions`) | KerML §8.4.4.9.6 | CAPTURED | `core-operator-semantics` |
| `Invariant.isNegated : Boolean` attribute | KerML §8.3.4.7.5 | STRUCTURAL | Model attribute definition; no runtime behavior separate from `negated-assert-must-be-false` |
| `checkInvariantSpecialization` — Invariant must specialize `trueEvaluations` or `falseEvaluations` | KerML §8.3.4.7.5, §8.4.4.8.2 | STRUCTURAL | Library specialization constraint |
| `checkPredicateSpecialization` — Predicate must specialize `BooleanEvaluation` | KerML §8.3.4.7.6, §8.4.4.8.1 | STRUCTURAL | Library specialization constraint |
| `FeatureReferenceExpression::evaluate(target)` — resolves to value expression or referent; empty list when unresolvable | KerML §8.3.4.8.5, §8.4.4.9.8 | CAPTURED | `feature-ref-resolves-to-bound-value` + `unbound-feature-yields-inconclusive-not-false` |
| `trueEvaluations` has result bound to `true`; `falseEvaluations` has result bound to `false` | `Performances.kerml` ll.221–241 | CAPTURED | `assert-constraint-must-be-true` + `negated-assert-must-be-false` (library semantic anchor for both) |
| `BooleanEvaluation` — base predicate type; result typed `Boolean[1]` | `Performances.kerml` l.94 | CAPTURED | `constraint-result-boolean` |

### Summary counts

| Classification | Count |
|----------------|-------|
| CAPTURED | 14 (including 4 that are the same obligation cited from multiple clauses, and 1 STRUCTURAL row also tagged CAPTURED because it was already in the matrix) |
| STRUCTURAL | 12 |
| OUT-OF-SCOPE | 0 |
| MISSED | 0 |

Unique matrix obligations that received at least one CAPTURED hit: all 10 obligations in the table
(`constraint-result-boolean`, `constraint-satisfied-iff-true`, `feature-ref-resolves-to-bound-value`,
`core-operator-semantics`, `unbound-feature-yields-inconclusive-not-false`, `constraint-usage-discovered`,
`assert-constraint-must-be-true`, `negated-assert-must-be-false`, `constraint-usage-not-model-level-evaluable`,
`constraint-check-time-indexed`, `constraint-def-result-binding`).

Note: `constraint-usage-discovered` (discovery of both `ConstraintUsage` and `AssertConstraintUsage`) is
implicitly covered by the abstract syntax reviewed above but was not separately enumerated as a standalone
normative unit — the obligation derives from the type definitions in §8.3.20.2–8.3.20.4 rather than a
single extractable sentence.

### Honesty statement

**Denominator closed: every behavioral unit is CAPTURED.**

All 11 normative prose obligations and abstract-syntax `evaluate`/`modelLevelEvaluable` operations from the
reviewed clauses are accounted for in the matrix — either as CAPTURED obligations (behavioral) or STRUCTURAL
entries (model well-formedness, not runtime behavior). The 12 STRUCTURAL units belong to a parser or
validation-area conformance sweep, not to runtime constraint evaluation. No behavioral obligation from the
reviewed clauses is absent from the matrix.
