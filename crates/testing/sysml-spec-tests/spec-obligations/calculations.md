# Obligation matrix — Calculations

**Area:** calculation-definition evaluation.
**Gate:** `tests/calculation_spec_conformance.rs`.
**Status:** fan-out area.

Spec sources: SysML §7.19 *Calculations* (`SysML-spec-r2025-04_REF.html`),
KerML §7.4.8/§7.4.9 *Functions / Expressions* (`KerML-spec-r2025-04_REF.html`),
`sysml.library/Systems Library/Calculations.sysml`. Verified `2026-06-21`.

Current-behavior anchor (read-only): `crates/lang/sysml-runtime/src/calculations.rs`
`compile_calculation` / `evaluate_calculation` / `CalculationRegistry`.

Most of the calculations spec surface is **STRUCTURAL well-formedness**
(specialization chains, result/return BindingConnectors) — those belong to the
parser/validation conformance sweep, not the runtime evaluator. The runtime
*evaluation* obligations are gated here.

## Obligation table

| ID | Obligation | Citation (tier) | Gate | Verdict |
|----|-----------|-----------------|------|---------|
| `calc-result-is-expression-value` | A calculation returns, in its result parameter, the value of evaluating its result expression. | §7.19.1; KerML §7.4.8.3 *"result of the result expression is implicitly bound to the result parameter"* (GOSPEL) | `calc_result_is_the_evaluated_expression` | **CONFORMS** |
| `invocation-binds-arguments-to-input-params` | Input parameters bind to their corresponding argument values. | KerML §7.4.9.1 *"edges of the tree are binding connectors between the input parameters … and the results of its argument expressions"* (GOSPEL) | `calc_inputs_bind_to_arguments` | **CONFORMS** |
| `calc-default-param-applied-when-arg-absent` | A declared parameter's default value is used when no argument is supplied. | KerML feature default / FeatureValue (LIBRARY) | `calc_uses_default_when_argument_omitted` | **CONFORMS** |
| `calc-always-has-result-parameter` | A calculation always has a result parameter (inherited if not owned); evaluation always yields a result. | §7.19.2 *"always has a result parameter, inherited if not owned."* (GOSPEL) | `calc_always_produces_a_result_value` | **CONFORMS** |
| `calc-invocation-of-another-calc` | A calculation may invoke another calculation; the callee's result flows into the caller. | KerML §7.4.9.1 (GOSPEL) | _(runtime unit test `calculations::tests::test_calc_nested_call`)_ | **CONFORMS (covered in runtime crate)** — not duplicated here. |
| `calc-usage-is-expression` | A CalculationUsage is an ActionUsage that is also a KerML Expression, typed by a Function. | §8.3.19.3 (GOSPEL, STRUCTURAL) | — | **STRUCTURAL** — type-hierarchy; validation sweep. |
| `result-expression-result-binding-connector` | A result expression's result is bound to the calc's result parameter via a BindingConnector. | KerML §8.4.4.8.1 `checkFunctionResultBindingConnector` (GOSPEL, STRUCTURAL) | — | **STRUCTURAL** — model well-formedness; the *behavioral* consequence is gated by `calc-result-is-expression-value`. |
| `result-parameter-via-return-membership` | The result parameter is owned via a ReturnParameterMembership with direction `out`. | KerML §8.3.4.7.8 (GOSPEL, STRUCTURAL) | — | **STRUCTURAL** — validation sweep. |
| `calc-result-redefines-supertype-result` | A calc's result parameter must redefine the result of every Function it specializes. | §8.4.15.1 `checkFeatureResultRedefinition` (GOSPEL, STRUCTURAL) | — | **STRUCTURAL** — validation sweep. |
| `calc-def-must-specialize-Calculation` | Every CalculationDefinition must specialize `Calculations::Calculation`. | §8.3.19.2 `checkCalculationDefinitionSpecialization` (GOSPEL, STRUCTURAL) | `implicit_generalization.rs` | **SATISFIED-BY-CONSTRUCTION (`2026-06-21`)** — not a validator obligation; `elaborate::implicit_generalization` derives the base specialization (mapping :102), present by construction after elaboration-with-library. Gated in sysml-core. (Steward-ruled; the mis-framed no-library structural gate was removed.) |
| `calc-usage-not-model-level-evaluable` | A CalculationUsage is not model-level evaluable; evaluation is instance-level. | §8.3.19.3 *"A CalculationUsage is not model-level evaluable."* (GOSPEL) | — | **SPEC-DEFINED flag** — same family as the constraints-area `…not-model-level-evaluable`; no runtime behavior to gate beyond instance-level eval (above). |
| `invocation-non-function-behavior-returns-self` | An invocation of a non-Function Behavior evaluates to the performance itself. | KerML §8.4.4.9.5 (GOSPEL) | — | **DOCUMENTED** — edge case; not exercised by the calc evaluator path (calcs are Functions). Low priority. |
| `model-level-evaluable-invocation-requires-library-function` | An invocation is model-level evaluable iff all args are and it invokes a model-level-evaluable library function. | KerML §7.4.9.1 (GOSPEL) | — | **DOCUMENTED** — concerns the metadata/model-level-eval path, distinct from runtime calc evaluation. |

