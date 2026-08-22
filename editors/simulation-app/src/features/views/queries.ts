/**
 * Query hooks for user-authored ViewUsage / ViewDefinition rendering
 * (Phase 5 backend).
 *
 * Pairs with the backend commands `sysml.query` and
 * `sysml.views.render` (REST: POST /api/command and GET /models/:uri/views/:view_id/render).
 */

import { useMutation, useQuery } from '@tanstack/react-query';
import { httpGet, httpPost } from '@/shared/api/http';
import { queryModel } from '@/shared/api/model';

// ── Types — mirror sysml_core::view_index ─────────────────────────────

/** Source span carried with each ViewSummary. */
export interface ViewSpan {
  file: string;
  start: number;
  end: number;
  line?: number | null;
  col?: number | null;
}

/** One Expose membership child of a view. */
export interface ExposeRef {
  /** ElementId of the Expose membership itself (UUID string). */
  id: string;
  /** True for `expose Foo::*`, false for `expose Foo`. */
  is_namespace: boolean;
  /** Qualified name written in source, or null when missing. */
  qualified_name: string | null;
  /** Resolved target ElementId, or null if it didn't resolve. */
  exposed_element_id: string | null;
}

export interface RenderingRef {
  id: string;
  name: string | null;
}

/** A discovered ViewUsage / ViewDefinition. */
export interface ViewSummary {
  id: string;
  name: string | null;
  kind: string;
  exposed: ExposeRef[];
  renderings: RenderingRef[];
  /** ElementFilterMembership ids attached to the view. */
  filters: string[];
  source_span: ViewSpan | null;
}

interface QueryElementSummary {
  id: string;
  name: string | null;
  qualified_name: string | null;
  kind: string;
  owner_id: string | null;
  source_span: ViewSpan | null;
  expansion?:
    | { kind: 'view'; data: ViewSummary }
    | null;
}

function viewSummaryFromQueryRow(row: QueryElementSummary): ViewSummary | null {
  if (row.expansion?.kind === 'view') return row.expansion.data;
  return null;
}

/**
 * Extract the absolute file path from a view's source span, stripping
 * the `file://` scheme the backend emits in `source_span.file`. The
 * REST `sysml.get_source` command + `loadedFiles` store both key by
 * raw path. Returns `null` when the view has no span (synthesised or
 * library element with no recorded source).
 *
 * Exported so panels (Bug B fix — ViewsPanel passes this as
 * `previewUri` per row) don't reinvent the strip.
 */
export function fileUriOf(view: ViewSummary): string | null {
  const raw = view.source_span?.file;
  if (!raw) return null;
  return raw.startsWith('file://') ? raw.slice('file://'.length) : raw;
}

/**
 * True when the view's source span lives under the bundled SysML
 * standard library (`/libraries/standard/`). These are imported
 * `ViewDefinition` *types* (Sequence, Grid, Interconnection, …), not
 * user-authored views — the Phase 5 spec scope was "list user views
 * the workspace declares", so we filter library defs out FE-side.
 *
 * Bug C1 fix (Apr 2026): the broad `{ type: 'view' }` query matches
 * every ViewDefinition in the elaborated graph including stdlib
 * imports. That dumped 24 noisy rows on espresso-production-cell with empty
 * `exposed` / `renderings` lists. Bug C2 (backend follow-up) will add
 * `user_authored: true` to `sysml.query`; until then this predicate is
 * the FE workaround.
 */
export function isStdlibView(view: ViewSummary): boolean {
  const file = view.source_span?.file ?? '';
  return file.includes('/libraries/');
}

// ── Query keys ────────────────────────────────────────────────────────

export const viewKeys = {
  list: (uri: string) => ['views', 'list', uri] as const,
  byViewpoint: (uri: string, viewpointId: string) =>
    ['views', 'by-viewpoint', uri, viewpointId] as const,
  viewpointSearch: (uri: string, q: string) =>
    ['views', 'viewpoint-search', uri, q] as const,
  render: (uri: string, viewId: string, expanded: string[]) =>
    ['views', 'render', uri, viewId, ...expanded.slice().sort()] as const,
};

// ── Viewpoint summaries (for the picker) ──────────────────────────────

export interface ViewpointPickerEntry {
  id: string;
  name: string | null;
  qualified_name: string | null;
  kind: string;
}

// ── Hooks ─────────────────────────────────────────────────────────────

/**
 * List user-authored views for a loaded URI.
 *
 * `uri = "__workspace__"` lists every view across the merged workspace
 * graph. A per-file URI narrows to views whose source span lives in
 * that file.
 */
