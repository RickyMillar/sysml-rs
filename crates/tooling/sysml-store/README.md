# sysml-store

Persistence layer for the SysML v2 toolchain. Owns two unrelated stores: commit-addressed *model snapshots* (version control) and an LRU archive of completed *runtime sessions* for replay and comparison.

`Layer 4 · tooling` · `persistence` · `version control` · `crate-type: rlib` · `feature: postgres (off by default)`

## Overview — two stores in one crate

`sysml-store` defines storage interfaces and in-memory implementations for two distinct concerns. They share a crate but nothing else — different keys, different error types, different eviction policies.

**1 · Model snapshot store.**

Version control for the semantic model. A `ModelGraph` is serialised to JSON and stored under a `(ProjectId, CommitId)` key with `SnapshotMeta` (parent link, message, Unix-seconds timestamp).

- Trait: `Store` (synchronous)

- Impl: `InMemoryStore` (default)

- Impl: `PostgresStore` (`postgres` feature, async, does *not* implement `Store`)

- Error: `StoreError`

**2 · Session archive largest file.**

Persistent record of *completed runtime sessions* — keyed by opaque session id, not by commit. Captures workflow origin, overrides, verdicts, and optional snapshot history so the UI can replay or compare past runs.

- Trait: `SessionArchive` (`Send + Sync`)

- Impl: `InMemorySessionArchive` (bounded LRU ring, golden-pinned entries exempt)

- Error: `ArchiveError`

- Module: `session_archive` (re-exported from the crate root)

>  **Why one crate?** Both subsystems are pure persistence with a `serde`-based wire contract and no `sysml-runtime` dependency. Snapshots store model JSON; the archive stores execution snapshots as opaque `serde_json::Value` so the service layer converts `ExecutionSnapshot ⇄ JSON` at the boundary. (Tracked as low-severity tech debt — see CLAUDE.md.)

## Where it sits

```text
consumers sysml-service · sysml-api
▼ depends on
L4 store sysml-store Store · InMemoryStore · PostgresStore SessionArchive · InMemorySessionArchive
▼ depends on
upstream sysml-core (ModelGraph, json) sysml-id (ProjectId, CommitId) serde · serde_json · thiserror
postgres sqlx (optional) tokio (optional)
```

`sysml-service` is the primary downstream consumer (it holds the `Store` behind `Arc<RwLock>` and the archive behind `Arc<dyn SessionArchive>`); `sysml-api` also depends on the crate. `sysml-store` depends only on `sysml-core` and `sysml-id` in the lang layer — no parser, no runtime.

## Public API

Everything below is re-exported from the crate root. The snapshot types live in `src/lib.rs`; the archive types are defined in `src/session_archive.rs` and re-exported via `pub use session_archive::{…}`.

