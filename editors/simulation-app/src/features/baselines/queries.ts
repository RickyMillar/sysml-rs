/**
 * Baselines + suspect data layer (Phase 7.5 v1.5, B3 wiring).
 *
 * Store commands are project-keyed. The FE supplies the ProjectId
 * explicitly (steward ruling: no server-side derivation, no second
 * convention) — v1.5 convention: the workspace directory's basename.
 * Deterministic and human-legible; when a workspace-manifest command
 * exposes `project.name` / IRI, that becomes the preferred source
 * (manifest identity is machine-independent; a directory name is only
 * checkout-stable). The store is in-memory per server process today, so
 * baselines do not survive a backend restart — surfaced in the UI copy,
 * not silently.
 *
 * Freshness: `store.diff`/`requirement_suspects` compare STORED
 * snapshots (`to` omitted = latest stored commit, NOT the live
 * workspace). Every suspect fetch therefore runs
 * `sysml.store.save_workspace` first — content-addressed commit ids make
 * the unchanged case free (same digest → same commit, nothing minted).
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import type { BaselineMeta, SnapshotMeta } from './types';
import { suspectsById, type SuspectRecord, type SuspectRecordWire } from './suspect';

export const baselineKeys = {
  list: (project: string) => ['baselines', project] as const,
  suspects: (project: string, baseline: string) => ['suspects', project, baseline] as const,
};

/** v1.5 ProjectId convention — see module doc. */
export function projectIdForWorkspace(workspaceRoot: string): string {
  const trimmed = workspaceRoot.replace(/[/\\]+$/, '');
  const segments = trimmed.split(/[/\\]/);
  return segments[segments.length - 1] || trimmed;
}

function command<T>(name: string, params: Record<string, unknown>): Promise<T> {
  return httpPost<T>('/api/command', { command: name, params });
}

export function useProjectId(): string | null {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  return workspaceRoot ? projectIdForWorkspace(workspaceRoot) : null;
}

/** Snapshot the live workspace into the store (idempotent server-side). */
export async function saveWorkspaceSnapshot(project: string): Promise<SnapshotMeta> {
  return command<SnapshotMeta>('sysml.store.save_workspace', { project });
}

export function useBaselines() {
  const project = useProjectId();
  return useQuery({
    queryKey: baselineKeys.list(project ?? ''),
    enabled: project !== null,
    queryFn: async () => {
      // Ensure the project exists in the store before listing — a fresh
      // backend has never seen this workspace.
      await saveWorkspaceSnapshot(project as string);
      return command<BaselineMeta[]>('sysml.store.baseline.list', { project });
    },
  });
}

export function useCreateBaseline() {
  const project = useProjectId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) => {
      if (!project) throw new Error('no workspace loaded');
      // Baseline the CURRENT workspace: snapshot first, then pin it.
      await saveWorkspaceSnapshot(project);
      return command<BaselineMeta>('sysml.store.baseline.create', { project, name });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['baselines'] });
      void queryClient.invalidateQueries({ queryKey: ['suspects'] });
    },
  });
}

/**
 * Suspect records vs the selected baseline, keyed by requirement id.
 * `rowsRevision` ties freshness to the loaded row set: a workspace
 * reload bumps the revision and refetches (save_workspace picks up the
 * new content; unchanged content is a no-op server-side).
 */
export function useSuspects(baseline: string | null, rowsRevision: number | null) {
  const project = useProjectId();
  return useQuery({
    queryKey: [
      ...baselineKeys.suspects(project ?? '', baseline ?? ''),
      rowsRevision,
    ] as const,
    enabled: project !== null && baseline !== null,
    queryFn: async (): Promise<Map<string, SuspectRecord>> => {
      await saveWorkspaceSnapshot(project as string);
      const wires = await command<SuspectRecordWire[]>(
        'sysml.workspace.requirement_suspects',
        { project, from: baseline },
      );
      // A row covered by a live (non-superseded) clearing attestation is
      // not suspect for display — the backend keeps it in the response
      // for transparency; the flag map drops it.
      return suspectsById(wires.filter((w) => w.cleared_by == null));
    },
  });
}
