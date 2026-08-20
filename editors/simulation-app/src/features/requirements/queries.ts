/**
 * Requirements workbench data layer — the one hook the table builds on.
 *
 * Calls `sysml.workspace.requirement_rows` (B2) through the shared
 * transport. Rows come back document-ordered (file, then byte offset —
 * the outline contract); this layer never re-sorts them. Pagination is
 * walked to exhaustion here so the table always holds the full set —
 * the workbench is an outline/document surface, not an infinite scroll.
 */

import { useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import type { RequirementDetail, RequirementRow, RequirementRowsResult } from './types';

export const requirementKeys = {
  rows: ['requirement-rows'] as const,
  detail: (id: string) => ['requirement-detail', id] as const,
};

/** Backend page cap is 1000; walk cursors up to this many pages before
 *  giving up (10k rows — far beyond any current workspace; if we ever
 *  hit it, surface the truncation rather than silently dropping rows). */
const PAGE_LIMIT = 1000;
const MAX_PAGES = 10;

export interface RequirementRowsData {
  rows: RequirementRow[];
  revision: number;
  /** True if MAX_PAGES was exhausted with a live cursor still open. */
  truncated: boolean;
}

export async function fetchAllRequirementRows(
  includeVerificationOccurrences = false,
): Promise<RequirementRowsData> {
  const rows: RequirementRow[] = [];
  let cursor: string | null = null;
  let revision = 0;
  for (let page = 0; page < MAX_PAGES; page++) {
    const result: RequirementRowsResult = await httpPost<RequirementRowsResult>(
      '/api/command',
      {
        command: 'sysml.workspace.requirement_rows',
        params: {
          spec: {
            limit: PAGE_LIMIT,
            ...(cursor ? { cursor } : {}),
            // Verification check occurrences are bookkeeping rows, hidden
            // by default (steward ruling 2026-07-16); only send the flag
            // when revealing so the default wire shape stays minimal.
            ...(includeVerificationOccurrences
              ? { include_verification_occurrences: true }
              : {}),
          },
        },
      },
    );
    // A revision change mid-walk invalidates the cursor; restart from a
    // clean slate rather than stitching two different graphs together.
    if (result.cursor_invalidated) {
      rows.length = 0;
      cursor = null;
      revision = result.revision;
      continue;
    }
    rows.push(...result.rows);
    revision = result.revision;
    cursor = result.cursor;
    if (!cursor) return { rows, revision, truncated: false };
  }
  return { rows, revision, truncated: cursor !== null };
}

/**
 * All requirement rows for the loaded workspace, document-ordered.
 * Parked until a workspace root exists. Cache is invalidated by
 * `useLoadWorkspace` (rows carry element UUIDs, which a reload
 * regenerates — see `features/packages/queries.ts`).
 */
/** The verbatim source slice covering one element's declaration
 *  (`sysml.get_source`). Enabled only when a uri is supplied — the rail's
 *  source section passes null until expanded. */
export function useElementSource(uri: string | null, elementId: string) {
  return useQuery({
    queryKey: ['element-source', uri ?? '', elementId],
    enabled: uri !== null,
    queryFn: () =>
      httpPost<{ text: string; start: number; end: number } | null>('/api/command', {
        command: 'sysml.get_source',
        params: { uri, id: elementId },
      }),
  });
}

export function useRequirementRows(includeVerificationOccurrences = false) {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  return useQuery({
    // Keyed under the same 'requirement-rows' prefix so existing
    // prefix-invalidations (workspace reload) cover both variants.
    queryKey: [...requirementKeys.rows, includeVerificationOccurrences],
    queryFn: () => fetchAllRequirementRows(includeVerificationOccurrences),
    enabled: workspaceRoot !== null,
  });
}

/**
 * The evaluated contract of one requirement (B2.1/R18) — per-element read
 * for the selected row only; rows deliberately don't carry this payload.
 */
export function useRequirementDetail(elementId: string | null) {
  return useQuery({
    queryKey: requirementKeys.detail(elementId ?? ''),
    queryFn: () =>
      httpPost<RequirementDetail>('/api/command', {
        command: 'sysml.workspace.requirement_detail',
        params: { element_id: elementId },
      }),
    enabled: elementId !== null,
  });
}

/** One link-picker candidate (`sysml_query::ElementSummary` slice). */
export interface LinkTargetCandidate {
  id: string;
  name: string | null;
  qualified_name: string | null;
  kind: string;
}

/**
 * Picker candidates for the R5 link-add popovers (design §7.6):
 * kind-filtered, USER-AUTHORED elements only (the `user_authored` query
 * filter — stdlib internals are never link targets). Fetch is deferred
 * until a picker opens (`kinds === null` while closed).
 */
export function useLinkTargetCandidates(kinds: string[] | null) {
  return useQuery({
    queryKey: ['link-target-candidates', kinds],
    enabled: kinds !== null,
    queryFn: async () => {
      const result = await httpPost<{ rows: LinkTargetCandidate[] }>('/api/command', {
        command: 'sysml.query',
        params: {
          uri: '__workspace__',
          spec: {
            filter: {
              type: 'all',
              filters: [{ type: 'kind', kinds }, { type: 'user_authored' }],
            },
            projection: 'summary',
            limit: 1000,
          },
        },
      });
      return result.rows;
    },
  });
}
