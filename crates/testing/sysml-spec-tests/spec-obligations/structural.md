# Obligation matrix — Structural well-formedness (cross-area)

**Area:** graph-integrity / well-formedness obligations that are **not tied to a
single element family** and so have no topical area file. Family-specific
structural rows live in their own trackers (e.g. requirement subject cardinality
in `requirements.md`, send payload in `actions.md`, case objective cardinality in
`verification-analysis-cases.md`, state subaction cardinality in
`state-machines.md`); this file holds only the cross-cutting graph-integrity
obligations (the `E0xx` structural-integrity series).

**Gate:** `tests/structural_spec_conformance.rs` — the same validator stack the
IDE runs (`ModelGraph::validate_structure` + `validate_relationship_types` +
`validate_semantic`). This is THE anchor that proves the structural surface is
live (not a silent no-op).

**Status:** cross-area graph-integrity obligations, gated.

Coverage legend: **GATED-here** · **STRUCTURAL** (validation) · **UNIMPLEMENTED**
(no validator surface — the absence is the finding).

## Obligation table

| ID | Obligation | Citation (tier) | Coverage |
|----|-----------|-----------------|----------|
| `model-element-must-be-owned` | Every non-root model element is owned by (a member of) a namespace; a bare top-level usage/def that is not a package is an orphan. | KerML §8.2 Namespaces / Memberships — a non-root `Element` has an owning `Namespace` (GOSPEL, STRUCTURAL); graph-integrity rule `E001` (`structural_validation::OrphanElement`) | **GATED-here** — `structural_spec_conformance::{orphan_top_level_definition_is_flagged, definition_inside_package_is_not_orphan}` — **CONFORMS** (a bare top-level `part def` raises E001 "must be inside a namespace"; a package-owned def is clean). |

## Reproducing the citations

The `E001` orphan check is graph-integrity (`ModelGraph::validate_structure`),
not a spec OCL — the citation is to the KerML ownership/membership model
(§8.2 Namespaces), rendered as the tool's structural-integrity `E0xx` series.
The behavioural authority is the gating test above; the spec grounding is the
KerML rule that every element except the root library namespace is a member of
exactly one owning namespace.
