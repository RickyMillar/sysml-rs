/**
 * CompareWorkflowNinebar — the flag-on Compare surface (ninebar
 * Phase 6): the diff canvas hero under ONE shared playhead.
 *
 * Column layout: playhead bar (40px) → honesty banners
 * (`history_truncated`) → [session/fork pick rail | diff canvas].
 * No resident config panels — the DoD's "two side-by-side recoloured
 * Run views" shape is exactly what this is not: one canvas, one
 * playhead, diff as its own semantic layer.
 *
 * Playhead anchoring (contract F8): when the picked set gains a
 * forked session, the playhead SNAPS to its `fork_point_tick` — the
 * earliest tick a fork comparison can diverge — and the marker strip
 * carries the anchor persistently.
 *
 * The legacy (flag-off) body lives in `CompareWorkflow.tsx` verbatim
 * and dies with the legacy shell in Phase 8.
 */

import { useEffect, useMemo, useRef } from 'react';
import { autoPickVariables } from '../selectors';
import { useCompareStore } from '../useCompareStore';
import { sessionStroke } from './channels';
import {
  useCompareSeries,
  usePickedSummaries,
  useTimelineDiff,
  useUnionVariableNames,
} from './compareData';
import { DiffCanvas } from './DiffCanvas';
import { EnsembleStrip, GoldenStrip, TwoDesignStrip } from './ModeStrips';
import { PlayheadBar } from './PlayheadBar';
import { SessionPickRail } from './SessionPickRail';

/** The Phase 6 mode switch (plan: "modes as a mode switch, not extra
 *  panels"). `diff` is the bare canvas; the others add a contextual
 *  strip above it, reusing the R4.3 mode math. */
const MODES = [
  { id: 'diff', label: 'Diff' },
  { id: 'ensemble', label: 'Ensemble' },
  { id: 'golden', label: 'Golden' },
  { id: 'two-design', label: 'Two-design' },
] as const;
type CompareNbMode = (typeof MODES)[number]['id'];

/** Cap on how many variables are FETCHED in auto mode (not a display
 *  cap — the picker lists every recorded name; picking one fetches
 *  it). Announced in the picker row whenever it truncates. */
const AUTO_FETCH_CAP = 24;
/** Auto-picked variables shown before the user narrows. */
const AUTO_SHOW = 6;

