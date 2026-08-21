# Validation (Spec-Driven)

Validation in sysml-rs is **spec-driven**. Most validation logic is generated from the SysML/KerML specification files and enforced in `sysml-core` at runtime.

There are two primary validation layers:

1. **Property validation** (per-element, shapes-based)
2. **Structural + relationship validation** (graph integrity + spec constraints)

---

## 1) Property Validation (OSLC Shapes)

Property validation is generated from the OSLC shapes files:

- `references/sysmlv2/KerML-shapes.ttl`
- `references/sysmlv2/SysML-shapes.ttl`

`sysml-codegen` parses these files and generates a typed accessor for each element type, with a `validate()` method that checks the property constraints.

### What is validated

Generated validation covers:

- **Required properties** (Exactly-one cardinality)
- **Type checks** (ElementId, string, bool, etc.)
- **Min cardinality** (One-or-many)
- **Max cardinality** (Zero-or-one / Exactly-one)
- **Read-only flags** (documented; not enforced at runtime)

These constraints are summarized during build by the property validation coverage report.

### Where it lives

- Generator: `crates/lang/codegen/src/validation_generator.rs`
- Constraint coverage logic: `crates/lang/codegen/src/property_validation_validator.rs`
- Runtime error types: `crates/lang/sysml-core/src/validation.rs`
- Generated accessors: `target/*/build/sysml-core-*/out/properties.generated.rs`

### How to use it

```rust
use sysml_core::{Element, ElementKind};

let element = Element::new_with_kind(ElementKind::PartUsage);
let part = element.as_part_usage().unwrap();
let result = part.validate();

if !result.is_valid() {
    for err in result.errors {
        eprintln!("{}", err);
    }
}
```

### Property validation error codes

Property validation errors are converted into diagnostics with codes:

- `V001` MissingRequired
- `V002` WrongType
- `V003` MinCardinality
- `V004` MaxCardinality
- `V005` ReadOnly

See: `crates/lang/sysml-core/src/validation.rs`

---

## 2) Structural + Relationship Validation

Structural validation checks that the **graph itself is well-formed** and that **relationship endpoints conform to the spec**.

### Structural integrity checks

`ModelGraph::validate_structure()` verifies:

- No orphan elements (unless allowed root kinds)
- No ownership cycles
- Membership references point to real elements
- Owning membership links are valid

### Relationship type constraints

`ModelGraph::validate_relationship_types()` validates relationship endpoints using spec-derived rules:

- **Source type** constraint (owner element kind)
- **Target type** constraint (target element kind)

These constraints are derived from XMI metamodel files and generated into `ElementKind` methods:

- `relationship_source_type()`
- `relationship_target_type()`
- `relationship_target_property()`
- `relationship_target_is_list()`

### When to run

- Run structural validation after parsing
- Run relationship type validation **after name resolution**, when target properties contain resolved `ElementId`s

### Usage example

```rust
use sysml_core::ModelGraph;

let graph = ModelGraph::new();

// After parsing + resolution:
let structural_errors = graph.validate_structure();
let relationship_errors = graph.validate_relationship_types();
```

### Structural error codes

Structural errors are converted into diagnostics with codes:

- `E001` OrphanElement
- `E002` OwnershipCycle
- `E003` DanglingMembershipRef
- `E004` RelationshipSourceTypeMismatch
- `E005` RelationshipTargetTypeMismatch
- `E006` DanglingRelationshipRef
- `E007` DanglingOwningMembership
- `E008` InvalidOwningMembership

See: `crates/lang/sysml-core/src/structural_validation.rs`

---

## 3) Semantic Validation (Spec Rules)

Semantic validation enforces SysML v2 specification rules that go beyond structural integrity. These are **domain-specific** checks that ensure a model is semantically valid.

### Architecture

The semantic validation system uses a **codegen pipeline**:

```
codegen/src/semantic_rules.toml        # 86 rules across 10 categories
    → semantic_rule_parser.rs          # TOML → SemanticRule structs
    → semantic_validation_generator.rs # Generates dispatch match
    → semantic_validation.generated.rs # Included in sysml-core via include!()
```

### Error code ranges

