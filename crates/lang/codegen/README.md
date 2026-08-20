# sysml-codegen

Build-time code generation. Parses the SysML v2 / KerML specification files (TTL, XMI, JSON Schema, Xtext, TOML) and emits Rust source that is compiled into `sysml-core` and `sysml-service`. Spec drift fails the build.

`Layer 0 · lang · Foundations` · `build-time code generation` · `crate-type: rlib` · `build-dependency only` · `+1 bin: emit-ts-classification`

## What it owns

The OMG SysML v2 / KerML metamodel is large and authoritative; hand-maintaining the Rust mirror of it would rot instantly. `sysml-codegen` is the single tool that turns the checked-in spec files under `references/sysmlv2/` into Rust source at build time. It is a **pure library of `String`-emitters and parsers** — it performs no file I/O of its own. All reading, env-var gating, and writing live in the consumers' `build.rs` files. Its invariant: *the generated Rust must stay in lock-step with the spec, and if coverage regresses the build must fail.*

**182.**

`ElementKind` variants — unique merged KerML + SysML types (`ElementKind::count()`).

**175.**

typed property-accessor structs generated from OSLC shapes (types that carry shape properties).

**7.**

value enums emitted: `FeatureDirectionKind`, `PortionKind`, `RequirementConstraintKind`, `StateSubactionKind`, `TransitionFeatureKind`, `TriggerKind`, `VisibilityKind`.

