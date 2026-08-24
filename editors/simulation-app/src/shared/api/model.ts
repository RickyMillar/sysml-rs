/**
 * Model domain API calls.
 *
 * Thin wrappers around httpPost for model introspection endpoints.
 * Session calls live in features/sessions/queries.ts and mutations.ts.
 */

import { httpGet, httpPost } from './http';

/**
 * The workspace-scope URI sentinel. The backend resolves it to the merged
 * workspace graph rather than a single file (see `sysml-service` scope
 * handling). The app only ever loads via `sysml.load_workspace`, so every
 * model-scope read addresses the workspace with this constant.
 */
export const WORKSPACE_URI = '__workspace__';

// ── Generic command dispatch ────────────────────────────────────────

async function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Models ──────────────────────────────────────────────────────────

/**
 * The workspace/tree APIs identify source files with canonical `file://` URIs,
 * while the REST `/files` endpoint deliberately takes a filesystem path. Keep
 * the conversion at that transport boundary: LSP and `sysml.get_source` still
 * require the canonical URI, so callers must not normalise their selection.
 */
export function fileUriToLocalPath(uriOrPath: string): string {
  if (!uriOrPath.startsWith('file://')) return uriOrPath;

  try {
    const url = new URL(uriOrPath);
    // A remote authority is not a local filesystem path. Leave it untouched
    // so the backend can report an honest unsupported/missing-path error.
    if (url.protocol !== 'file:' || (url.host && url.host !== 'localhost')) return uriOrPath;

    const path = decodeURIComponent(url.pathname);
    // `file:///C:/…` is represented as `/C:/…` by URL.pathname. The backend
    // expects the native Windows path; POSIX paths retain their leading slash.
    return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path;
  } catch {
    // Keep malformed inputs unchanged — the endpoint's error includes the
    // original value and is more actionable than a client-side parse failure.
    return uriOrPath;
  }
}

export async function loadFile(uriOrPath: string): Promise<{ uri: string; source: string }> {
  return httpPost('/files', { path: fileUriToLocalPath(uriOrPath) });
}

// ── Table payload types (mirror crates/lang/sysml-diagram/src/tmodel/) ─

export type TableColumnKind = 'text' | 'number' | 'boolean' | 'symbol';

export interface TableColumn {
  id: string;
  label: string;
  kind: TableColumnKind;
}

export interface TableCell {
  display: string;
  cssClasses?: string[];
  elementId?: string;
}

export interface TableRow {
  id: string;
  cells: TableCell[];
}

export interface TableModel {
  title?: string;
  /** Generator tag (e.g. `"traceability_matrix"`). */
  kind?: string;
  columns: TableColumn[];
  rows: TableRow[];
}

// ── Geometry payload types (mirror crates/lang/sysml-diagram/src/gmodel/) ─

