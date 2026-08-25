---
title: Diagnostics reference
description: Every registered core diagnostic code, generated from the sysml-core error-code registry, plus the runtime health families.
scope:
  - sysml-rs implementation
status: pre-alpha
last_verified_against: 11bd751
source_of_truth:
  - website/src/generated/diagnostics-core.json
  - crates/lang/sysml-core/src/error_codes.rs
known_limitations: /sysml-rs/reference/known-limitations/
---

<!--
GENERATED — do not edit.
Regenerate (with the artifact it renders, src/generated/diagnostics-core.json) via:
  cd website && node scripts/generate-reference.mjs
-->

This page lists every diagnostic code registered in the sysml-core error-code registry (79 codes), generated directly from that registry. These codes appear in editor diagnostics, `sysml check`/`sysml inspect` output, and API diagnostic responses. For the conceptual framing of the code families, see the Book's [Appendix F — diagnostic codes](/sysml-rs/learn/appendix-f-diagnostic-codes.html).

**Scope**: the per-code tables below cover the core registry only. Runtime health codes are a separate surface, documented at family level [further down](/sysml-rs/reference/diagnostics/#runtime-health-families) — they are produced during execution, not by static analysis, and are not part of this registry.

## Structural (8 codes)

Structural integrity of the semantic graph (E-series): ownership, memberships, and relationship endpoints.

| Code | Meaning |
|---|---|
| `E001` | orphan element without an owner |
| `E002` | ownership cycle detected |
| `E003` | dangling membership reference |
| `E004` | relationship source type mismatch |
| `E005` | relationship target type mismatch |
| `E006` | dangling relationship reference |
| `E007` | dangling owning membership reference |
| `E008` | invalid owning membership type |

## Resolution (4 codes)

Name resolution and import health (E2xx and IM-series).

| Code | Meaning |
|---|---|
| `E200` | unresolved name reference |
| `E201` | ambiguous reference (requires qualification) |
| `IM010` | name not in local scope but defined elsewhere — add import or qualify |
| `IM012` | file opened in strict single-file mode; cross-file imports cannot resolve |

## Semantic (62 codes)

Semantic validation (S-series) plus the specialised semantic families: physics (PH), flows (FL), variability (VR), runtime semantic core (RS), and quantities/dimensions (UQ).

| Code | Meaning |
|---|---|
| `S001` | duplicate member name in namespace |
| `S005` | same top-level package name declared in multiple files (workspace) |
| `S015` | invalid typing for usage element |
| `S030` | invalid specialization across type boundaries |
| `S041` | ReturnParameterMembership in non-function/expression context |
| `S042` | membership in wrong ownership context |
| `S043` | SubjectMembership in non-requirement/case context |
| `S044` | ObjectiveMembership in non-case context |
| `S045` | ActorMembership in non-requirement/case context |
| `S046` | StakeholderMembership in non-requirement context |
| `S047` | RequirementConstraintMembership in non-requirement context |
| `S048` | ViewRenderingMembership in non-view context |
| `S051` | ResultExpressionMembership in non-function/expression context |
| `S060` | member cardinality violation |
| `S066` | function has more than one ReturnParameterMembership |
| `S067` | expression has more than one ReturnParameterMembership |
| `S068` | state definition has duplicate subaction |
| `S069` | state usage has duplicate subaction |
| `S070` | ViewDefinition has more than one ViewRenderingMembership |
| `S071` | ViewUsage has more than one ViewRenderingMembership |
| `S130` | RequirementDefinition constraint not composite |
| `S131` | RequirementUsage constraint not composite |
| `S132` | SatisfyRequirementUsage typed by more than one requirement |
| `S133` | ConcernDefinition has more than one SubjectMembership |
| `S134` | ConcernUsage has more than one SubjectMembership |
| `S135` | VerificationCaseDefinition has more than one SubjectMembership |
| `S136` | VerificationCaseUsage has more than one SubjectMembership |
| `S137` | AnalysisCaseDefinition has more than one SubjectMembership |
| `S138` | AnalysisCaseUsage has more than one SubjectMembership |
| `S139` | UseCaseDefinition has more than one SubjectMembership |
| `S140` | UseCaseUsage has more than one SubjectMembership |
| `S106` | connection owned by package instead of type |
| `S107` | interface owned by package instead of type |
| `S108` | flow owned by package instead of type |
| `S090` | AttributeUsage must not be composite |
| `S091` | AttributeDefinition must not be composite |
| `PH001` | domain mismatch on flow connection |
| `PH002` | conservation imbalance — all ports same direction |
| `PH003` | incomplete physics port — missing effort or flow feature |
| `PH004` | direction conflict on flow connection |
| `PH005` | R/C/I element detected but not wired with constraint |
| `PH006` | Real-typed attribute could use ISQ type for physics features |
| `FL010` | port type mismatch on flow connection |
| `FL011` | target port expects a feature the source does not provide |
| `FL012` | conjugation incompatibility on flow connection |
| `FL013` | unconnected output port / open terminal |
| `FL014` | direction conflict on flow connection |
| `FL015` | port multiplicity detected (informational) |
| `FL016` | structural payload incompatibility on flow connection |
| `FL017` | link class unresolved — routing as message channel |
| `FL018` | transfer between ports not connected by any declared interface or connection (Ports.sysml interfacingPorts constraint) |
| `FL019` | transfer direction violation — pick-up at an in-direction port or drop-off into an out-direction port (post-conjugation) |
| `FL020` | payload type does not conform to the flow's source-output / target-input typing (Transfers.kerml payload subsetting) |
| `VR001` | assignment to configuration attribute (defaulted part attribute) at runtime |
| `RS001` | multiple runtime writers — two executors claim the same runtime variable slot |
| `RS002` | unknown override target — session override names neither a runtime slot alias nor an existing context variable |
| `RS003` | unresolved runtime name — expression reference resolves to neither a runtime slot nor a model feature (hard compile error since RSC-2.5) |
| `RS014` | time-accurate zero-crossing re-step failed hard — a located crossing could not be re-stepped without silently corrupting the run: the per-tick crossing bound was exceeded, a sub-interval integration did not perform (non-RK45 solver on the re-step path is a Wave-2b deferral), the target state machine is not slot-attached (its mode/drive writeback would be dropped — the L44 raw add_state_machine shape), or a non-crossing due event raced the crossing for the same SM in the same tick (FIFO ordering unresolved) |
| `UQ001` | quantity dimension mismatch at a binding connector — endpoints carry incompatible ISQ dimensions (error), or a dimensioned endpoint is bound to an untyped attribute (warning) |
| `UQ002` | quantity dimension mismatch across a signal link — the source and target port-feature slots carry incompatible ISQ dimensions, so the boundary has no meaningful conversion (same-dimension scale differences are converted, not flagged) |
| `UQ003` | cross-dimension comparison in a constraint expression — an ordering comparison (&lt;, &lt;=, &gt;, &gt;=) between operands with incompatible ISQ dimensions (the static twin of the RSC-5.1b eval-time error) |
| `UQ004` | dimensioned argument to a dimensionless-only function — a transcendental (sin, cos, exp, ln, …) requires a pure-number argument |

## Validation (5 codes)

Property-level validation (V-series).

| Code | Meaning |
|---|---|
| `V001` | missing required property |
| `V002` | property has wrong type |
| `V003` | property requires at least one value |
| `V004` | property allows at most one value |
| `V005` | read-only property modified |

## Runtime health families

Execution surfaces (simulation sessions, verification runs, flow simulation) report **runtime health codes** in these families. They are emitted by the runtime, carry run-specific context, and are intentionally *not* part of the static registry above, so they are documented here at family level only — the authoritative per-code source is the runtime output itself.

| Family | Covers |
|---|---|
| `AX` | Action execution health |
| `SM` | State-machine execution health |
| `FL` | Flow / transfer execution health |
| `VC` | Verification-case execution health |
| `CN` | Constraint evaluation health |
| `RQ` | Requirement evaluation health |
| `PH` | Physics / hybrid-simulation execution health |

Note that `PH` appears in both worlds: the static registry has `PH001`–`PH006` physics lints (listed above under Semantic), while runtime physics health codes in the `PH` family are separate. Known gaps in diagnostic coverage are tracked in [Known limitations](/sysml-rs/reference/known-limitations/).

## How this page is generated

This page and its data artifact were generated by `node scripts/generate-reference.mjs` (run from `website/`) at sysml-rs commit `11bd751` on 2026-08-25. Input: `cargo run --release -p spec-index -- diagnostics-registry --json` (the `sysml-core::error_codes` registry).
Do not edit the page by hand — regenerate it. `npm run gen-check` reports drift between the committed artifacts and a fresh generation.