| Category | Code Range | Module |
|----------|-----------|--------|
| Distinguishability | S001-S010 | `semantic_checks/distinguishability.rs` |
| Typing | S011-S020 | `semantic_checks/typing.rs` |
| Specialization | S021-S030 | `semantic_checks/specialization.rs` |
| Ownership | S031-S040 | `semantic_checks/ownership.rs` |
| Cardinality | S041-S050 | `semantic_checks/cardinality.rs` |
| Variation | S051-S060 | `semantic_checks/variation.rs` |
| Connectors | S100-S105 | `semantic_checks/connectors.rs` |
| States | S110-S114 | `semantic_checks/states.rs` |
| Actions | S120-S127 | `semantic_checks/actions.rs` |
| Requirements | S130-S140 | `semantic_checks/requirements.rs` |

### Where it lives

- Rule catalog: `codegen/src/semantic_rules.toml`
- Rule parser: `codegen/src/semantic_rule_parser.rs`
- Code generator: `codegen/src/semantic_validation_generator.rs`
- Generated code: `target/*/build/sysml-core-*/out/semantic_validation.generated.rs`
- Check modules: `sysml-core/src/semantic_checks/*.rs`
- Error types: `sysml-core/src/validation.rs` (`SemanticError`, `SemanticErrorKind`)

### LSP integration

Semantic validation runs as part of `parse_and_publish_diagnostics` in the LSP server. A **timeout budget** ensures responsiveness: if structural validation consumes more than 50% of the resolution timeout, semantic validation is skipped for that file.

### Coverage tracking

`sysml-core/build.rs` scans `semantic_checks/*.rs` for `pub fn` signatures and compares against the rule catalog in `semantic_rules.toml`, reporting coverage percentage at build time. Current status: 86 rules, 70 check functions, 100% coverage.

### Usage

```rust
use sysml_core::validate_semantic;

let errors = validate_semantic(&graph);
for err in &errors {
    eprintln!("[{}] {}: {}", err.code, err.element_name, err.message);
}
```

---

## LSP Validation Tiers

The LSP server gates which validation checks run based on the **resolution tier** reached during the diagnostic pipeline. This avoids false positives when cross-file resolution has not completed.

| Tier | Resolution scope | Checks that run |
|------|-----------------|-----------------|
| **T1Syntax** | Parse only | Tree-sitter parse errors. No validation. |
| **T2Local** | Single-file resolution | Structural (E001-E008), relationship types, semantic (S001+). Property validation **skipped**. |
| **T3Full** | Multi-file + library resolution | All T2 checks **plus** property validation (V001-V005). |

Property validation (V001-V005) is restricted to T3Full because V001 ("missing required property") checks for properties populated by cross-file resolution (pass2). Running V001 at T2Local would produce false positives for every element whose required properties come from library imports or cross-file specializations.

### Additional gating controls

- **`skip_later_phases`**: When the file has 16 or more syntax errors, all validation (structural, semantic, property) is skipped entirely. Only parse errors are reported.
- **`features.validation`**: A per-file toggle that disables all validation when set to `false`. Controlled by server configuration.

### Telemetry

The diagnostic decision log (`lsp.diagnostics.decision`) includes `property_validation_ran` (bool) and `property_validation_errors` (count) fields to track tier gating behavior in production.

### Reference

`ResolutionTier` enum: `sysml-lsp-server/src/background.rs`

---

## Spec Coverage Enforcement

At build time, `sysml-core/build.rs` validates that generated validation logic is aligned with the spec:

- TTL vs XMI type coverage
- TTL vs JSON enum coverage
- Cross-reference coverage (Xtext vs resolution)
- Property validation coverage summary

This ensures that runtime validators are driven by **authoritative spec inputs**.

---

## Extending Validation

### Adding a new constraint type

1. Update the shapes files in `references/sysmlv2/`
2. Extend `ConstraintType` in `codegen/src/property_validation_validator.rs`
3. Emit runtime checks in `codegen/src/validation_generator.rs`
4. Rebuild and verify coverage output

### Adding a new structural rule

1. Implement the rule in `sysml-core/src/structural_validation.rs`
2. Add a diagnostic code mapping
3. Add tests to the same module

---

## Related Files

- `crates/lang/sysml-core/src/validation.rs`
- `crates/lang/sysml-core/src/structural_validation.rs`
- `crates/lang/codegen/src/validation_generator.rs`
- `crates/lang/codegen/src/property_validation_validator.rs`
- `crates/lang/sysml-core/build.rs`
