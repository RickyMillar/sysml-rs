# Spec-Driven Code Generation (Build-Time)

This project already uses **build-time code generation** driven directly by the SysML/KerML specifications. This is **not** model-to-target-language codegen; it is spec-to-Rust codegen that produces core types, accessors, and validation helpers used by `sysml-core`.

## What Gets Generated

The `sysml-codegen` crate reads specification files and generates Rust source that is compiled into `sysml-core`:

- `ElementKind` enum (all SysML + KerML element types)
- Type hierarchy methods (supertypes, predicates, definition/usage mappings)
- Relationship constraint methods (source/target type constraints)
- Relationship target property mapping (which property holds the target reference)
- Value enums (FeatureDirectionKind, VisibilityKind, etc.)
- Typed property accessors (per-element property getters)
- Per-element validation methods (shape-based constraints)
- Cross-reference registry (from Xtext grammar)

All of this is **derived from the official spec files** and validated for coverage at build time.

---

## Where It Runs

`sysml-core/build.rs` orchestrates the entire pipeline using `sysml-codegen`.

Key entry points:

- `crates/lang/sysml-core/build.rs`
- `crates/lang/codegen/src/*`

The generated files are written to the build output directory and included into `sysml-core/src/lib.rs`:

- `element_kind.generated.rs`
- `enums.generated.rs`
- `properties.generated.rs`
- `crossrefs.generated.rs`

---

## Spec Inputs

`build.rs` reads spec files from `references/sysmlv2/`:

| Purpose | Path |
|---------|------|
| KerML vocab | `references/sysmlv2/Kerml-Vocab.ttl` |
| SysML vocab | `references/sysmlv2/SysML-vocab.ttl` |
| KerML shapes | `references/sysmlv2/KerML-shapes.ttl` |
| SysML shapes | `references/sysmlv2/SysML-shapes.ttl` |
| KerML XMI | `references/sysmlv2/KerML/20250201/KerML.xmi` |
| SysML XMI | `references/sysmlv2/SysML/20250201/SysML.xmi` |
| JSON enums | `references/sysmlv2/SysML-v2-API-Services/conf/json/schema/metamodel/*Kind.json` |

Cross-reference data is pulled from the **Xtext grammars** in the pilot implementation:

- `SysML-v2-Pilot-Implementation/org.omg.kerml.xtext/.../KerML.xtext`
- `SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/.../SysML.xtext`

If these Xtext files are missing, cross-reference validation is skipped with a warning.

---

## Pipeline Overview

```
Spec files (TTL + Shapes + XMI + JSON + Xtext)
        │
        ▼
+---------------------------+
| sysml-codegen parsers     |
| - ttl_parser              |
| - shapes_parser           |
| - xmi_*_parser            |
| - json_schema_parser      |
| - xtext_crossref_parser   |
+---------------------------+
        │
        ▼
+---------------------------+
| Validation & coverage     |
| - type coverage (TTL vs XMI)
| - enum coverage (TTL vs JSON)
| - crossref coverage (Xtext vs resolution)
| - property validation coverage
+---------------------------+
        │
        ▼
+---------------------------+
| Generators                |
| - enum_generator          |
| - hierarchy_generator     |
| - relationship_generator  |
| - accessor_generator      |
| - validation_generator    |
| - crossref_generator      |
+---------------------------+
        │
        ▼
Generated Rust (OUT_DIR) -> included by sysml-core
```

---

## Generated Outputs (What They Contain)

### 1) `element_kind.generated.rs`

- `ElementKind` enum containing all KerML/SysML types
- Type hierarchy methods:
  - `supertypes()`, `direct_supertypes()`, `is_subtype_of()`
- Predicates and mappings:
  - `is_definition()`, `is_usage()`, `definition_of()` etc.
- Relationship constraints:
  - `relationship_source_type()`, `relationship_target_type()`
- Relationship target property mapping:
  - `relationship_target_property()`
  - `relationship_target_is_list()`

### 2) `enums.generated.rs`

- Value enums (e.g., `FeatureDirectionKind`, `VisibilityKind`, ...)
- Values validated against JSON schemas

### 3) `properties.generated.rs`

- Typed accessor structs (e.g., `PartUsageProps`) built from shapes
- Each accessor exposes property getters based on OSLC constraints
- Per-element validation method:
  - `fn validate(&self) -> ValidationResult`

### 4) `crossrefs.generated.rs`

- Registry of grammar cross-references with target types and scope strategies
- Used by name resolution for spec completeness

---

## Build-Time Spec Validation (Fails the Build)

`sysml-core/build.rs` enforces spec coverage and will **panic** on mismatch:

- **Type coverage**: TTL classes vs XMI classes must match
- **Enum coverage**: TTL enum values vs JSON enum values must match
- **Crossref coverage**: all Xtext cross-references must be implemented or explicitly skipped
- **Resolution completeness**: unresolved_* properties must map to registry entries (strict when `SYSML_STRICT_VALIDATION` is set)

Additional coverage reporting (warnings):

- Relationship constraint coverage (XMI + fallback)
- Property validation coverage (constraint types implemented vs missing)

---

## How to Inspect Generated Output

```bash
# Build sysml-core to generate outputs
cargo build -p sysml-core

# Inspect generated files (path is hashed)
ls target/debug/build/sysml-core-*/out/
```

Useful outputs:

- `element_kind.generated.rs`
- `enums.generated.rs`
- `properties.generated.rs`
- `crossrefs.generated.rs`

Build output includes helpful `cargo:warning=` lines that summarize coverage.

---

## Extending or Fixing Codegen

### Add or Update Element Types

1. Update spec files under `references/sysmlv2/`
2. Rebuild `sysml-core`
3. If validation fails, the error explains which types are missing or extra

### Add Property Constraints

1. Update shapes files (`KerML-shapes.ttl`, `SysML-shapes.ttl`)
2. Review parsing in `codegen/src/shapes_parser.rs`
3. Check validation logic in `codegen/src/validation_generator.rs`

### Add New Validation Constraint Types

1. Extend `ConstraintType` in `codegen/src/property_validation_validator.rs`
2. Update `validation_generator.rs` to emit runtime checks
3. Rebuild and confirm coverage output

### Adjust Relationship Constraints

1. XMI is authoritative (`KerML.xmi`, `SysML.xmi`)
2. Parsing lives in `codegen/src/xmi_relationship_parser.rs`
3. Generated constraints are used in `ElementKind::relationship_*` methods

### Adjust Cross-Reference Registry

1. Xtext grammar is authoritative (`KerML.xtext`, `SysML.xtext`)
2. Parsing lives in `codegen/src/xtext_crossref_parser.rs`
3. Coverage enforcement in `build.rs` ensures resolution completeness

---

## Common Failures and What They Mean

- `TYPE COVERAGE VALIDATION FAILED`:
  TTL vs XMI class lists are out of sync

- `ENUM COVERAGE VALIDATION FAILED`:
  TTL enum values do not match JSON schemas

- `CROSS-REFERENCE COVERAGE FAILED`:
  Xtext grammar crossrefs exist without resolution support

- `Resolution Spec Completeness FAILED` (when strict validation is enabled):
  unresolved_* properties in resolution are missing from crossref registry

---

## Related Files

- `crates/lang/sysml-core/build.rs`
- `crates/lang/sysml-core/src/lib.rs`
- `crates/lang/codegen/README.md`
- `crates/lang/codegen/src/*`
- `references/sysmlv2/*`