export function CompareWorkflowNinebar() {
  const pickedIds = useCompareStore((s) => s.pickedSessionIds);
  const sharedTick = useCompareStore((s) => s.sharedTick);
  const setSharedTick = useCompareStore((s) => s.setSharedTick);
  const pickedVars = useCompareStore((s) => s.pickedVariables);
  const setPickedVars = useCompareStore((s) => s.setPickedVariables);
  const activeModeId = useCompareStore((s) => s.activeModeId);
  const setActiveModeId = useCompareStore((s) => s.setActiveModeId);
  const mode: CompareNbMode = (MODES.find((m) => m.id === activeModeId)?.id ??
    'diff') as CompareNbMode;

  const { summaries, missingIds } = usePickedSummaries(pickedIds);

  // Pair mode: exactly two RESOLVED picks → the backend's exact diff.
  const isPair = summaries.length === 2;
  const { data: pairDiff } = useTimelineDiff(
    isPair ? summaries[0] : null,
    isPair ? summaries[1] : null,
  );

  const { names: unionNames, namesBySession } = useUnionVariableNames(summaries);

  // Which variables to FETCH: the user's explicit picks, else the
  // auto-fetch window over the union.
  const fetchVars = useMemo(() => {
    if (pickedVars !== null) return pickedVars.filter((v) => unionNames.includes(v));
    return unionNames.slice(0, AUTO_FETCH_CAP);
  }, [pickedVars, unionNames]);

  const { samplesByVar, maxTick } = useCompareSeries(summaries, fetchVars);

  // Which variables to SHOW: explicit picks verbatim; auto = top-N by
  // cross-session variance over the fetched window.
  const shownVars = useMemo(() => {
    if (pickedVars !== null) return fetchVars;
    return autoPickVariables(samplesByVar, AUTO_SHOW);
  }, [pickedVars, fetchVars, samplesByVar]);

  // F8 anchor: first picked fork's fork_point_tick. Snap the playhead
  // there when the picked-fork set changes (marker stays always).
  const forkAnchorTick = useMemo(() => {
    const fork = summaries.find((s) => s.fork_point_tick !== null);
    return fork?.fork_point_tick ?? null;
  }, [summaries]);
  const lastAnchorKey = useRef<string>('');
  useEffect(() => {
    const key = summaries
      .filter((s) => s.fork_point_tick !== null)
      .map((s) => s.id)
      .join(',');
    if (key && key !== lastAnchorKey.current && forkAnchorTick !== null) {
      setSharedTick(forkAnchorTick);
    }
    lastAnchorKey.current = key;
  }, [summaries, forkAnchorTick, setSharedTick]);

  const ready = summaries.length >= 2;

  return (
    <div
      data-testid="compare-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <nav
        aria-label="Compare mode selector"
        data-testid="compare-mode-tabs"
        className="flex items-center gap-1 px-3 shrink-0"
        style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}
      >
        {MODES.map((m) => (
          <button
            key={m.id}
            type="button"
            data-testid={`compare-mode-tab-${m.id}`}
            data-active={mode === m.id}
            onClick={() => setActiveModeId(m.id)}
            style={{
              padding: '3px 10px',
              borderRadius: 4,
              fontSize: 12,
              background: mode === m.id ? 'var(--surface-raised)' : 'transparent',
              color: mode === m.id ? 'var(--text-primary)' : 'var(--text-muted)',
              border:
                mode === m.id
                  ? '1px solid var(--border-hairline)'
                  : '1px solid transparent',
              cursor: 'pointer',
            }}
          >
            {m.label}
          </button>
        ))}
      </nav>

      <PlayheadBar
        maxTick={maxTick}
        markers={{
          forkAnchorTick,
          firstDivergenceTick: pairDiff?.first_divergence_tick ?? null,
        }}
        forkableRows={summaries.map((s, i) => ({
          id: s.id,
          ticks: s.forkable_ticks ?? [],
          stroke: sessionStroke(i),
        }))}
      />

      {pairDiff?.history_truncated && (
        <div
          data-testid="compare-history-truncated"
          className="flex items-center px-3 shrink-0"
          style={{
            height: 26,
            gap: 8,
            fontSize: 'var(--text-xs)',
            color: 'var(--severity-warning)',
            borderBottom: '1px solid var(--border-hairline)',
          }}
        >
          <span aria-hidden>⚠</span>
          history truncated — one side has evicted snapshots the other
          still holds; the earliest divergence may lie before the shared
          range.
        </div>
      )}
      {missingIds.length > 0 && (
        <div
          data-testid="compare-missing-banner"
          className="flex items-center px-3 shrink-0"
          style={{
            height: 26,
            gap: 8,
            fontSize: 'var(--text-xs)',
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-hairline)',
          }}
        >
          {missingIds.length} picked session{missingIds.length === 1 ? ' is' : 's are'} no
          longer available (see the session rail).
        </div>
      )}

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <SessionPickRail />

        <div className="flex flex-col flex-1 min-w-0 min-h-0">
          {ready && (
            <VariablePickerRow
              unionNames={unionNames}
              shownVars={shownVars}
              isAuto={pickedVars === null}
              autoFetchCap={AUTO_FETCH_CAP}
              autoTruncated={pickedVars === null && unionNames.length > AUTO_FETCH_CAP}
              onAdd={(name) => setPickedVars([...(pickedVars ?? shownVars), name])}
              onRemove={(name) =>
                setPickedVars((pickedVars ?? shownVars).filter((v) => v !== name))
              }
              onAuto={() => setPickedVars(null)}
            />
          )}

          {ready && mode === 'ensemble' && (
            <EnsembleStrip
              variables={shownVars}
              samplesByVar={samplesByVar}
              summaries={summaries}
              sharedTick={sharedTick}
            />
          )}
          {ready && mode === 'golden' && (
            <GoldenStrip
              variables={shownVars}
              samplesByVar={samplesByVar}
              summaries={summaries}
            />
          )}
          {ready && mode === 'two-design' && (
            <TwoDesignStrip
              variables={shownVars}
              samplesByVar={samplesByVar}
              summaries={summaries}
              onScrubTo={setSharedTick}
            />
          )}

          {!ready ? (
            <TeachingState pickedCount={summaries.length} />
          ) : (
            <DiffCanvas
              summaries={summaries}
              samplesByVar={samplesByVar}
              variables={shownVars}
              maxTick={maxTick}
              sharedTick={sharedTick}
              onScrubTo={setSharedTick}
              pairDiff={isPair ? (pairDiff ?? null) : null}
              namesBySession={namesBySession}
            />
          )}
        </div>
      </div>
    </div>
  );
}

