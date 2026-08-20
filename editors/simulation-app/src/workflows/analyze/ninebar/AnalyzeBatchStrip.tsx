/**
 * AnalyzeBatchStrip — the shared bottom-strip content for the flag-on
 * Analyze method bodies (ninebar Phase 5).
 *
 * Plan §0: the strip holds "only what this model at this moment demands"
 * — for a batch study that is the live batch lifecycle: a `<Ninebar/>`
 * while children run (a legitimate live measure, per the phase brief),
 * the per-status child counts (pending/running/complete/failed), and the
 * promote-to-Compare handoff once ≥2 children have real sessions.
 *
 * Child "failed" is an EXECUTION severity (`--severity-error`), while a
 * child whose verdicts contain a fail is a VERDICT (`--verdict-fail`) —
 * two different ladders, deliberately kept apart (brief §4).
 */

import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { Ninebar } from '@/components/Ninebar';
import { EvaluationModeBadge } from '@/components/EvaluationModeBadge';
import { useCompareStore } from '@/workflows/compare/useCompareStore';
import type { BatchStatus, ChildDescriptor } from '@/engine/types';

export interface AnalyzeBatchStripProps {
  /** Short method label, e.g. "Sweep" / "Monte Carlo". */
  methodLabel: string;
  status: BatchStatus;
  children: ChildDescriptor[];
  /**
   * Fallback progress for runners that report only completed/total
   * without streaming per-child descriptors (trade study). Ignored when
   * `children` is non-empty.
   */
  progress?: { completed: number; total: number } | null;
}

export function countByStatus(children: readonly ChildDescriptor[]) {
  const counts = { pending: 0, running: 0, complete: 0, failed: 0 };
  for (const c of children) counts[c.status] += 1;
  return counts;
}

/** Children with a fail verdict — the verdict ladder, not execution. */
export function failingChildren(children: readonly ChildDescriptor[]): ChildDescriptor[] {
  return children.filter((c) => (c.verdicts ?? []).some((v) => v.verdict === 'fail'));
}

/**
 * Pick the sessions promoted to Compare: failing children first (the
 * interesting ones), then remaining completed children, clamped to
 * Compare's 6-pick window by its own store.
 */
export function promotedSessionIds(children: readonly ChildDescriptor[]): string[] {
  const failing = failingChildren(children);
  const rest = children.filter((c) => c.status === 'complete' && !failing.includes(c));
  return [...failing, ...rest]
    .map((c) => c.session_id)
    .filter((id): id is string => typeof id === 'string' && id.length > 0);
}

export function AnalyzeBatchStrip({ methodLabel, status, children, progress = null }: AnalyzeBatchStripProps) {
  const navigate = useNavigate();
  const setPickedSessionIds = useCompareStore((s) => s.setPickedSessionIds);

  const counts = useMemo(() => countByStatus(children), [children]);
  const failing = useMemo(() => failingChildren(children).length, [children]);
  const promotable = useMemo(() => promotedSessionIds(children), [children]);
  const total = children.length;
  const isRunning = status.kind === 'running';

  const promote = () => {
    if (promotable.length < 2) return;
    setPickedSessionIds(promotable);
    navigate('/run/compare');
  };

  return (
    <div
      data-testid="analyze-batch-strip"
      className="flex items-center gap-4 px-3 h-full"
      style={{ fontSize: 11 }}
    >
      <span className="mono-text" style={{ color: 'var(--text-muted)' }}>{methodLabel}</span>

      {isRunning && (
        <span className="flex items-center gap-2" style={{ color: 'var(--accent-fg)' }}>
          <Ninebar size={14} label={`${methodLabel} batch running`} />
          {status.kind === 'running' ? `${status.completed} of ${total || status.completed + status.running} complete` : 'running'}
        </span>
      )}

      {status.kind === 'failed' && (
        <span data-testid="analyze-strip-failed" style={{ color: 'var(--severity-error)' }}>
          batch failed — {status.reason}
        </span>
      )}

      {total === 0 && progress && status.kind !== 'pending' ? (
        <span className="mono-text" data-testid="analyze-strip-progress" style={{ color: 'var(--text-secondary)' }}>
          {progress.completed} / {progress.total} complete
        </span>
      ) : total === 0 && status.kind === 'pending' ? (
        <span style={{ color: 'var(--text-muted)' }}>No batch yet</span>
      ) : total === 0 ? (
        <span style={{ color: 'var(--text-muted)' }}>—</span>
      ) : (
        <span className="flex items-center gap-3 mono-text" data-testid="analyze-strip-counts">
          <span style={{ color: 'var(--text-muted)' }}>{counts.pending} pending</span>
          <span style={{ color: 'var(--accent-fg)' }}>{counts.running} running</span>
          <span style={{ color: 'var(--text-secondary)' }}>{counts.complete} complete</span>
          {counts.failed > 0 && (
            <span style={{ color: 'var(--severity-error)' }}>{counts.failed} failed</span>
          )}
          {failing > 0 && (
            <>
              <span data-testid="analyze-strip-failing" style={{ color: 'var(--verdict-fail)' }}>
                ✗ {failing} failing
              </span>
              {/* B10 §2.1a(d): evaluation_mode is a BINDING label on every
                  verdict-bearing surface — legible next to the verdict.
                  Batch children ARE session executions, so their verdicts
                  are trajectory-computed by construction (the archive
                  stamps ArchivedVerdict.evaluation_mode = Trajectory). */}
              <EvaluationModeBadge mode="trajectory" size="compact" testId="analyze-strip-mode" />
            </>
          )}
          <span style={{ color: 'var(--text-muted)' }}>· {total} total</span>
        </span>
      )}

      <button
        type="button"
        data-testid="analyze-promote-compare"
        disabled={promotable.length < 2}
        onClick={promote}
        title={
          promotable.length < 2
            ? 'Needs at least two children with sessions'
            : `Open Compare with ${Math.min(promotable.length, 6)} child sessions (failing first)`
        }
        style={{
          marginLeft: 'auto',
          padding: '2px 10px',
          borderRadius: 4,
          fontSize: 11,
          cursor: promotable.length < 2 ? 'not-allowed' : 'pointer',
          background: 'transparent',
          color: promotable.length < 2 ? 'var(--text-muted)' : 'var(--accent-fg)',
          border: '1px solid var(--border-hairline)',
        }}
      >
        ↳ Compare
      </button>
    </div>
  );
}
