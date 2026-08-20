# sysml-service-macros

The proc-macro backbone of the service hub: `#[service_impl]` + `#[service_command]` turn each annotated method on `SysmlService` into a deserializable request struct, a `CommandMeta` constant, a typed `ServiceCommand` impl, and a runtime `inventory` registration — exposing one method across every transport (CLI · LSP · REST · MCP) at once.

`Layer 5 · tooling` · `proc-macro codegen` · `crate-type: proc-macro` · `edition 2021`

## What it owns

This crate is a pure **compile-time** code generator. It carries no runtime types of its own — the structs it references (`CommandMeta`, `ParamMeta`, `CommandCategory`, the `ServiceCommand` trait, `CommandRegistration`) all live in the consuming crate, [sysml-service](../sysml-service/README.md). The macros emit code that names those types by absolute path (e.g. `crate::command_meta::CommandMeta`), so the generated code only compiles *inside* sysml-service.

It exposes exactly two attribute macros:

**`#[service_impl]`.**

The **container** macro. Applied once, to the `impl SysmlService` block. It walks every method, finds the ones annotated with `#[service_command(...)]`, generates the four companion items per command, strips the markers, and emits the generated items *after* the (otherwise untouched) impl block. Non-annotated methods pass through verbatim.

**`#[service_command(...)]`.**

The **marker** macro. It carries the command metadata (name, category, description, returns, stateful) but is a deliberate *no-op* on its own — its expansion returns its input unchanged. The real work happens because the enclosing `#[service_impl]` reads and consumes it before the compiler ever expands it.

The single real call site is `crates/tooling/sysml-service/src/lib.rs:381` — one `#[service_impl]` block containing **124** `#[service_command(` annotations (verified by grep, 2026-06-03). The generated registry asserts a floor of `command_count() >= 48` in sysml-service's tests; the eight categories below partition them by tier.

## Where it sits

```text
transports sysml-cli· sysml-lsp-server· sysml-api (REST)· sysml-mcp
▲ dispatch by name via the inventory registry
hub sysml-service command_trait · command_meta
▲ `#[service_impl]` expands generated code *inside* sysml-service
codegen sysml-service-macros
depends on ↓ (compile-time only)
deps syn 2 quote 1 proc-macro2 1 convert_case 0.6
```

Only one crate depends on sysml-service-macros: **sysml-service**. This crate fans out from that single edge to every service command and every transport.

## The expansion pipeline

```text
1 · parse CommandAttrs+MethodInfo ·parse.rs
▼ attribute args + method signature (params, types, `#[doc]`)
2 · map wire_type·type_string ·conversion_expr·type_mapping.rs
▼ Rust param types → owned wire types + wire→Rust conversions
3 · codegen Request struct·XXX_META ·Command impl·inventory::submit!
▼ runtime discovery
4 · runtime inventory::collect!(CommandRegistration) →execute_command(name, json)
```

## Worked example — what one annotation generates

This `#[service_command]` (the canonical example from `src/lib.rs:12-37`):

```
#[service_impl]
impl SysmlService {
    #[service_command(
        name = "sysml.find",
        category = Query,
        description = "Find elements by name pattern",
        returns = "Vec<Element>",
    )]
    pub fn find(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Name pattern (substring match)"] pattern: &str,
) -> Result<Vec<Element>, ServiceError> {
        // ... implementation ...
    }
}
```

…expands to four items emitted *after* the impl block (the method body and signature are left intact, minus the stripped `#[service_command]` and `#[doc]` param attributes):