>  **How the type counts relate.** KerML contributes ~84 vocabulary types and SysML ~182; the two vocabularies *overlap heavily* (SysML re-declares many KerML types). After `merge_enum_info` / hierarchy merge dedupes by name, the canonical `ElementKind` enum has **182** unique variants — this is the one number that is actually compiled. Of those, **175** have OSLC shape properties and so get a generated accessor struct. There is no longer a "266" figure (the old README's count was stale); `ElementKind::count()` returns `182`.

## Where it sits

```text
spec files TTL vocab/shapes XMI metamodel JSON Schema *Kind Xtext grammars *.toml rules
▼ parsed by
Layer 0 sysml-codegen parsers → validators → generators
▼ String output written by consumer build.rs
consumers sysml-core/build.rs → 5×`.generated.rs` sysml-service/build.rs → classification
▼ `include!()`
compiled sysml-core (ElementKind, accessors, validation) sysml-service (archetype query)
```

`sysml-codegen` is a **build-dependency**, never a runtime dependency. Its parsing deps (`regex`, `quick-xml`, `toml`, `serde_json`) therefore never reach downstream binaries. Workspace build-dependency consumers: `sysml-core`, `sysml-service` (and the helper binary, run by hand). `sysml-parser-incremental` exposes a `codegen` feature that pulls it in for grammar tooling.

## Generated outputs (the deterministic transform)

Each row is one output file, the generator(s) that emit it, and the spec sources that feed it. The first five are written by `sysml-core/build.rs` into `$OUT_DIR` and pulled in via `include!()`; the last by `sysml-service/build.rs`.

| Output | Generator(s) | Fed by | Consumer build.rs |
|---|---|---|---|
| `element_kind.generated.rs` | `generate_enum`, `generate_hierarchy_methods` | KerML + SysML TTL vocab | sysml-core |
| `enums.generated.rs` | `generate_value_enums` | JSON Schema `*Kind.json` | sysml-core |
| `properties.generated.rs` | `accessor_generator`, `validation_generator`, `relationship_generator` | OSLC shapes + XMI relationship constraints | sysml-core |
| `crossrefs.generated.rs` | `generate_crossref_registry` | Xtext cross-references | sysml-core |
| `semantic_validation.generated.rs` | `generate_semantic_validation` | `semantic_rules.toml` | sysml-core |
| archetype classification (Rust) | `archetype_generator::generate_classification_rust` | `archetype_rules.toml` | sysml-service |
| `element-kind-classification.generated.ts` | `archetype_generator::generate_classification_ts` | `archetype_rules.toml` | bin: `emit-ts-classification` (manual) |

## What fails the build

Validators run in the consumer `build.rs` before generation. By design they **panic** on coverage mismatch so a spec regression cannot ship silently.

**Type / enum coverage.**

`validate_type_coverage` + `validate_enum_coverage`: every TTL type must match the XMI metamodel and every JSON-schema enum must be generated. A type in one source but not the other fails the build.

**Relationship coverage.**

`validate_relationship_coverage` + `validate_relationship_property_coverage`: relationship source/target constraints (XMI-authoritative) must be present for every relationship type.

**Cross-reference coverage.**

`validate_crossref_coverage`: every Xtext cross-reference property must map to a scope strategy or be on the `INTENTIONALLY_SKIPPED` list.

**Resolution spec — gated.**

`validate_resolution_spec_with_mappings`: missing registry entries warn by default and **only** panic when the `SYSML_STRICT_VALIDATION` env var is set.

**Property validation.**

`validate_property_validation_coverage`: checks that emitted property-validation methods cover the resolved shape constraints.

**Spec location.**

If the spec dir can't be found the build panics. Resolution order: `SYSML_REFS_DIR` env var → `<repo>/references/sysmlv2` → panic. (There is no `SYSMLV2_REFS_DIR`; that was stale doc folklore.)

## Module map

26 modules in `lib.rs`, grouped by role: parsers read spec files, inheritance resolves the type hierarchy, validators cross-check coverage, generators emit Rust (or TS) source.

| Module | Role | Wired? | Responsibility |
|---|---|---|---|
| `ttl_parser` | parser | yes | Parse TTL vocab → `TypeInfo`/`EnumInfo`; `merge_enum_info` dedupes cross-vocab enums. |
| `shapes_parser` | parser | yes | Parse OSLC shapes → property constraints; resolve shared properties. (char-indexed; lint-suppressed) |
| `xmi_class_parser` | parser | yes | Extract classes from KerML/SysML XMI metamodel. |
| `xmi_relationship_parser` | parser | yes | Extract relationship source/target constraints (XMI-authoritative). |
| `json_schema_parser` | parser | yes | Parse `*Kind.json` schemas → value-enum definitions. |
| `xtext_parser` | parser | partial | Parse Xtext keywords/operators/enums/rules. (line-indexed; lint-suppressed) |
| `xtext_crossref_parser` | parser | yes | Parse Xtext cross-references → `CrossReference` registry input. |
| `semantic_rule_parser` | parser | yes | Parse `semantic_rules.toml` → `SemanticRule` catalog. |
| `inheritance` | inheritance | yes | Build type hierarchy + resolve transitive property inheritance. |
| `hierarchy_generator` | inheritance / generator | yes | Compute transitive closure + emit hierarchy methods (`is_a`, supertypes). |
| `spec_validation` | validator | yes | Type + enum coverage cross-check (TTL ↔ XMI ↔ JSON). |
| `crossref_validation` | validator | yes | Cross-reference coverage; scope-strategy inference; skip list. |
| `grammar_element_validator` | validator | — | Validate grammar element ↔ element-kind linkage. |
| `property_validation_validator` | validator | yes | Coverage of generated property-validation methods. |
| `resolution_spec_validator` | validator | yes (gated) | Resolution-spec completeness vs registry; panics only under `SYSML_STRICT_VALIDATION`. |
| `enum_generator` | generator | yes | Emit the `ElementKind` enum (182 variants) + `iter`/`as_str`/`from_str`/`count`. |
| `enum_value_generator` | generator | yes | Emit the 7 value enums (FeatureDirectionKind, …). |
| `relationship_generator` | generator | yes | Emit relationship methods + property methods with XMI constraints. |
| `accessor_generator` | generator | yes | Emit 175 typed property-accessor structs. |
| `validation_generator` | generator | yes | Emit property-validation methods from resolved constraints. |
| `crossref_generator` | generator | yes | Emit the cross-reference registry. |
| `semantic_validation_generator` | generator | yes | Emit the semantic-validation dispatch from the TOML catalog. |
| `archetype_generator` | generator | yes | Emit element-kind classification — Rust (sysml-service) and TS (FE) — from `archetype_rules.toml`. |
| `pest_generator` | generator | orphaned | Emit Pest keyword/operator/enum rules. Not wired into any `build.rs` (see warning below). |
| `pest_validator` | validator | orphaned | Pest keyword/rule coverage. Not wired into any `build.rs`. |
| `treesitter_generator` | generator | orphaned | Emit tree-sitter-shaped data; not consumed by any `build.rs` (referenced only as a parity baseline in sysml-spec-tests). |

> ⚠  **Orphaned codegen.** `pest_generator` / `pest_validator` were written for the Pest PEG parser crate `sysml-parser-batch`, which has been **deleted** (tree-sitter via `sysml-parser-incremental` is now the sole parser). No `build.rs` calls the `generate_pest_*` or `validate_*_coverage` functions anymore, and `treesitter_generator` is likewise not wired into a build. They remain in the public API but guard no live target. Treat them as candidates for removal, not as load-bearing build steps.

## Public API

#### `parse_ttl_vocab(ttl: &str) -> Result<Vec<TypeInfo>, ParseError>`

Parse a TTL vocabulary string into type records. `TypeInfo.supertypes` holds *direct* parents only; transitive closure is computed later by `inheritance` / `hierarchy_generator`. Companion: `parse_ttl_enums`, `merge_enum_info` (dedupes enums declared in both KerML and SysML).

#### `generate_enum(name: &str, kerml: &[TypeInfo], sysml: &[TypeInfo]) -> String`

Emit the merged element-kind enum. With the real spec inputs this produces `ElementKind` with 182 unique variants plus `iter()`, `as_str()`, `from_str()`, and `const fn count()`.

#### `generate_value_enums(enums: &[EnumInfo]) -> String`

Emit the 7 value enums (e.g. `FeatureDirectionKind` with `in/out/inout`) from JSON-schema-derived definitions.

#### `generate_hierarchy_methods(kerml, sysml) -> String · TypeHierarchy`

Compute the transitive type hierarchy and emit query methods. `TypeHierarchy` is the in-memory hierarchy structure.

#### `generate_relationship_methods_with_xmi(...) · generate_relationship_property_methods(...)`

Emit relationship source/target methods using XMI-authoritative constraints (`RelationshipConstraint`, `RelationshipTargetProperty`). Coverage checked by `validate_relationship_coverage` / `validate_relationship_property_coverage`.

#### `accessor_generator::generate_property_accessors(resolved) · validation_generator::generate_validation_methods(resolved)`

From resolved (inheritance-flattened) shape properties, emit 175 typed accessor structs and their property-validation methods. Coverage gate: `validate_property_validation_coverage`.

#### `generate_crossref_registry(refs) -> String · validate_crossref_coverage(...)`

Emit the cross-reference scope registry from parsed Xtext cross-references; validate that every reference maps to a `ScopeStrategy` or is in `INTENTIONALLY_SKIPPED`.

#### `parse_semantic_rules_file(path) -> ... · generate_semantic_validation(rules) -> String`

Parse `semantic_rules.toml` into a `SemanticRule` catalog and emit the semantic-validation dispatch. Helpers: `group_rules_by_element_type`, `unique_check_functions`, `summarize_rules`.

#### `archetype_generator::generate_classification_rust(path) · generate_classification_ts(path) -> Result<String, ArchetypeGenError>`

From `archetype_rules.toml`, emit element-kind classification. The Rust form is generated on every build by `sysml-service/build.rs`; the TS form (`element-kind-classification.generated.ts`, checked into the FE) is emitted on demand by the `emit-ts-classification` binary so the two outputs stay tied to one rules file.

## Usage

Consumer pattern (this is roughly what `sysml-core/build.rs` does — the crate emits strings, the caller writes them):

```
// sysml-core/build.rs
use std::{env, fs, path::Path};

let out_dir = env::var("OUT_DIR").unwrap();

let kerml_types = sysml_codegen::parse_ttl_vocab(&kerml_vocab)?;
let sysml_types = sysml_codegen::parse_ttl_vocab(&sysml_vocab)?;

// Fail the build if TTL and XMI disagree on type coverage.
let report = sysml_codegen::validate_type_coverage(&ttl_type_names, &xmi_classes);
assert!(report.is_valid(), "spec type coverage regressed");

let enum_code = sysml_codegen::generate_enum("ElementKind", &kerml_types, &sysml_types);
fs::write(Path::new(&out_dir).join("element_kind.generated.rs"), enum_code)?;
// ...then enums / properties / crossrefs / semantic_validation, same shape.
```

Regenerate the front-end classification file after editing the rules:

```
cargo run -p sysml-codegen --bin emit-ts-classification
# writes editors/simulation-app/src/types/element-kind-classification.generated.ts
```

## Build & test

```
# Unit tests for parsers and generators
cargo test -p sysml-codegen

# Run the full pipeline as a side effect of building the consumer
cargo build -p sysml-core

# Inspect the emitted Rust (not committed — lives in OUT_DIR)
ls target/debug/build/sysml-core-*/out/*.generated.rs
```

## Invariants & pitfalls

- **Generators emit `String`; they never write files.** All I/O, env-var gating, and output ordering live in the consumer `build.rs` — so the real pipeline contract is in `sysml-core/build.rs` and `sysml-service/build.rs`, not here.

- **Validators panic by design.** A spec/XMI/JSON mismatch fails the build. The one softened gate is the resolution spec, behind `SYSML_STRICT_VALIDATION`.

- **Spec dir resolution:** `SYSML_REFS_DIR` → `<repo>/references/sysmlv2` → panic. There is no in-repo `spec/` directory and no `SYSMLV2_REFS_DIR` var.

- **XMI is authoritative** for relationship source/target constraints; **TTL is authoritative** for the type hierarchy; JSON Schema drives the value enums.

- **Adding a parser/generator module** means adding the `pub mod` line *and* the `pub use` re-exports in `lib.rs` — the crate re-exports heavily.

- **TTL/shape parsing is format-sensitive.** `shapes_parser` and `xtext_parser` are hand-rolled char/line scanners carrying `#[allow(clippy::indexing_slicing)]` with manual bounds guards — test against the real spec files, not synthetic snippets.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
