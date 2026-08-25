---
title: Language capability matrix
description: Measured per-concept support of the SysML v2 / KerML language surface, aggregated from the language pack.
scope:
  - sysml-rs implementation
  - Experimental / partial support
status: pre-alpha
last_verified_against: 11bd751
source_of_truth:
  - website/src/generated/capability-matrix.json
  - website/.learn-src/src/language-pack/
known_limitations: /sysml-rs/reference/known-limitations/
---

<!--
GENERATED — do not edit.
Regenerate (with the artifact it renders, src/generated/capability-matrix.json) via:
  cd website && node scripts/generate-reference.mjs
-->

This matrix aggregates the [language pack](/sysml-rs/learn/language-pack/) — one card per language concept, each carrying evidence-gated support statements for the pipeline stages `parse`, `lower`, `resolve`, `elaborate`, `validate`, `execute`, `format`, `lsp`. A stage is **validated** only when a purpose-built fixture proves it; everything else is reported as **unknown**, never assumed. Absence of a "validated" mark is absence of evidence, not necessarily absence of support.

Measured from the language pack shipped in the pinned Book checkout (Book pin `e243806`), pack generator version 1, built against OMG spec drop 2025-04 / metamodel drop 20250201.

## Totals

327 concept cards; 138 have at least one validated stage.

| Stage | Cards validated |
|---|---|
| `parse` | 138 |
| `lower` | 138 |
| `resolve` | 101 |
| `elaborate` | 0 |
| `validate` | 8 |
| `execute` | 0 |
| `format` | 0 |
| `lsp` | 0 |

## Per-category support

A concept card can belong to more than one category, so category counts overlap. Evidence kinds refer to the pack's committed example fixtures.

### behavior (51 concepts)

Validated-stage counts: `parse` 29 · `lower` 29 · `resolve` 19 · `elaborate` 0 · `validate` 2 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Behavior | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Function | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Interaction | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Invariant | KerML | `parse` `lower` | positive + negative fixtures | — |
| Predicate | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Calc Result Is Expression Value | KerML | — | none | — |
| Clock Timeflow Constraint | KerML | — | none | — |
| Durationof End Minus Start | KerML | — | none | — |
| Happens Just Before No Intervening | KerML | — | none | — |
| Invocation Binds Arguments To Input Params | KerML | — | none | — |
| Local Clock Defaults To Universal Clock | KerML | — | none | — |
| Occurrence Has Lifetime Extent | KerML | — | none | — |
| Timeof Continuity Constraint | KerML | — | none | — |
| Timeof Ordering Constraint | KerML | — | none | — |
| Accept Action | SysML | `parse` `lower` | positive + negative fixtures | — |
| Action Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Action Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Assignment Action | SysML | `parse` `lower` | positive + negative fixtures | — |
| Calculation Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Calculation Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Decision Node | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Exhibit State Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| For Loop Action | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Fork Node | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Join Node | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Merge Node | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Message | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Message Event | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Perform Action | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Send Action | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| State Action Usage | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| State Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| State Usage | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Succession | SysML | `parse` `lower` | positive + negative fixtures | — |
| Succession Flow Usage | SysML | `parse` `lower` | positive + negative fixtures | — |
| Terminate Node | SysML | `parse` `lower` | positive + negative fixtures | — |
| Transition Usage | SysML | `parse` `lower` | positive + negative fixtures | — |
| While Loop Action | SysML | `parse` `lower` | positive + negative fixtures | — |
| Calc Always Has Result Parameter | SysML | — | none | — |
| Decision Node Exactly One Outgoing | SysML | — | none | — |
| For Loop Iterates Over Sequence | SysML | — | none | — |
| Fork Node Concurrent Fanout | SysML | — | none | — |
| If Action Evaluates Test Then Branch | SysML | — | none | — |
| Interpolate Returns Null Out Of Bounds | SysML | — | none | — |
| Join Node Synchronize All Incoming | SysML | — | none | — |
| Merge Node Any One Incoming | SysML | — | none | — |
| Mref Dimension Must Match Attribute | SysML | — | none | — |
| Quantity Arithmetic Dimension Rules | SysML | — | none | — |
| Sampled Function Must Be Monotonic | SysML | — | none | — |
| Send Action Initiates Message Transfer | SysML | — | none | — |
| While Loop Iterates While Test | SysML | — | none | — |

