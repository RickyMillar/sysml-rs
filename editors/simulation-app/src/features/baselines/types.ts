/**
 * Baseline / snapshot-diff wire types — TS mirrors of the Rust shapes
 * behind `sysml.store.baseline.*` and `sysml.store.diff` (B3).
 *
 * Sources of truth:
 * - `crates/tooling/sysml-store/src/lib.rs` (`BaselineMeta`, `SnapshotMeta`)
 * - `crates/lang/sysml-core/src/diff.rs` (`GraphDiff`, `ElementDiff`,
 *   `FieldDelta`, `RelationshipDelta`)
 *
 * Identity contract (ADR-009, binding): diff correlation is strictly by
 * ElementId. A scope rename regenerates descendant ids, so the subtree
 * surfaces as removed+added — the UI must present that as "identity
 * changed" and MUST NOT name-match the two sides back together.
 */

/** Git provenance captured at baseline creation (B6) — corroborating
 *  metadata only; the content-addressed commit is the identity. */
export interface GitProvenance {
  /** HEAD commit SHA at creation time. */
  sha: string;
  /** Uncommitted changes existed — the SHA alone does not reproduce
   *  the baselined content. Recorded honestly, never refused. */
  dirty: boolean;
  /** Branch name; null on a detached HEAD. */
  branch: string | null;
}

/** `BaselineMeta` — a named, immutable pointer to a commit. */
export interface BaselineMeta {
  /** Unique (per project) baseline name, e.g. `"B2 — PDR"`. */
  name: string;
  /** The commit this baseline points at (transparent string id). */
  commit: string;
  /** Creation timestamp (Unix epoch SECONDS — not ms). */
  created_at: number;
  /** Git provenance; null/absent = not captured (non-git workspace or
   *  a baseline predating B6) — never fabricated. */
  provenance?: GitProvenance | null;
}

/** `SnapshotMeta` — metadata for one stored commit. */
export interface SnapshotMeta {
  commit: string;
  parent: string | null;
  message: string;
  /** Unix epoch SECONDS. */
  timestamp: number;
}

/**
 * `FieldDelta` — one changed field on an element. Internally tagged on
 * `field` (snake_case). `from`/`to` prop values are untagged
 * `sysml_meta::Value` — arbitrary JSON.
 */
export type FieldDelta =
  | { field: 'kind'; from: string; to: string }
  | { field: 'name'; from: string | null; to: string | null }
  | { field: 'owner'; from: string | null; to: string | null }
  | { field: 'prop_added'; key: string; to: unknown }
  | { field: 'prop_removed'; key: string; from: unknown }
  | { field: 'prop_changed'; key: string; from: unknown; to: unknown };

/** Which end of a changed relationship the reporting element occupies. */
export type RelDirection = 'outgoing' | 'incoming';

/**
 * A relationship triple present in exactly one of the two graphs,
 * attributed to a surviving endpoint (reported on each surviving end).
 */
export interface RelationshipDelta {
  kind: string;
  /** The element at the other end of the triple. */
  other: string;
  direction: RelDirection;
  /** True if the triple exists only in the NEW graph. */
  added: boolean;
}

/** Per-element record inside `GraphDiff.modified`. */
export interface ElementDiff {
  id: string;
  kind: string;
  changed_fields: FieldDelta[];
  relationship_deltas: RelationshipDelta[];
}

/** `GraphDiff` — element-level diff between two stored snapshots. */
export interface GraphDiff {
  /** Ids present only in the new graph (sorted). */
  added: string[];
  /** Ids present only in the old graph (sorted). */
  removed: string[];
  /** Ids in both with a field- or relationship-level delta (sorted). */
  modified: ElementDiff[];
}