| Type | Kind | Store | Purpose |
|---|---|---|---|
| `Store` | trait (sync) | snapshot | Backend interface: `put_snapshot`, `get_snapshot`, `latest`, `list_commits`, `list_projects`. |
| `InMemoryStore` | struct | snapshot | HashMap-backed `Store` impl. Caps commits per project at 100 (oldest evicted). |
| `PostgresStore` | struct (feat) | snapshot | sqlx `PgPool` backend. Async `*_async` methods; does NOT implement `Store`. Behind `postgres`. |
| `Snapshot` | struct | snapshot | Stored entry: `meta` + JSON `data`. `new(meta, graph)` serialises; `graph()` deserialises. |
| `SnapshotMeta` | struct | snapshot | Commit id, optional parent, message, Unix-seconds timestamp. Builder: `with_parent`, `with_timestamp`. |
| `StoreError` | enum | snapshot | 6 variants: `ProjectNotFound`, `CommitNotFound`, `SerializationError`, `DeserializationError`, `DatabaseError`, `Conflict`. |
| `create_in_memory_store()` | fn (feat) | snapshot | Convenience constructor exported under `postgres`; returns an `InMemoryStore` fallback. |
| `SessionArchive` | trait | archive | `Send + Sync` interface: `record`, `get`, `list`, `mark_golden`, `unmark_golden`. |
| `InMemorySessionArchive` | struct | archive | Bounded LRU ring (`RwLock`). `len` / `is_empty` helpers. Golden entries pinned. |
| `ArchivedSession` | struct | archive | Full archive entry: id, label, origin, workspace, timings, ticks, overrides, verdicts, snapshots, golden. `to_summary()`. |
| `ArchivedSessionSummary` | struct | archive | List-view projection — omits `snapshots`, `overrides`, raw `verdicts` (carries `verdict_counts` + `snapshot_count`). |
| `ArchivedVerdict` | struct | archive | One verdict: `case_id`, lowercase `verdict` string, ms `timestamp`, optional `evidence`. |
| `ArchivedEvidence` | struct | archive | Deep-link back to `session_id` + `tick` + optional `element_id` that produced a verdict. |
| `VerdictCounts` | struct | archive | 4-bucket tally (`pass/fail/inconclusive/error`). `from_verdicts`, `total`; unknown discriminants dropped. |
| `SessionOrigin` | enum | archive | 5 workflow origins: `Run`, `Verify`, `Sweep`, `MonteCarlo`, `TradeStudy`. snake_case wire; legacy `"compliance"` → `Verify`. |
| `GoldenMetadata` | struct | archive | Golden-pin marker: `label` + ms `marked_at`. Present iff session was pinned. |
| `ArchiveFilter` | struct | archive | Additive query for `list`: `workspace_uri`, `origin`, `since`, `only_golden`. |
| `ArchiveError` | enum | archive | 2 variants: `NotFound`, `Internal` (lock poisoning / impl failure). |
| `MAX_ARCHIVED_SESSIONS` | const | archive | `256` — non-golden ring cap. Golden sessions are stored on top, unbounded. |

## Source modules

| File | Size | Responsibility | Key types |
|---|---|---|---|
| `src/session_archive.rs` | ~32 KB | Runtime-session archive: wire types, `SessionArchive` trait, bounded LRU in-memory impl with golden pinning. | `SessionArchive`, `InMemorySessionArchive`, `ArchivedSession`, `SessionOrigin`, `ArchiveFilter`, … |
| `src/lib.rs` | ~13 KB | Snapshot store: `Store` trait, `InMemoryStore` with 100-commit cap, snapshot/meta types. Re-exports the archive module. | `Store`, `InMemoryStore`, `Snapshot`, `SnapshotMeta`, `StoreError` |
| `src/postgres.rs` | ~5 KB | PostgreSQL backend (feature-gated). `PgPool`, schema DDL, async snapshot ops. | `PostgresStore`, `create_in_memory_store` |

## API reference

### Snapshot store

#### `trait Store`

Synchronous backend interface for commit-addressed model snapshots. Only `InMemoryStore` implements it today.

```
pub trait Store {
    fn put_snapshot(&mut self, project: &ProjectId, meta: SnapshotMeta,
                    graph: &ModelGraph) -> Result<(), StoreError>;
    fn get_snapshot(&self, project: &ProjectId, commit: &CommitId)
                    -> Result<Option<Snapshot>, StoreError>;
    fn latest(&self, project: &ProjectId) -> Result<Option<CommitId>, StoreError>;
    fn list_commits(&self, project: &ProjectId) -> Result<Vec<SnapshotMeta>, StoreError>;
    fn list_projects(&self) -> Result<Vec<ProjectId>, StoreError>;
}
```

`list_commits` returns most-recent-first. `put_snapshot` with an existing commit id returns `StoreError::Conflict`.

#### `struct PostgresStore (feature = "postgres")`

Async sqlx backend. **Does not implement `Store`** — it exposes parallel async methods and has no async equivalents of `list_commits`/`list_projects` yet. Callers cannot treat the two backends polymorphically (known abstraction debt).