### cases (15 concepts)

Validated-stage counts: `parse` 11 · `lower` 11 · `resolve` 10 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Analysis Case | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Analysis Case Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Case Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Case Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Include Use Case | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Objective | SysML | `parse` `lower` | positive + negative fixtures | — |
| Requirement Verification | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Use Case Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Use Case Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Verification Case | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Verification Case Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Analysis Case Objective Bound To Result | SysML | — | none | — |
| Case Has Subject And Objective | SysML | — | none | — |
| Verdict Criteria Modeled Explicitly | SysML | — | none | — |
| Verdict Semantics | SysML | — | none | — |

### connection (13 concepts)

Validated-stage counts: `parse` 12 · `lower` 12 · `resolve` 11 · `elaborate` 0 · `validate` 3 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Interaction | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Association | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Association Structure | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Binding Connector | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connector | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connection Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connection Usage | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Flow Connection | SysML | `parse` `lower` `resolve` `validate` | positive + negative fixtures | — |
| Interface Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Interface Usage | SysML | `parse` `lower` `resolve` `validate` | positive + negative fixtures | — |
| Port Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Port Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Port Usage Referential | SysML | — | none | — |

### expression (41 concepts)

Validated-stage counts: `parse` 15 · `lower` 15 · `resolve` 7 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Function | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Predicate | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Additive Operator | Expressions | — | none | — |
| And Operator | Expressions | — | none | — |
| Cast Operator | Expressions | — | none | — |
| ClassificationTest Operator | Expressions | — | none | — |
| ConditionalAnd Operator | Expressions | — | none | — |
| Conditional Operator | Expressions | — | none | — |
| ConditionalOr Operator | Expressions | — | none | — |
| Constructor Expression | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Equality Operator | Expressions | — | none | — |
| Exponentiation Operator | Expressions | — | none | — |
| Feature Reference Expression | Expressions | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Feature Value | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Implies Operator | Expressions | — | none | — |
| Invocation Expression | Expressions | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Boolean Literal | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Infinity Literal | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Integer Literal | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Real Literal | Expressions | `parse` `lower` | positive + negative fixtures | — |
| String Literal | Expressions | `parse` `lower` | positive + negative fixtures | — |
| MetaCast Operator | Expressions | — | none | — |
| MetaClassificationTest Operator | Expressions | — | none | — |
| Metadata Access Expression | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Multiplicative Operator | Expressions | — | none | — |
| NullCoalescing Operator | Expressions | — | none | — |
| Null Expression | Expressions | `parse` `lower` | positive + negative fixtures | — |
| Or Operator | Expressions | — | none | — |
| Relational Operator | Expressions | — | none | — |
| Unary Operator | Expressions | — | none | — |
| Xor Operator | Expressions | — | none | — |
| Constraint Result Boolean | KerML | — | none | — |
| Core Operator Semantics | KerML | — | none | — |
| Feature Ref Resolves To Bound Value | KerML | — | none | — |
| Unbound Feature Yields Inconclusive Not False | KerML | — | none | — |
| Calculation Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Calculation Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Assert Constraint Must Be True | SysML | — | none | — |
| Constraint Satisfied Iff True | SysML | — | none | — |
| Constraint Usage Discovered | SysML | — | none | — |
| Negated Assert Must Be False | SysML | — | none | — |

### implementation (10 concepts)