## Coverage

- Gated / behaviorally-gateable (evaluation obligations) = **5 / 5 = 100%**
  (the 4 CONFORMS gated here + nested-call covered in the runtime crate).
- All obligations (incl. 5 STRUCTURAL + 1 spec-flag + 2 documented): **5 / 13 ≈ 38%** —
  the remainder are structural well-formedness rules owned by a future
  parser/validation conformance sweep, not the runtime evaluator.

## Ranked findings

1. **No runtime evaluation gap** — every behavioral calc obligation CONFORMS.
2. **Structural calc well-formedness is unowned** — the specialization /
   result-binding-connector / return-membership rules (6 obligations) have no
   conformance gate anywhere yet. They belong to a *structural/validation*
   conformance sweep (a distinct area from this runtime sweep). Flag for the
   director: decide whether structural well-formedness gets its own sweep.

## Reproducing the citations

```bash
SYS="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/SysML-spec-r2025-04_REF.html"
KER="references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_spec_sources/KerML-spec-r2025-04_REF.html"
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$SYS" > /tmp/sys.txt
python3 -c "import re,sys; print(re.sub(r'<[^>]+>',' ',open(sys.argv[1],encoding='utf-8',errors='replace').read()))" "$KER" > /tmp/ker.txt
```

| Obligation(s) | Source | grep term |
|---|---|---|
| `calc-result-is-expression-value` | `$KER` §7.4.8.3 | `grep -n -i "result of the result expression is implicitly bound" /tmp/ker.txt` |
| `invocation-binds-arguments-to-input-params` | `$KER` §7.4.9.1 | `grep -n -i "edges of the tree are binding connectors" /tmp/ker.txt` |
| `calc-always-has-result-parameter` | `$SYS` §7.19.2 | `grep -n -i "always has a result parameter, inherited if not owned" /tmp/sys.txt` |
| `calc-usage-not-model-level-evaluable` | `$SYS` §8.3.19.3 | `grep -n -i "CalculationUsage is not model-level evaluable" /tmp/sys.txt` |
| `calc-def-must-specialize-Calculation` | `$SYS` §8.3.19.2 | `grep -n -i "checkCalculationDefinitionSpecialization" /tmp/sys.txt` |

## Completeness audit — clauses reviewed (2026-06-21)

### Sections reviewed

| Doc | Clauses reviewed |
|-----|-----------------|
| SysML spec | §7.19.1 (overview), §7.19.2 (defs & usages), §8.3.19.2 (CalculationDefinition abstract syntax), §8.3.19.3 (CalculationUsage abstract syntax), §8.4.15.1 (Calculation Definitions semantics), §8.4.15.2 (Calculation Usages semantics) |
| KerML spec | §7.4.8.1–5 (Functions/Expressions overview & declaration), §7.4.9.1–5 (Expressions overview, operators, primary, base, literal), §8.3.4.7.3 (Expression abstract syntax incl. `evaluate()` op), §8.3.4.7.4 (Function), §8.3.4.7.7 (ResultExpressionMembership), §8.3.4.7.8 (ReturnParameterMembership), §8.3.4.8.8 (InvocationExpression incl. `evaluate()` op), §8.4.4.8.1–2 (Functions/Expressions semantics), §8.4.4.9.1–7 (Expressions semantics incl. InvocationExpression §8.4.4.9.5) |

### Classification table