```
// 1 · request struct — &str params become owned String wire types
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindRequest {
    pub uri: String,
    pub pattern: String,
}

// 2 · metadata constant — drives discovery, schemas, MCP tool listing
pub const FIND_META: crate::command_meta::CommandMeta = crate::command_meta::CommandMeta {
    name: "sysml.find",
    category: crate::command_meta::CommandCategory::Query,
    description: "Find elements by name pattern",
    params: &[
        crate::command_meta::ParamMeta { name: "uri",     ty: "string", required: true, description: "URI of the loaded model" },
        crate::command_meta::ParamMeta { name: "pattern", ty: "string", required: true, description: "Name pattern (substring match)" },
    ],
    returns: "Vec<Element>",
    stateful: false,
};

// 3 · zero-sized command struct + typed ServiceCommand impl
pub struct FindCommand;
impl crate::command_trait::ServiceCommand for FindCommand {
    const META: crate::command_meta::CommandMeta = FIND_META;
    type Request  = FindRequest;
    type Response = Vec<Element>;          // unwrapped from Result<T, _>
    fn execute(service: &SysmlService, req: Self::Request)
        -> Result<Self::Response, crate::ServiceError> {
        service.find(&req.uri, &req.pattern)   // &str args rebuilt from wire fields
    }
}

// 4 · inventory registration — type-erased JSON-in / JSON-out handler
inventory::submit! {
    crate::command_trait::CommandRegistration {
        meta: &FIND_META,
        handler: |service, body| {
            let req: FindRequest = serde_json::from_value(body)
                .map_err(|e| crate::ServiceError::InvalidInput(e.to_string()))?;
            let result = <FindCommand as crate::command_trait::ServiceCommand>::execute(service, req)?;
            serde_json::to_value(&result)
                .map_err(|e| crate::ServiceError::Internal(e.to_string()))
        },
    }
}
```

## Attribute reference

| Key | Value form | Required | Meaning |
|---|---|---|---|
| `name` | string literal | yes | Dot-separated command name, e.g. `"sysml.find"`. The dispatch key for `execute_command`. |
| `category` | bare identifier | yes | A `CommandCategory` variant (see table below). Emitted as `CommandCategory::<Ident>`. |
| `description` | string literal | yes | Human-readable summary surfaced in `--help`, MCP tool descriptions, etc. |
| `returns` | string literal | yes | Return-type *description* string (free text, e.g. `"Vec<Element>"`). Documentation only — the real response type is inferred from the method signature. |
| `stateful` | bool literal | no (default `false`) | Marks commands that touch session state (e.g. simulation sessions keyed by `ElementId`). |

### Command categories

The accepted `category` identifiers are the variants of `CommandCategory` in `sysml-service/src/command_meta.rs` (tiers mirror the MCP structure):

| Variant | Tier / purpose |
|---|---|
| `FileManagement` | File loading and workspace management |
| `Query` | Tier 1 — stateless model queries |
| `Analysis` | Tier 2 — cached analysis operations |
| `Execution` | Tier 3 — session-based execution |
| `Visualization` | Tier 4 — diagram and export operations |
| `Storage` | Tier 5 — persistence operations |

> ⚠ **category is an identifier, not a string.** Use `category = Query`, never `category = "Query"`. The parser rejects a string literal with *“`category` expects an identifier”*. An unknown variant fails later, when the generated `CommandCategory::<Ident>` path is type-checked in sysml-service.

## Wire-type mapping

Method parameters are usually borrowed (`&str`, `&[T]`, `&ElementId`) but a request struct must be owned and `Deserialize`. `type_mapping.rs` owns the contract: `wire_type` picks the request field type, `type_string` produces the human label stored in `ParamMeta.ty`, `is_optional` decides the serde defaulting, and `conversion_expr` emits the wire→Rust conversion (plus any `let` binding) used inside `execute`.

| Rust param type | Wire field type | ParamMeta.ty | Conversion in `execute` |
|---|---|---|---|
| `&str` | `String` | `string` | `&req.field` |
| `&String` | `String` | `string` | `&req.field` |
| `&Path` | `String` | `string` | `Path::new(&req.field)` |
| `usize · u32 · u64 · i32 · i64 · f64 · bool · String` | same | same name | `req.field` (by value) |
| `&ElementId` | `String` | `ElementId` | `ElementId::from_string(..)`, pass `&` |
| `&ProjectId` / `&CommitId` | `String` | `string` | `ProjectId::new(..)` / `CommitId::new(..)` |
| `&ElementKind` / `&RelationshipKind` | `String` | enum name | `serde_json::from_value(String(..))` |
| `&HashSet<T>` | `Vec<T>` | `Set<T>` | `.into_iter().collect()` |
| `&Vec<T>` / `&[T]` | `Vec<T>` | `[T]` | `&req.field` (Vec derefs to slice) |
| `(A, B, …)` tuple | tuple of wire types | `(a, b, …)` | `req.field` (pass-through) |
| `&ModelGraph` | `serde_json::Value` | `ModelGraph` | `from_value` → `sysml_core::ModelGraph`, pass `&` |
| `&Value` (serde_json) | `serde_json::Value` | `json` | `&req.field` (no deserialize) |
| `SnapshotMeta · Breakpoint · BatchFilter` | `serde_json::Value` | type name | `from_value` into the concrete type |
| `Option<T>` | `Option<wire(T)>` | `T?` | per-T (e.g. `Option<&str>` → `.as_deref()`) |