Validated-stage counts: `parse` 0 · `lower` 0 · `resolve` 0 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| constructor expression does not lower to a distinct kind | Tooling | — | none | 1 |
| if / terminate action nodes do not lower to distinct node kinds | Tooling | — | none | 1 |
| enum members do not lower to a distinct EnumerationUsage | Tooling | — | none | 1 |
| message / message-event do not lower to distinct kinds | Tooling | — | none | 1 |
| individual / portion occurrence prefixes do not lower to distinct kinds | Tooling | — | none | 1 |
| prefixed `individual\|variation &lt;kind&gt; def` and `ref &lt;keyword-usage&gt;` forms misparse | Tooling | — | none | 1 |
| membership-wrapped role usages under-resolve their type refs nondeterministically | Tooling | — | none | 1 |
| state effect / state action / trigger subactions do not lower to distinct kinds | Tooling | — | none | 1 |
| TextualRepresentation lowers to a generic ReferenceUsage | Tooling | — | none | 1 |
| type-relationship operators do not materialize a distinct relationship kind | Tooling | — | none | 1 |

### library (33 concepts)

Validated-stage counts: `parse` 0 · `lower` 0 · `resolve` 0 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Boolean Evaluation | KerML | — | none | — |
| FlowTransfer | KerML | — | none | — |
| HappensBefore | KerML | — | none | — |
| HappensDuring | KerML | — | none | — |
| head (Sequence Function) | KerML | — | none | — |
| includes (Sequence Function) | KerML | — | none | — |
| isEmpty (Sequence Function) | KerML | — | none | — |
| last (Sequence Function) | KerML | — | none | — |
| MessageTransfer | KerML | — | none | — |
| notEmpty (Sequence Function) | KerML | — | none | — |
| Occurrence | KerML | — | none | — |
| max (Scalar Function) | KerML | — | none | — |
| min (Scalar Function) | KerML | — | none | — |
| size (Sequence Function) | KerML | — | none | — |
| State Performance | KerML | — | none | — |
| tail (Sequence Function) | KerML | — | none | — |
| Transfer | KerML | — | none | — |
| Analysis Case (Library Definition) | SysML | — | none | — |
| Calculation (Library Definition) | SysML | — | none | — |
| Case (Library Definition) | SysML | — | none | — |
| Constraint Check | SysML | — | none | — |
| DurationValue (ISQ Base Quantity) | SysML | — | none | — |
| kilogram (SI Unit) | SysML | — | none | — |
| LengthValue (ISQ Base Quantity) | SysML | — | none | — |
| MassValue (ISQ Base Quantity) | SysML | — | none | — |
| metre (SI Unit) | SysML | — | none | — |
| Pass If | SysML | — | none | — |
| Requirement Check | SysML | — | none | — |
| second (SI Unit) | SysML | — | none | — |
| State Action (Library Definition) | SysML | — | none | — |
| Verdict Kind | SysML | — | none | — |
| Verification Case (Library Definition) | SysML | — | none | — |
| Verification Method Kind | SysML | — | none | — |

### metadata (6 concepts)

Validated-stage counts: `parse` 6 · `lower` 6 · `resolve` 3 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Comment | KerML | `parse` `lower` | positive + negative fixtures | — |
| Documentation | KerML | `parse` `lower` | positive + negative fixtures | — |
| Textual Representation | KerML | `parse` `lower` | positive + negative fixtures | — |
| Metaclass | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Metadata Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Metadata Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |

### requirements (18 concepts)

Validated-stage counts: `parse` 13 · `lower` 13 · `resolve` 10 · `elaborate` 0 · `validate` 1 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Actor | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Assert Constraint | SysML | `parse` `lower` | positive + negative fixtures | — |
| Concern Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Concern Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Constraint Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Constraint Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Framed Concern | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Requirement Constraint (assume / require) | SysML | `parse` `lower` | positive + negative fixtures | — |
| Requirement Definition | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Requirement Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Requirement Satisfaction | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Stakeholder | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Subject | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Negated Satisfy Requires Not Satisfied | SysML | — | none | — |
| Requirement Check Result Is Boolean | SysML | — | none | — |
| Requirement Is Constraint Satisfied Iff True | SysML | — | none | — |
| Requirement Result Is Assumption Implies Required | SysML | — | none | — |
| Requirement Subject Must Be First Parameter | SysML | — | none | — |