// ── Variable picker row ─────────────────────────────────────────────

function VariablePickerRow({
  unionNames,
  shownVars,
  isAuto,
  autoFetchCap,
  autoTruncated,
  onAdd,
  onRemove,
  onAuto,
}: {
  unionNames: string[];
  shownVars: string[];
  isAuto: boolean;
  autoFetchCap: number;
  autoTruncated: boolean;
  onAdd: (name: string) => void;
  onRemove: (name: string) => void;
  onAuto: () => void;
}) {
  const addable = unionNames.filter((n) => !shownVars.includes(n));
  return (
    <div
      data-testid="compare-variable-picker"
      className="flex items-center px-3 shrink-0"
      style={{
        minHeight: 30,
        gap: 6,
        borderBottom: '1px solid var(--border-hairline)',
        flexWrap: 'wrap',
        paddingTop: 3,
        paddingBottom: 3,
      }}
    >
      <span
        style={{
          fontSize: 'var(--text-xs)',
          textTransform: 'uppercase',
          letterSpacing: '0.05em',
          color: 'var(--text-muted)',
        }}
      >
        variables
      </span>
      {shownVars.map((name) => (
        <span
          key={name}
          data-testid={`compare-var-chip-${name}`}
          className="inline-flex items-center"
          style={{
            gap: 4,
            fontSize: 'var(--text-xs)',
            fontFamily: 'var(--font-mono)',
            padding: '1px 6px',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            color: 'var(--text-primary)',
          }}
        >
          {name}
          <button
            type="button"
            aria-label={`Remove ${name}`}
            onClick={() => onRemove(name)}
            style={{
              border: 'none',
              background: 'transparent',
              color: 'var(--text-muted)',
              cursor: 'pointer',
              padding: 0,
              fontSize: 'var(--text-xs)',
              lineHeight: 1,
            }}
          >
            ×
          </button>
        </span>
      ))}
      {addable.length > 0 && (
        <select
          data-testid="compare-var-add"
          value=""
          onChange={(e) => {
            if (e.target.value) onAdd(e.target.value);
          }}
          style={{
            fontSize: 'var(--text-xs)',
            background: 'var(--surface-panel)',
            color: 'var(--text-secondary)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            padding: '1px 4px',
          }}
        >
          <option value="">+ variable…</option>
          {addable.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      )}
      {!isAuto && (
        <button
          type="button"
          data-testid="compare-var-auto"
          onClick={onAuto}
          title="Back to auto-pick (top variables by cross-session variance)"
          style={{
            fontSize: 'var(--text-xs)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            background: 'transparent',
            color: 'var(--text-secondary)',
            cursor: 'pointer',
            padding: '1px 6px',
          }}
        >
          auto
        </button>
      )}
      {autoTruncated && (
        <span
          data-testid="compare-var-truncation-note"
          style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}
        >
          auto scored the first {autoFetchCap} of {unionNames.length} recorded
          variables — pick any other explicitly
        </span>
      )}
    </div>
  );
}

// ── Teaching state ──────────────────────────────────────────────────

function TeachingState({ pickedCount }: { pickedCount: number }) {
  const needed = Math.max(0, 2 - pickedCount);
  return (
    <div
      data-testid="compare-teaching"
      className="flex flex-col items-center justify-center flex-1"
      style={{ gap: 8, color: 'var(--text-muted)' }}
    >
      <div style={{ fontSize: 'var(--text-md, 14px)', color: 'var(--text-secondary)' }}>
        Pick {needed === 0 ? 2 : needed} more session{needed === 1 ? '' : 's'} to compare
      </div>
      <div
        style={{
          fontSize: 'var(--text-sm)',
          maxWidth: 380,
          textAlign: 'center',
          lineHeight: 1.5,
        }}
      >
        Compare reads the live session catalog — run a scenario, fork it
        at a decision point (fork rows carry ⑂), and the diff canvas
        shows exactly where the branches diverge under one playhead.
      </div>
    </div>
  );
}