export function useViewsList(uri: string | null) {
  return useQuery({
    queryKey: viewKeys.list(uri ?? ''),
    queryFn: async () => {
      if (!uri) return [] as ViewSummary[];
      const result = await queryModel<QueryElementSummary[]>(uri, {
        filter: { type: 'view', viewpoint_id: null },
        projection: 'summary_expand',
        sort: [{ field: 'name', dir: 'asc' }],
        limit: 1000,
      });
      return result.rows
        .map(viewSummaryFromQueryRow)
        .filter((v): v is ViewSummary => v !== null)
        .filter((v) => !isStdlibView(v));
    },
    enabled: !!uri,
  });
}

/**
 * List views attached to a specific viewpoint.
 *
 * Separate hook + cache key from `useViewsList` so switching back to
 * "All views" hits the cached unfiltered fetch instead of refetching.
 */
export function useViewsByViewpoint(uri: string | null, viewpointId: string | null) {
  return useQuery({
    queryKey: viewKeys.byViewpoint(uri ?? '', viewpointId ?? ''),
    queryFn: async () => {
      if (!uri || !viewpointId) return [] as ViewSummary[];
      const result = await queryModel<QueryElementSummary[]>(uri, {
        filter: { type: 'view', viewpoint_id: viewpointId },
        projection: 'summary_expand',
        sort: [{ field: 'name', dir: 'asc' }],
        limit: 1000,
      });
      return result.rows
        .map(viewSummaryFromQueryRow)
        .filter((v): v is ViewSummary => v !== null)
        .filter((v) => !isStdlibView(v));
    },
    enabled: !!uri && !!viewpointId,
  });
}

/**
 * Typeahead search for viewpoint elements. `query` is expected to be
 * already-debounced by the caller.
 *
 * Empty query → list all viewpoints (small workspaces). The hook's
 * react-query key includes the debounced text, so each distinct value
 * gets its own cache slot.
 */
export function useViewpointSearch(uri: string | null, query: string) {
  return useQuery({
    queryKey: viewKeys.viewpointSearch(uri ?? '', query),
    queryFn: async () => {
      if (!uri) return [] as ViewpointPickerEntry[];
      const filters: Array<Record<string, unknown>> = [
        { type: 'kind', kinds: ['ViewpointDefinition', 'ViewpointUsage'] },
      ];
      if (query.length > 0) {
        filters.push({
          type: 'name_match',
          name_match: { contains: query, ci: true },
        });
      }
      const filter =
        filters.length === 1 ? filters[0]! : { type: 'all', filters };
      const result = await queryModel<ViewpointPickerEntry[]>(uri, {
        filter,
        projection: 'summary',
        sort: [{ field: 'name', dir: 'asc' }],
        limit: 100,
      });
      return result.rows;
    },
    enabled: !!uri,
  });
}

/**
 * Render a specific authored view as an SModel diagram payload.
 *
 * Returns the raw JSON object the renderer consumes — callers are
 * expected to drop it into `useWorkspaceStore.setDiagramPayload(...)`.
 */
/**
 * Build a `view scratch : InterconnectionView { expose ...; }` source
 * snippet from the qualified names (or ids) of selected elements.
 *
 * Bucket 5.A2 — wraps the backend's `sysml.views.create_scratch` /
 * `POST /views/scratch` command. Returns plain text: the snippet the
 * user can paste into their `.sysml` file. The backend takes care of
 * spec-correct shaping (`view scratch : InterconnectionView` per the
 * roadmap).
 */
export function useCreateScratchView() {
  return useMutation({
    // Dispatch via the canonical command endpoint like every other call
    // — the bespoke `POST /views/scratch` REST route isn't in the vite
    // proxy (create-view v2 hit a 404 here, 2026-07-15) and there is no
    // reason for this one command to have its own transport path.
    mutationFn: (exposeRefs: string[]): Promise<string> =>
      httpPost<string>('/api/command', {
        command: 'sysml.views.create_scratch',
        params: { expose: exposeRefs },
      }),
  });
}

// `useViewRender` (the legacy SModel `/render` fetch hook) was removed in 3.12:
// every view family — graph AND non-graph — now renders from the single
// `sysml.diagram.viewmodel` query. `SelectedViewRenderer` reads `vm.non_graph`
// for Table/Tree/Geometry; SvgCanvas reads the scene for graph views. The
// `/render` REST command itself remains for CLI/LSP/MCP SGraph consumers.