### state-machine (11 concepts)

Validated-stage counts: `parse` 3 · `lower` 3 · `resolve` 1 · `elaborate` 0 · `validate` 1 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| At Most One Transition Fires Per Trigger | KerML | — | none | — |
| State Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| State Usage | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Transition Usage | SysML | `parse` `lower` | positive + negative fixtures | — |
| At Most One Each Subaction Kind | SysML | — | none | — |
| Entry Do Exit Ordering | SysML | — | none | — |
| Initial State Via Entry Succession | SysML | — | none | — |
| No Transition On Unmatched Event | SysML | — | none | — |
| Transition Firing Order Exit Effect Entry | SysML | — | none | — |
| Transition Guard Boolean | SysML | — | none | — |
| Transition Selection | SysML | — | none | — |

### structure (65 concepts)

Validated-stage counts: `parse` 61 · `lower` 61 · `resolve` 51 · `elaborate` 0 · `validate` 5 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Association | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Association Structure | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Binding Connector | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Class | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Classifier | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Classifier Conjugation | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Conjugation | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connector | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Data Type | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Differencing | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Disjoining | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| EndFeatureMembership | KerML | — | none | — |
| Feature Conjugation | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Feature Inverting | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| FeatureMembership | KerML | — | none | — |
| Feature Typing | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Filter Package | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Import | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Intersecting | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Library Package | KerML | `parse` `lower` | positive + negative fixtures | — |
| Membership | KerML | — | none | — |
| Membership Import | KerML | `parse` `lower` | positive + negative fixtures | — |
| Metaclass | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Multiplicity | KerML | `parse` `lower` | positive + negative fixtures | — |
| Namespace | KerML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Namespace Import | KerML | `parse` `lower` | positive + negative fixtures | — |
| OwningMembership | KerML | — | none | — |
| Package | KerML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Redefinition | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Specialization | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Structure | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Subsetting | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Type | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Type Featuring | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Unioning | KerML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Allocation Definition | SysML | `parse` `lower` | positive + negative fixtures | — |
| Allocation Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Attribute Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Attribute Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Binding Connector (as Usage) | SysML | `parse` `lower` | positive + negative fixtures | — |
| Conjugated Port Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connection Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Connection Usage | SysML | `parse` `lower` `validate` | positive + negative fixtures | — |
| Dependency | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Enumerated Value | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Enumeration Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Enumeration Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Event Occurrence Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Feature Typing | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Flow Connection | SysML | `parse` `lower` `resolve` `validate` | positive + negative fixtures | — |
| Flow Definition | SysML | `parse` `lower` | positive + negative fixtures | — |
| Individual Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Individual Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Interface Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Interface Usage | SysML | `parse` `lower` `resolve` `validate` | positive + negative fixtures | — |
| Item Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Item Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Occurrence Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Occurrence Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Part Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Part Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Port Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Port Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Portion Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Reference Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |

### validation (124 concepts)