| Normative unit | Clause | Classification | ID or reason |
|----------------|--------|----------------|--------------|
| result expression bound to result param (prose) | SysML §7.19.2, KerML §7.4.8.2 | CAPTURED | `calc-result-is-expression-value` |
| calc always has result parameter | SysML §7.19.2 | CAPTURED | `calc-always-has-result-parameter` |
| invocation edge = BindingConnector (args→inputs) | KerML §7.4.9.1 | CAPTURED | `invocation-binds-arguments-to-input-params` |
| nested calc invocation result flows to caller | KerML §7.4.9.1 | CAPTURED | `calc-invocation-of-another-calc` |
| default param applied when arg absent | KerML §8.3.4.8.8, feature default | CAPTURED | `calc-default-param-applied-when-arg-absent` |
| CalculationUsage is not model-level evaluable | SysML §8.3.19.3 | CAPTURED | `calc-usage-not-model-level-evaluable` |
| CalculationUsage typed by Function (type-hierarchy) | SysML §8.3.19.3 | STRUCTURAL | type-hierarchy; validation sweep |
| `checkCalculationDefinitionSpecialization` | SysML §8.3.19.2 | CAPTURED (STRUCTURAL) | `calc-def-must-specialize-Calculation` |
| `checkCalculationUsageSpecialization` | SysML §8.3.19.3 | STRUCTURAL | specialization chain; validation sweep |
| `checkCalculationUsageSubcalculationSpecialization` | SysML §8.3.19.3, §8.4.15.2 | STRUCTURAL | composite subcalc must specialize `subcalculations`; no runtime eval path affected |
| `checkFunctionResultBindingConnector` / `checkExpressionResultBindingConnector` | KerML §8.3.4.7.4, §8.4.4.8.1, SysML §8.4.15.1–2 | CAPTURED (STRUCTURAL) | `result-expression-result-binding-connector` |
| `checkFeatureResultRedefinition` (result redefines supertype result) | KerML §8.4.4.8.1, SysML §8.4.15.1 | CAPTURED (STRUCTURAL) | `calc-result-redefines-supertype-result` |
| `ReturnParameterMembership` direction `out` | KerML §8.3.4.7.8 | CAPTURED (STRUCTURAL) | `result-parameter-via-return-membership` |
| `Expression.evaluate()` delegation to ResultExpressionMembership | KerML §8.3.4.7.3 | OUT-OF-SCOPE (runtime path) | behavioral consequence already subsumed by `calc-result-is-expression-value`; the delegation chain is an internal abstract-syntax formalism, not a separate observable obligation |
| `InvocationExpression.evaluate()` — apply Function to arg values | KerML §8.3.4.8.8 | CAPTURED | subsumed by `invocation-binds-arguments-to-input-params` + `calc-result-is-expression-value` |
| non-Function Behavior invocation returns performance itself | KerML §8.4.4.9.5 | CAPTURED | `invocation-non-function-behavior-returns-self` (DOCUMENTED) |
| model-level-evaluable iff args evaluable + library fn | KerML §7.4.9.1, §8.3.4.8.8 | CAPTURED | `model-level-evaluable-invocation-requires-library-function` (DOCUMENTED) |
| operator expressions map to library function invocations | KerML §7.4.9.2, §8.4.4.9.6 | OUT-OF-SCOPE | concerns the expression-language operator-to-function mapping; owned by the expression-evaluation path, not the calc evaluator specifically |
| primary/base expression forms (index, sequence, collect, select, etc.) | KerML §7.4.9.3–4 | OUT-OF-SCOPE | expression sub-language features; not obligations on CalculationDefinition/Usage evaluation |
| literal expressions produce typed DataValue result | KerML §7.4.9.5, §8.4.4.9.2 | OUT-OF-SCOPE | expression-language leaf semantics; owned by expression evaluator, not calc-specific |
| pure calculation properties (no side effects, determinism) | SysML §7.19.1 prose | OUT-OF-SCOPE | purity is a design recommendation ("should"), not a normative runtime obligation; no formal constraint in abstract syntax |

### Honesty

**No MISSED behavioral obligations were found.** The existing matrix's claim of 5/5 (100%) behavioral coverage is correct. The `Expression.evaluate()` and `InvocationExpression.evaluate()` operations in the KerML abstract syntax (§8.3.4.7.3, §8.3.4.8.8) are formalized delegation chains — their observable behavioral consequences are already fully subsumed by the two gated invocation obligations. The two previously DOCUMENTED items (`invocation-non-function-behavior-returns-self`, `model-level-evaluable-invocation-requires-library-function`) remain low-priority edge cases outside the calc evaluator path. All newly enumerated clauses classify as STRUCTURAL or OUT-OF-SCOPE.

**Count summary (this audit): 21 normative units enumerated; 5 CAPTURED-behavioral, 8 CAPTURED-structural/documented, 3 STRUCTURAL (ungated), 5 OUT-OF-SCOPE.**
