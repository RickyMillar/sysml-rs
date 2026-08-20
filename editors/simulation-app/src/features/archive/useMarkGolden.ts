/**
 * useMarkGolden — pair of react-query mutations for pinning / unpinning
 * an archived session as "golden" (reference run) (R4.1).
 *
 * Backend commands:
 *   - `sysml.sessions.archive.mark_golden`   ({ id, label }) → { ok: bool }
 *   - `sysml.sessions.archive.unmark_golden` ({ id })        → { ok: bool }
 *
 * Both mutations invalidate every archive list (the `is_golden` flag
 * feeds the "only golden" toggle + the golden-star indicator), and the
 * matching detail key so a preview pane picks up the new state.
 */

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { archiveKeys } from './useArchiveList';

interface MarkGoldenArgs {
  id: string;
  label: string;
}

interface UnmarkGoldenArgs {
  id: string;
}

interface AckResponse {
  ok: boolean;
}

function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

/**
 * Pin an archived session as "golden" with a human label. The label is
 * stored on the wire as `golden_label`; the `is_golden` flag flips to
 * `true` on success.
 */
export function useMarkGolden() {
  const qc = useQueryClient();
  return useMutation<AckResponse, Error, MarkGoldenArgs>({
    mutationFn: (args) =>
      cmd<AckResponse>('sysml.sessions.archive.mark_golden', {
        id: args.id,
        label: args.label,
      }),
    onSuccess: (_data, variables) => {
      void qc.invalidateQueries({ queryKey: archiveKeys.lists() });
      void qc.invalidateQueries({ queryKey: archiveKeys.detail(variables.id) });
    },
  });
}

/**
 * Clear the "golden" pin from an archived session. Idempotent — a
 * session that is already unmarked returns `{ ok: true }` without
 * mutating state server-side.
 */
export function useUnmarkGolden() {
  const qc = useQueryClient();
  return useMutation<AckResponse, Error, UnmarkGoldenArgs>({
    mutationFn: (args) =>
      cmd<AckResponse>('sysml.sessions.archive.unmark_golden', {
        id: args.id,
      }),
    onSuccess: (_data, variables) => {
      void qc.invalidateQueries({ queryKey: archiveKeys.lists() });
      void qc.invalidateQueries({ queryKey: archiveKeys.detail(variables.id) });
    },
  });
}
