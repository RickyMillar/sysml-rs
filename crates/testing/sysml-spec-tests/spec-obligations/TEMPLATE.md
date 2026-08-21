# Obligation matrix — <Family name>

<!-- ADD-A-FAMILY TEMPLATE (testing-architecture-redesign §3C).
     Copy this file to <family>.md and follow the checklist at the bottom.
     Files named TEMPLATE.md are ignored by the obligation_matrix_consistency
     meta-gate; everything else in this directory is scanned. -->

**Area:** <one-line scope statement>.
**Gate:** `tests/<family>_spec_conformance.rs`.
**Status:** <fan-out area / in progress / DONE+GATED>.

Spec sources: SysML §<clauses> (`SysML-spec-r2025-04_REF.html`),
KerML §<clauses> (`KerML-spec-r2025-04_REF.html`),
`sysml.library/<library file(s)>`. Verified `<date>`.

Current-behavior anchor (read-only): `<crate/module the obligations run against>`.

## Obligation table

| ID | Obligation | Citation (tier) | Gate | Verdict |
|----|-----------|-----------------|------|---------|
| `<family>-<kebab-id>` | <ONE normative sentence.> | §<clause> *"<verified quoted sentence>"* (GOSPEL) | `<test_fn_name>` | **CONFORMS** |
| `<family>-<other-id>` | <sentence> | `<Library.sysml>:<line>` (LIBRARY) | — | **STRUCTURAL** — validation sweep. |

Row rules (from `README.md` "How an obligation is recorded"):
- **ID** — kebab-case, stable forever; the gating test carries a matching
  `// OBL: <id>` line (first token after `OBL:` is the id; annotations may
  follow). The `obligation_matrix_consistency` meta-gate enforces both
  directions.
- **Citation** — quote the highest source that establishes the obligation and
  VERIFY the quote against the spec text before writing it down. Everyday
  lookup: grep `references/sysmlv2/derived/{SysML,KerML}-spec-r2025-04.txt`
  (clause headings are `## <clause> <title>` lines); the HTML stays gospel.
- **Tier** — GOSPEL (spec prose) · LIBRARY (normative model) · STRUCTURAL
  (well-formedness only) · SPEC-SILENT (no normative runtime behavior).
  STRUCTURAL / SPEC-SILENT rows are recorded, never silently dropped; they
  need no runtime gate (README "Coverage metric" counts them honestly).
- **Fixtures are purpose-built spec-faithful snippets, NOT corpus files** —
  corpus survival is a no-regression signal, never conformance proof (root
  source-precedence rule 5 in the tracker README).

## Coverage

- Gated / behaviorally-gateable = **N / M** (excludes STRUCTURAL +
  SPEC-SILENT rows).
- Gated / all obligations = **N / T**.

## Completeness audit — clauses reviewed

<!-- R2 discipline: enumerate every spec subclause examined and classify
     every normative unit so the denominator is provably closed. -->

| Clause | Classification |
|--------|----------------|
| §<clause> <title> | CAPTURED / STRUCTURAL / OUT-OF-SCOPE / MISSED |

## Reproducing the citations

| Obligation | Source | Lookup |
|------------|--------|--------|
| `<family>-<kebab-id>` | `derived/SysML-spec-r2025-04.txt` §<clause> | `grep -n -i "<distinctive quote fragment>" references/sysmlv2/derived/SysML-spec-r2025-04.txt` |

(The derived plaintext is checksum-gated against the pinned spec HTML —
`spec-drop.toml` + the `derived_indexes` gate. To re-derive from the HTML
itself: the tag-strip recipe in
`README.md`.)

---

## Add-a-family checklist

1. **Copy this template** to `spec-obligations/<family>.md`; fill the header
   (area, gate file, sources, behavior anchor).
2. **Derive obligations spec-doc-first** (README "Source precedence"): spec
   document → normative library → TTL → xtext. One row per obligation:
   stable kebab ID, one normative sentence, verified citation, tier.
3. **Close the denominator**: fill the completeness-audit section — every
   subclause of the area classified CAPTURED / STRUCTURAL / OUT-OF-SCOPE;
   a MISSED entry is work, not a footnote.
4. **Create `tests/<family>_spec_conformance.rs`** following the
   `runtime_spec_conformance.rs` convention: pure-runtime, ONE obligation
   per test, purpose-built spec-faithful fixtures, a `// OBL: <id>` line per
   test, a `// VERDICT:` marker per case, and a self-scanning
   `*_matrix_summary` test printing verdict counts.
5. **Encode gaps honestly**: a divergence = an `#[ignore]`d test asserting
   the SPEC-CORRECT expectation (plain `cargo test` green + shows pending;
   `-- --ignored` fails, proving the gap is real; closing it = delete the
   `#[ignore]`). Never assert wrong behavior as correct.
6. **Register the family**: add a row to the Status table in `README.md`
   and append a ledger row (append-only) recording the increment.
7. **Run the consistency gates**:
   `cargo test -p sysml-spec-tests --test obligation_matrix_consistency`
   (marker↔matrix) and the new family gate. Both green before commit.