`Option<T>` fields additionally get `#[serde(default, skip_serializing_if = "Option::is_none")]` so absent JSON keys deserialize as `None`. Any param type not in this table is a hard compile error (*“unsupported parameter type”*) — extend `type_mapping.rs` to add one.

## Public API

#### `— *#[proc_macro_attribute] service_impl(attr, item)` — *container*

Applied to `impl SysmlService`. Parses the block as `syn::ItemImpl`, iterates its methods, and for each one carrying `#[service_command(...)]`: parses the attribute into `CommandAttrs`, the signature into `MethodInfo`, runs `codegen::generate`, then strips the marker and any param-level `#[doc]` attributes from the emitted method. Output = the original impl block followed by all generated items. The `attr` argument is ignored (takes no parameters).

#### `— *#[proc_macro_attribute] service_command(attr, item)` — *marker · no-op*

Returns `item` unchanged. Exists only so the attribute can be imported and written syntactically; `#[service_impl]` consumes its arguments before expansion. **If a method carries `#[service_command]` but its impl block is *not* wrapped in `#[service_impl]`, the annotation compiles cleanly and is silently ignored** — no request type, no meta, no registration, and the command never reaches any transport.

## Internal modules

| Module | Responsibility | Key items |
|---|---|---|
| `lib.rs` | The two attribute macros; the container's strip-and-emit loop. | `service_impl`, `service_command` |
| `parse.rs` | Parse attribute args and the method signature (params, types, doc comments, `Result<T,E>` unwrap). | `CommandAttrs`, `MethodInfo`, `ParamInfo`, `extract_response_type` |
| `type_mapping.rs` | Rust→wire type rules and wire→Rust conversion expressions (largest module). | `wire_type`, `type_string`, `is_optional`, `conversion_expr`, `ConversionResult` |
| `codegen.rs` | Assemble the four generated items; naming helpers. | `generate`, `request_struct_name`, `meta_const_name`, `command_struct_ident` |

### Naming conventions

From a method named `find` (snake_case → derived idents, verified by unit tests in `codegen.rs`):

- `find` → `FindRequest` (Pascal + `Request`)

- `find` → `FIND_META` (SCREAMING_SNAKE + `_META`)

- `find` → `FindCommand` (Pascal + `Command`)

## Dependencies

**Upstream (build-time).**

- `syn 2` — features `full`, `extra-traits`

- `quote 1` — token-stream quasi-quoting

- `proc-macro2 1` — `Ident`, `Span`, `TokenStream`

- `convert_case 0.6` — Pascal / ScreamingSnake casing

No runtime / domain dependencies. Workspace lints applied via `[lints] workspace = true`.

**Downstream (sole consumer).**

- [sysml-service](../sysml-service/README.md) — owns `CommandMeta`, `ParamMeta`, `CommandCategory`, the `ServiceCommand` trait, and `CommandRegistration` (`inventory::collect!`). Single `#[service_impl]` site at `src/lib.rs:381`.

The generated code is **not** standalone: it names sysml-service items by `crate::…` path and references the `SysmlService` type, so it only compiles within that crate.

## Pitfalls & invariants

> ⚠ **Marker outside the container is a silent no-op.** `#[service_command]` only does anything when the impl block is wrapped in `#[service_impl]`. There is no compile-time guard, so a misplaced annotation just disappears from every transport.

> ⚠ **First error aborts the batch.** The container loop returns on the first parse/codegen failure, so one malformed annotation masks any further errors across the ~124-command impl. Fix the first reported error and recompile.

> **Param `#[doc]` attributes are repurposed.** A `#[doc = "…"]` on a function parameter becomes that parameter's `ParamMeta.description`, then is stripped from the emitted signature (param-level doc is not valid Rust). With no doc, the description falls back to the parameter name.

> **`Result<T, E>` is auto-unwrapped.** The response type is the inner `T`; a non-`Result` return is wrapped in `Ok(..)`. The error type is always normalised to `ServiceError` in the generated `execute`.

> **Unsupported param types fail loudly** with *“unsupported parameter type”* — but the span points at the whole method/argument rather than the offending type, because `type_mapping` returns stringly-typed errors that lose finer span info. Add a branch in `type_mapping.rs` to support a new type.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
