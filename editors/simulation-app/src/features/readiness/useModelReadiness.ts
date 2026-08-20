/**
 * useModelReadiness — aggregates diagnostics + dependency status +
 * capabilities into one `ReadinessSummary` (ninebar Phase 1.5).
 *
 * Reuses existing query hooks wherever they exist:
 *   - `sysml.diagnostics`             -> `useDiagnostics` (features/diagnostics)
 *   - `sysml.workspace.capabilities`  -> `useWorkspaceStore.capabilities`
 *     (already fetched + cached during workspace hydrate,
 *     `features/packages/queries.ts::hydrateWorkspaceStore`; no query
 *     hook existed for it, but a *cache* did, so no new fetch is added)
 *   - `sysml.dependency.status`       -> `useDependencyStatus` (new hook,
 *     added to `features/packages/queries.ts` — no FE binding existed
 *     beyond the debug drawer's ad-hoc inline query)
 *
 * `workspaceRoot` + `uris` are read the same way
 * `DiagnosticsPanel`/`DebugDrawer` already do (`useWorkspaceUIStore` for
 * the root, `useWorkspaceUris` for the cached URI list) so this hook
 * doesn't invent a second notion of "is a workspace loaded".
 */
import { useMemo } from 'react';
import { useDiagnostics } from '@/features/diagnostics/useDiagnostics';
import { useDependencyStatus, useWorkspaceUris } from '@/features/packages/queries';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { aggregateReadiness } from './aggregate';
import type { ReadinessSummary } from './types';

export function useModelReadiness(): ReadinessSummary {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const uris = wsData?.uris ?? [];

  const diagnosticsQuery = useDiagnostics({ uris });
  const dependencyStatusQuery = useDependencyStatus(workspaceRoot ? [workspaceRoot] : []);
  const capabilities = useWorkspaceStore((s) => s.capabilities);

  const hasWorkspace = !!workspaceRoot;
  const diagnosticEntries = diagnosticsQuery.entries;
  const dependencyStatus = dependencyStatusQuery.data;

  return useMemo(
    () =>
      aggregateReadiness({
        hasWorkspace,
        diagnostics: diagnosticEntries,
        dependencyStatus,
        capabilities,
      }),
    [hasWorkspace, diagnosticEntries, dependencyStatus, capabilities],
  );
}
