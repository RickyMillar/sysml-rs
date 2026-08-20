# sysml-query

Transport-agnostic structured query engine for row-shaped reads over a `ModelGraph`: composable filters, projections, sort keys, and cursor pagination.

`Layer 4 · tooling` · `model query engine` · `crate-type: rlib` · `single file · src/lib.rs`

## Overview

`sysml-query` answers one question for the rest of the stack: *"given this graph, which elements match this query, in what order, and on what page?"* It is a pure function over an immutable `sysml_core::ModelGraph` — it owns no state, no workspace, no cache, no auth, and no transport. Callers pass a `QuerySpec` and a precomputed `revision`; the engine returns a paged `QueryResult`.

The crate is consumed by exactly one downstream: `sysml-service`, the unified hub that dispatches commands to CLI / LSP / REST / MCP. The service owns workspace selection, its own process-level `query_cache`, and is responsible for computing the `revision` (via `graph_revision`) and stamping the result's `cache_status`. Every type derives `serde` + `schemars::JsonSchema`: the wire format is JSON, and the schema is published through the service for MCP/REST clients.

>  **The invariant this crate enforces:** query semantics are identical across every transport. Only the `QueryProfile` (default page size) differs — `Service` pages at 500, `Mcp` at 100 to protect agent context windows. Same graph + same `QuerySpec` ⇒ same rows, regardless of who called.

## Where it sits

```text
callers CLI· LSP· REST· MCP
▼ dispatch
L4 hub sysml-service owns workspace · query_cache · revision · cache_status
▼ QuerySpec + revision
L4 query sysml-query filter → sort → paginate → project
▼ reads (immutable)
L1 model sysml-core::ModelGraph build_view_index requirements_unverified
```

## Request / response model

A query flows through four stages: **filter** (which elements), **sort** (deterministic order, with `ElementId` as the final tie-breaker), **paginate** (cursor + limit), then **project** (row shape).

### QuerySpec fields

| Field | Type | Default | Meaning |
|---|---|---|---|
| `filter` | `Filter` | `All{filters:[]}` (matches everything) | Composable predicate tree, max depth 8. |
| `projection` | `Projection` | `Summary` | Row shape of the response. |
| `sort` | `Vec<SortKey>` | `[]` | Ordered sort keys; ties broken by `ElementId`. |
| `limit` | `Option<usize>` | profile default, clamped to 1000 | Page size. `Service`=500, `Mcp`=100. |
| `cursor` | `Option<String>` | `None` | Opaque base64 page token from a prior result. |

### Filter variants

| Variant (tag) | Group | Matches when… |
|---|---|---|
| `all` | composite | every child filter matches (AND). |
| `any` | composite | at least one child matches (OR). |
| `not` | composite | the boxed child does not match. |
| `kind` | primitive | element's `ElementKind` is in `kinds`. |
| `name_match` | primitive | element `name` matches a `NameMatch`. |
| `qualified_name_match` | primitive | element's qualified name matches a `NameMatch`. |
| `owner` | primitive | owner chain satisfies `OwnerFilter` (id/kind, optionally transitive). |
| `id_in` | primitive | element `id` is in the given set. |
| `has_relation` | primitive | element is source/target of a relationship of a given kind (optional target kind). |
| `unverified_requirement` | legacy shim | requirement is unverified — wraps `core_query::requirements_unverified` (legacy `sysml.unverified`). |
| `view` | legacy shim | element is a view, optionally under a given viewpoint (legacy view wrapper). |
| `viewpoint` | legacy shim | element is a viewpoint, optionally for a given stakeholder (legacy viewpoint wrapper). |

> ⚠  **Legacy shims.** `view`, `viewpoint`, and `unverified_requirement` bake legacy service command names into the otherwise generic query vocabulary. They are slated to be re-expressed via the composable primitives (`kind` + `has_relation`) and retired. Prefer the primitives for new work.

### Projection → response row shape

`QueryResult.rows` is `#[serde(untagged)]` — it serializes as the bare payload (array or number) so clients read it directly. The projection chosen decides which `QueryRows` variant lands.

| Projection | QueryRows variant | Payload |
|---|---|---|
| `ids` | `Ids` | `Vec<ElementId>` for the page. |
| `elements` | `Elements` | `Vec<Element>` — full cloned elements. |
| `summary` | `Summary` | `Vec<ElementSummary>` — id/name/qname/kind/owner/span. |
| `summary_expand` | `Summary` | as `summary`, plus one-level `SummaryExpansion::View(ViewSummary)` for views. |
| `count` | `Count` | `usize` total over the filtered set (no paging, no projection cost). |

## Public API

#### `— *fn execute_query(graph, spec, revision) -> Result<QueryResult, QueryError>`

Convenience entry point using the `Service` profile. Validates the filter (rejects malformed `NameMatch` and over-deep nesting), filters and sorts the full element set, paginates with the cursor, then projects.