export interface Viewport {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export type GeometryPrimitive = {
  shape: 'rect';
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  label?: string;
  cssClasses?: string[];
  elementId?: string;
};

export interface GeometryModel {
  title?: string;
  /** Generator tag (e.g. `"spatial_layout"`). */
  kind?: string;
  viewport?: Viewport;
  primitives: GeometryPrimitive[];
}

// ── Tree payload types (mirror crates/lang/sysml-diagram/src/tree/) ──

export interface TreeNode {
  id: string;
  elementId?: string;
  label: string;
  /** SysML element kind name (e.g. `"PartUsage"`). */
  kindLabel?: string;
  stereotype?: string;
  cssClasses?: string[];
  children?: TreeNode[];
}

export interface TreeModel {
  title?: string;
  /** Generator tag (e.g. `"containment_tree"`). */
  kind?: string;
  roots: TreeNode[];
}

// ── Model Introspection ─────────────────────────────────────────────

export interface QueryResult<T = unknown> {
  rows: T;
  total_estimate?: number | null;
  cursor?: string | null;
  cursor_invalidated?: boolean;
  revision?: number;
  cache_status?: 'uncached' | 'hit' | 'miss';
}

export async function queryModel<T = unknown>(
  uri: string,
  spec: Record<string, unknown>,
): Promise<QueryResult<T>> {
  return cmd('sysml.query', { uri, spec });
}

export async function getStats(uri: string): Promise<Record<string, unknown>> {
  return cmd('sysml.stats', { uri });
}

/**
 * Fetch the renderer-agnostic ViewModel (Bucket 1.7, re-keyed F2) for a DECLARED
 * view — scene + design tokens + text-map + interactions + frame, scoped by the
 * view's Expose/filter memberships. `viewUsageId` is the ElementId of a
 * ViewUsage/ViewDefinition (from `sysml.views.list`). Consumed by SvgCanvas.
 */
export async function getViewModel(
  uri: string,
  viewUsageId: string,
  expandedIds: string[] = [],
): Promise<Record<string, unknown>> {
  return cmd('sysml.diagram.viewmodel', {
    uri,
    view_usage_id: viewUsageId,
    expanded_ids: expandedIds,
  });
}

/** Fetch the per-tick simulation overlay for a session + declared view. Session
 *  state (not salsa-cached); poll while a session runs. `view_usage_id` must
 *  match the viewmodel call so the scene ids align. */
export async function getSimOverlay(
  sessionId: string,
  viewUsageId: string,
  expandedIds: string[] = [],
): Promise<Record<string, unknown>> {
  return cmd('sysml.diagram.sim_overlay', {
    session_id: sessionId,
    view_usage_id: viewUsageId,
    expanded_ids: expandedIds,
  });
}

/** Fetch the per-run verdict overlay for a session + declared view. Session
 *  state (not salsa-cached); poll while a session runs. `view_usage_id` must
 *  match the viewmodel call so the scene ids align. Companion verdict sidecar to
 *  `getSimOverlay`. */
export async function getVerdictOverlay(
  sessionId: string,
  viewUsageId: string,
  expandedIds: string[] = [],
): Promise<Record<string, unknown>> {
  return cmd('sysml.diagram.verdict_overlay', {
    session_id: sessionId,
    view_usage_id: viewUsageId,
    expanded_ids: expandedIds,
  });
}

/** Fetch the static diagnostics overlay (validation-diagnostic badges: worst
 *  severity + tooltip detail) for a declared view's scene. Workspace state —
 *  needs no session. `view_usage_id`/`expanded_ids` must match the viewmodel
 *  call so the scene ids align. */
export async function getDiagnosticOverlay(
  viewUsageId: string,
  expandedIds: string[] = [],
): Promise<Record<string, unknown>> {
  return cmd('sysml.diagram.diagnostic_overlay', {
    view_usage_id: viewUsageId,
    expanded_ids: expandedIds,
  });
}

/** Row shape returned by {@link findElements} — the backend's
 *  `ElementSummary` projection. `qualified_name` is the full ownership
 *  path (`Pkg::Sub::Name`); `owner_id` the owning element. Both absent
 *  when the element (or an ancestor) is unnamed / unowned. */
export interface FoundElement {
  id: string;
  name: string | null;
  kind: string;
  qualified_name?: string | null;
  owner_id?: string | null;
}

export async function findElements(
  uri: string,
  kind?: string,
  namePattern?: string,
): Promise<FoundElement[]> {
  const filters: Array<Record<string, unknown>> = [];
  if (kind) {
    filters.push({ type: 'kind', kinds: [kind] });
  }
  if (namePattern !== undefined) {
    filters.push({
      type: 'name_match',
      name_match: { contains: namePattern, ci: false },
    });
  }
  const filter =
    filters.length === 0
      ? { type: 'all', filters: [] }
      : filters.length === 1
        ? filters[0]!
        : { type: 'all', filters };
  // `summary` projection: lighter rows than `elements` (no props /
  // relationship payloads) AND carries qualified_name + owner_id —
  // the structural grouping keys run-target consumers need.
  const result = await queryModel<FoundElement[]>(uri, {
    filter,
    projection: 'summary',
    limit: 1000,
  });
  return result.rows;
}
