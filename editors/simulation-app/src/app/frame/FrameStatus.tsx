/**
 * FrameStatus — the quiet mono model-status on the frame's trailing edge
 * (ninebar screenshot-comparison ruling C, 2026-07-14 — "the frame's
 * right side gets the demo's quiet mono status: model @ rev · session").
 *
 * The demo prints `breaker_trip_unit @ a41c9f · dap 7f3a` — model name, then
 * source revision, then the session context. We render the model name
 * and, when it's known, the model revision (`@ <rev>`); the revision
 * comes from the §6.2 provenance billet (model-revision hash captured on
 * `sessions.create`), which is a backend item not yet landed, so the
 * `@ <rev>` segment is simply omitted until it exists rather than shown
 * as a placeholder. The `· session` segment is already carried by the
 * interactive `SessionSwitcherChip` beside it, so this element owns only
 * the model identity to avoid duplicating the session control.
 *
 * Read-only, muted, monospace — never a selection/active surface (the
 * frame carries no amber).
 */

import type { CSSProperties } from 'react';
import { useWorkspaceUIStore } from '@/features/workspace/store';

/** Derive the model label from the workspace root path's last segment. */
function modelLabel(workspaceRoot: string | null | undefined): string | null {
  if (!workspaceRoot) return null;
  const segs = workspaceRoot.split(/[/\\]/).filter(Boolean);
  return segs[segs.length - 1] ?? workspaceRoot;
}

export function FrameStatus() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const model = modelLabel(workspaceRoot);
  if (!model) return null;

  return (
    <span data-testid="frame-status" style={statusStyle} title={workspaceRoot ?? undefined}>
      {model}
    </span>
  );
}

const statusStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, monospace)',
  fontSize: 'var(--text-xs)',
  color: 'var(--text-muted)',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  maxWidth: 260,
};