#### `— *fn execute_query_with_profile(graph, spec, revision, profile) -> Result<QueryResult, QueryError>`

As above but takes an explicit `QueryProfile`. The only difference the profile makes is the default page size when `spec.limit` is `None` (`Service`=500, `Mcp`=100); both clamp to `MAX_LIMIT`=1000.

#### `— *fn graph_revision(graph) -> u64`

Deterministic fingerprint the service uses to key its query cache and to validate cursors. **Hashes only the count and sorted ID sets of elements and relationships** — not element content. An in-place edit (rename, kind change, re-parent) that adds/removes no ID yields an identical revision. See pitfalls.

#### `— *struct QueryResult { rows, total_estimate, cursor, cursor_invalidated, revision, cache_status }`

`total_estimate` is the full filtered count; `cursor` is the token for the next page (`None` when exhausted); `cursor_invalidated` is `true` when the incoming cursor's revision differs from the current one. `cache_status` is `Uncached` from the engine — the service overwrites it via `with_cache_status`.

#### `— *struct NameMatch { exact?, prefix?, contains?, regex?, ci }`

Exactly one of `exact` / `prefix` / `contains` / `regex` must be set, else `QueryError::Invalid`. `ci` enables case-insensitive matching (regex gets an inline `(?i:…)` wrapper).

#### `— *enum QueryError { Invalid, Regex, Cursor }`

`thiserror`-derived. `Invalid` for filter/name-match validation, `Regex` for an uncompilable pattern, `Cursor` for decode/version failures.

## Usage example

End-to-end: build a `QuerySpec`, derive the revision, execute, read the rows. This compiles against the current API (mirrors the crate's own tests).

```
use sysml_query::{execute_query, graph_revision, Filter, Projection, QueryRows, QuerySpec};
use sysml_core::{Element, ElementKind, ModelGraph};

let mut graph = ModelGraph::new();
let pkg = graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Pkg"));
let _part = graph.add_element(
    Element::new_with_kind(ElementKind::PartUsage)
        .with_name("Engine")
        .with_owner(pkg),
);

let spec = QuerySpec {
    filter: Filter::Kind { kinds: vec![ElementKind::PartUsage] },
    projection: Projection::Summary,
    ..QuerySpec::default()
};

let revision = graph_revision(&graph);
let result = execute_query(&graph, &spec, revision).unwrap();

if let QueryRows::Summary(rows) = result.rows {
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].qualified_name.as_deref(), Some("Pkg::Engine"));
}
```

### Pagination loop

```
let revision = graph_revision(&graph);
let mut cursor = None;
loop {
    let spec = QuerySpec { limit: Some(100), cursor: cursor.clone(), ..QuerySpec::default() };
    let page = execute_query(&graph, &spec, revision).unwrap();
    // … consume page.rows …
    match page.cursor {
        Some(next) => cursor = Some(next),
        None => break, // exhausted
    }
}
```

If the graph changes mid-pagination, `graph_revision` moves, the next page's `cursor_invalidated` is set, and the start offset resets — callers should restart the page walk on that signal.

## Dependencies

**Upstream (reads).**

- `sysml-core` — `ModelGraph`, `Element`, `ElementKind`, `RelationshipKind`; view helpers `build_view_index`, `views_by_viewpoint`, `viewpoints_by_stakeholder`, `ViewSummary`; and `query::requirements_unverified`.

- `sysml-id` — `ElementId`.

- `sysml-span` — `Span` (carried on summaries).

- `serde` / `serde_json` / `schemars` — JSON wire format + schema.

- `regex`, `base64` (URL-safe cursor), `thiserror`.

**Downstream (sole consumer).**

- `sysml-service` — owns the `query_cache: DashMap<String, QueryResult>`, computes `graph_revision`, picks the `QueryProfile`, and stamps `cache_status`. CLI / LSP / REST / MCP reach this engine only through the service.

## Pitfalls & invariants

- **No index — full scan per query.** Every call iterates `graph.elements.values()`, filters and sorts the entire set, *then* paginates. Deep cursors re-scan and re-sort the whole graph each page. The service's process-level cache mitigates repeats; a cold query is O(n)+O(n log n).

- **`graph_revision` is ID-only.** It hashes counts + sorted ID sets, not content. An in-place rename / kind-change / re-parent that touches no IDs produces an identical revision, which can serve stale cached rows or silently accept a cursor minted against different content.

- **One `NameMatch` mode only.** Setting zero or multiple of `exact/prefix/contains/regex` is a hard `QueryError::Invalid`.

- **Filter depth is capped at 8.** Deeper nesting is rejected before execution.

- **Cursors are versioned.** `CURSOR_VERSION` must bump if the cursor payload shape changes; old cursors then fail to decode rather than mis-page.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