Validated-stage counts: `parse` 0 · `lower` 0 · `resolve` 0 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| At Most One Return Parameter | KerML | — | none | — |
| At Most One Transition Fires Per Trigger | KerML | — | none | — |
| Behavior Not Specialize Structure | KerML | — | none | — |
| Binary Connector Two Ends | KerML | — | none | — |
| Binding Connector Two Ends | KerML | — | none | — |
| Calc Result Is Expression Value | KerML | — | none | — |
| Class Not Specialize Datatype | KerML | — | none | — |
| Clock Timeflow Constraint | KerML | — | none | — |
| Connector Owned By Type | KerML | — | none | — |
| Constraint Result Boolean | KerML | — | none | — |
| Core Operator Semantics | KerML | — | none | — |
| Datatype Not Specialize Class | KerML | — | none | — |
| Durationof End Minus Start | KerML | — | none | — |
| Feature Ref Resolves To Bound Value | KerML | — | none | — |
| Happens Just Before No Intervening | KerML | — | none | — |
| Invocation Binds Arguments To Input Params | KerML | — | none | — |
| Local Clock Defaults To Universal Clock | KerML | — | none | — |
| Model Element Must Be Owned | KerML | — | none | — |
| No Inherited Name Conflict | KerML | — | none | — |
| No Name Alias Conflict | KerML | — | none | — |
| Occurrence Has Lifetime Extent | KerML | — | none | — |
| Parameter Membership Owning Type | KerML | — | none | — |
| Result Expression In Function Or Expression | KerML | — | none | — |
| Return Parameter Membership Owning Type | KerML | — | none | — |
| Structure Not Specialize Behavior | KerML | — | none | — |
| Succession Two Ends | KerML | — | none | — |
| Timeof Continuity Constraint | KerML | — | none | — |
| Timeof Ordering Constraint | KerML | — | none | — |
| Unbound Feature Yields Inconclusive Not False | KerML | — | none | — |
| Unique Owned Member Names | KerML | — | none | — |
| Accept Action Has Payload | SysML | — | none | — |
| Action Typed By Behavior | SysML | — | none | — |
| Actor Membership In Req Or Case | SysML | — | none | — |
| Allocation Has Ends | SysML | — | none | — |
| Allocation Typed By Allocation Defs | SysML | — | none | — |
| Analysis Case At Most One Subject | SysML | — | none | — |
| Analysis Case Objective Bound To Result | SysML | — | none | — |
| Assert Constraint Must Be True | SysML | — | none | — |
| Assignment Action Has Target | SysML | — | none | — |
| At Most One Each Subaction Kind | SysML | — | none | — |
| At Most One Objective | SysML | — | none | — |
| At Most One State Subaction | SysML | — | none | — |
| At Most One Subject | SysML | — | none | — |
| At Most One View Rendering | SysML | — | none | — |
| Attribute Def Not Specialize Item Def | SysML | — | none | — |
| Attribute Must Not Be Composite | SysML | — | none | — |
| Attribute Typed By Datatypes | SysML | — | none | — |
| Calc Always Has Result Parameter | SysML | — | none | — |
| Calculation Typed By One Calc Def | SysML | — | none | — |
| Case Has Subject And Objective | SysML | — | none | — |
| Case Typed By One Case Def | SysML | — | none | — |
| Concern At Most One Subject | SysML | — | none | — |
| Connection Has Ends | SysML | — | none | — |
| Connection Typed By Association | SysML | — | none | — |
| Constraint Satisfied Iff True | SysML | — | none | — |
| Constraint Typed By Predicate | SysML | — | none | — |
| Constraint Usage Discovered | SysML | — | none | — |
| Decision Node Exactly One Outgoing | SysML | — | none | — |
| Decision Node One Incoming | SysML | — | none | — |
| Entry Do Exit Ordering | SysML | — | none | — |
| Enumeration Typed By One Enum Def | SysML | — | none | — |
| Exhibit State One Type | SysML | — | none | — |
| Flow Typed By Interaction | SysML | — | none | — |
| For Loop Iterates Over Sequence | SysML | — | none | — |
| Fork Node Concurrent Fanout | SysML | — | none | — |
| Fork Node One Incoming | SysML | — | none | — |
| If Action Evaluates Test Then Branch | SysML | — | none | — |
| Initial State Via Entry Succession | SysML | — | none | — |
| Interface Has Ends | SysML | — | none | — |
| Interface Typed By Interface Defs | SysML | — | none | — |
| Interpolate Returns Null Out Of Bounds | SysML | — | none | — |
| Item Def Not Specialize Attribute Def | SysML | — | none | — |
| Item Typed By Item Defs | SysML | — | none | — |
| Join Node One Outgoing | SysML | — | none | — |
| Join Node Synchronize All Incoming | SysML | — | none | — |
| Merge Node Any One Incoming | SysML | — | none | — |
| Merge Node One Outgoing | SysML | — | none | — |
| Mref Dimension Must Match Attribute | SysML | — | none | — |
| Negated Assert Must Be False | SysML | — | none | — |
| Negated Satisfy Requires Not Satisfied | SysML | — | none | — |
| No Transition On Unmatched Event | SysML | — | none | — |
| Objective Membership In Case | SysML | — | none | — |
| Occurrence Typed By Occurrence Defs | SysML | — | none | — |
| Parallel State No Transitions | SysML | — | none | — |
| Part Typed By Part Defs | SysML | — | none | — |
| Perform Action One Type | SysML | — | none | — |
| Port Definition Owned Usages Referential | SysML | — | none | — |
| Port Typed By Port Defs | SysML | — | none | — |
| Port Usage Nested Usages Referential | SysML | — | none | — |
| Port Usage Referential | SysML | — | none | — |
| Quantity Arithmetic Dimension Rules | SysML | — | none | — |
| Requirement Check Result Is Boolean | SysML | — | none | — |
| Requirement Constraint In Requirement | SysML | — | none | — |
| Requirement Constraints Composite | SysML | — | none | — |
| Requirement Is Constraint Satisfied Iff True | SysML | — | none | — |
| Requirement Result Is Assumption Implies Required | SysML | — | none | — |
| Requirement Subject Must Be First Parameter | SysML | — | none | — |
| Requirement Typed By One Req Def | SysML | — | none | — |
| Sampled Function Must Be Monotonic | SysML | — | none | — |
| Satisfy Req One Type | SysML | — | none | — |
| Send Action Has Payload | SysML | — | none | — |
| Send Action Initiates Message Transfer | SysML | — | none | — |
| Stakeholder Membership In Requirement | SysML | — | none | — |
| State Subaction Owned By State | SysML | — | none | — |
| State Typed By State Defs | SysML | — | none | — |
| Subject Is First Parameter | SysML | — | none | — |
| Subject Membership In Req Or Case | SysML | — | none | — |
| Transition Feature In Transition | SysML | — | none | — |
| Transition Firing Order Exit Effect Entry | SysML | — | none | — |
| Transition Guard Boolean | SysML | — | none | — |
| Transition Has Source | SysML | — | none | — |
| Transition Owned By State Or Action | SysML | — | none | — |
| Transition Selection | SysML | — | none | — |
| Usage Typed By Definitions | SysML | — | none | — |
| Use Case At Most One Subject | SysML | — | none | — |
| Variant Membership In Variation | SysML | — | none | — |
| Variation Members Are Variants | SysML | — | none | — |
| Variation Must Be Abstract | SysML | — | none | — |
| Variation No Chain | SysML | — | none | — |
| Verdict Criteria Modeled Explicitly | SysML | — | none | — |
| Verdict Semantics | SysML | — | none | — |
| Verification Case At Most One Subject | SysML | — | none | — |
| View Rendering In View | SysML | — | none | — |
| While Loop Iterates While Test | SysML | — | none | — |

### views (8 concepts)

Validated-stage counts: `parse` 8 · `lower` 8 · `resolve` 6 · `elaborate` 0 · `validate` 0 · `execute` 0 · `format` 0 · `lsp` 0

| Concept | Language | Validated stages | Evidence | Known gaps |
|---|---|---|---|---|
| Membership Expose | SysML | `parse` `lower` | positive + negative fixtures | — |
| Namespace Expose | SysML | `parse` `lower` | positive + negative fixtures | — |
| Rendering Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Rendering Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| View Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| View Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Viewpoint Definition | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |
| Viewpoint Usage | SysML | `parse` `lower` `resolve` | positive + negative fixtures | — |

## How this page is generated

This page and its data artifact were generated by `node scripts/generate-reference.mjs` (run from `website/`) at sysml-rs commit `11bd751` on 2026-08-25. Input: the language-pack cards in the pinned Book checkout `website/.learn-src/src/language-pack/` (Book pin `e243806dc7b47464e1a0ade153ff2bc76616c767`).
Do not edit the page by hand — regenerate it. `npm run gen-check` reports drift between the committed artifacts and a fresh generation.