```
impl PostgresStore {
    pub async fn new(database_url: &str) -> Result<Self, StoreError>;
    pub async fn init_schema(&self) -> Result<(), StoreError>;       // CREATE TABLE snapshots + index
    pub async fn put_snapshot_async(&self, project: &ProjectId,
                                    meta: SnapshotMeta, graph: &ModelGraph)
                                    -> Result<(), StoreError>;
    pub async fn get_snapshot_async(&self, project: &ProjectId, commit: &CommitId)
                                    -> Result<Option<Snapshot>, StoreError>;
    pub async fn latest_async(&self, project: &ProjectId)
                              -> Result<Option<CommitId>, StoreError>;
}
```

Schema: table `snapshots(project_id, commit_id, parent_id, message, timestamp, data JSONB)`, PK `(project_id, commit_id)`, plus a descending-timestamp index per project.

### Session archive

#### `trait SessionArchive`

`Send + Sync` so the service can hold it behind `Arc<dyn SessionArchive>`.

```
pub trait SessionArchive: Send + Sync {
    fn record(&self, entry: ArchivedSession) -> Result<(), ArchiveError>;   // idempotent on entry.id
    fn get(&self, id: &str) -> Option<ArchivedSession>;
    fn list(&self, filter: ArchiveFilter) -> Vec<ArchivedSessionSummary>;     // newest-first
    fn mark_golden(&self, id: &str, label: String) -> Result<(), ArchiveError>;
    fn unmark_golden(&self, id: &str) -> Result<(), ArchiveError>;
}
```

`record` replaces in place when the id already exists (no duplicate, no LRU bump). `list` filters are additive and project to summaries.

#### `struct InMemorySessionArchive — bounded LRU ring`

Single `RwLock<Inner>`. Reads (`get`, `list`) take a read lock; mutators take a write lock. Entries are stored as `Arc<ArchivedSession>` for cheap clone-on-read.

- **Cap:** `MAX_ARCHIVED_SESSIONS = 256` non-golden entries.

- **Eviction:** FIFO over insertion order, *skipping* golden entries. If every entry is golden, the ring grows past the cap rather than drop pinned data.

- **Golden pinning:** `mark_golden` stamps `GoldenMetadata`; pinned sessions never evict until `unmark_golden`.

#### `wire contract — ArchivedSession / SessionOrigin`

The `ArchivedSession` / `ArchivedSessionSummary` JSON shape is the stable contract consumed by the frontend. All timestamps are Unix milliseconds (`i64`). `SessionOrigin` serialises snake_case:

```
"run" | "verify" | "sweep" | "monte_carlo" | "trade_study"
// deserialise-only alias: "compliance" -> Verify (legacy rows)
```

Summaries deliberately omit `snapshots`, `overrides`, and raw `verdicts`; they carry `verdict_counts` (a `VerdictCounts` tally) and `snapshot_count` instead.

## Eviction at a glance

**Snapshot store · 100-commit cap.**

```text
putcommit N→push_back
▼ when len > 100
evictoldest commit + snapshot
```

`MAX_COMMITS_PER_PROJECT = 100` (private const in `lib.rs`). Eviction drops the oldest snapshot data; the `latest` pointer is not reconciled (minor debt for evicted parents).

**Session archive · 256-entry LRU.**

```text
recordsession→order.push_back
▼ when len > 256
evictoldest non-golden·golden ⇒ skipped
```

`MAX_ARCHIVED_SESSIONS = 256` (public const). Golden-pinned sessions are exempt from the count's eviction scan.

## Usage

### Model snapshots

```
use sysml_store::{InMemoryStore, Store, SnapshotMeta};
use sysml_core::ModelGraph;
use sysml_id::{ProjectId, CommitId};

let mut store = InMemoryStore::new();
let project = ProjectId::new("my-project");
let graph = ModelGraph::new();

let meta = SnapshotMeta::new(CommitId::new("v1"), "Initial commit");
store.put_snapshot(&project, meta, &graph)?;

let latest = store.latest(&project)?;                       // Some(CommitId("v1"))
let snap   = store.get_snapshot(&project, &CommitId::new("v1"))?.unwrap();
let restored: ModelGraph = snap.graph()?;                    // JSON round-trip back
let commits = store.list_commits(&project)?;                 // most-recent-first
# Ok::<(), sysml_store::StoreError>(())
```

### Session archive

```
use sysml_store::{
    InMemorySessionArchive, SessionArchive, ArchivedSession,
    SessionOrigin, ArchiveFilter,
};

let archive = InMemorySessionArchive::new();
archive.record(ArchivedSession {
    id: "sess-1".into(),
    label: None,
    origin: SessionOrigin::Verify,
    workspace_uri: "file:///workspace".into(),
    created_at: 1_700_000_000_000,   // Unix ms
    ended_at: 1_700_000_001_000,
    ticks: 42,
    overrides: vec![],
    verdicts: vec![],
    snapshots: vec![],               // opaque serde_json::Value history
    golden: None,
})?;

archive.mark_golden("sess-1", "baseline".into())?;           // pin it
let recent = archive.list(ArchiveFilter {                    // newest-first summaries
    origin: Some(SessionOrigin::Verify),
    only_golden: true,
    ..Default::default()
});
# Ok::<(), sysml_store::ArchiveError>(())
```

## Dependencies & features

**Always on.**

- `sysml-core` (feature `serde`) — `ModelGraph`, `json::{to_json_string, from_json_str}`

- `sysml-id` (feature `serde`) — `ProjectId`, `CommitId`

- `serde` + `serde_json` — wire serialisation

- `thiserror` — `StoreError`, `ArchiveError`

**Feature `postgres` (off by default).**

- Enables optional `sqlx` + `tokio`

- Compiles the `postgres` module → `PostgresStore` + `create_in_memory_store()`

- Build/test with `--features postgres` (PostgreSQL not required to compile, only to run async ops)

**Downstream:** `sysml-service` (primary consumer — owns the `Store` and `SessionArchive` as shared service state) and `sysml-api`.

## Invariants & pitfalls

>  **Commit-addressed.** Snapshots key on `(ProjectId, CommitId)`. Re-using a commit id within a project is rejected with `StoreError::Conflict`.

>  **JSON, not bincode.** Model data round-trips through `sysml_core::json`. `sysml_meta::Value` uses `#[serde(untagged)]`, which bincode cannot round-trip — never serialise a `ModelGraph` with bincode.

> ⚠  **Sync/async split.** The `Store` trait is synchronous and only `InMemoryStore` implements it. `PostgresStore` lives outside the trait with `*_async` methods and is missing async `list_commits`/`list_projects`. The two backends are not yet polymorphic.

> ⚠  **Bounded retention.** Both stores hard-truncate: snapshots at 100 commits/project, the archive at 256 non-golden sessions. In-memory snapshot history is not durable — treat it as a working cache, not a backup.

>  **Stable archive wire shape.** `ArchivedSession`/`ArchivedSessionSummary` JSON is consumed by the frontend. Timestamps are Unix ms (`i64`); `SessionOrigin` is snake_case with a deserialise-only `"compliance" → Verify` alias. Changing field names or origin strings is a breaking contract change.

## Testing

```
# Unit tests (InMemoryStore + InMemorySessionArchive, no DB needed)
cargo test -p sysml-store

# Include the PostgreSQL backend (compiles sqlx; running async ops needs a live DB)
cargo test -p sysml-store --features postgres
```

Coverage: snapshot put/get round-trip, latest tracking, commit-list ordering, project listing, conflict detection, 100-commit cap eviction; archive record/get/list/filter, idempotent record, golden mark/unmark/pinning, ring eviction, and serde wire-shape lock-down.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
