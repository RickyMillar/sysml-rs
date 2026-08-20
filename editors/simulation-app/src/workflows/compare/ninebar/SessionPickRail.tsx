/**
 * SessionPickRail — the Phase 6 session/fork picker (left column of
 * the Compare surface; "session/fork pickers in the left rail", one
 * hop from the frame session switcher which deep-links here).
 *
 * Lists LIVE sessions from `sessions.list` — the population that
 * carries fork lineage (`fork_point_tick`) and `forkable_ticks`.
 * Forked sessions show a `⑂ fork @ N` badge. Picks are 2..6 via the
 * shared `useCompareStore` (unchanged mechanics — only the data
 * source moved off IndexedDB archives; archived/golden runs enter
 * through the golden mode's picker instead, see plan item 24).
 *
 * Picked ids that no longer resolve against the live list (reaped /
 * server restart) render an explicit "no longer available" row —
 * never silently dropped.
 */

import { useState } from 'react';
import { useSessionList } from '@/features/sessions/queries';
import { useForkSession } from '@/features/sessions/mutations';
import type { SessionSummary } from '@/features/sessions/types';
import { useCompareStore } from '../useCompareStore';
import { sessionStroke } from './channels';
import { describeForkAtTickError, parseForkAtTickError } from './forkErrors';

function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}

export function SessionPickRail() {
  const { data: sessions, isLoading } = useSessionList();
  const picked = useCompareStore((s) => s.pickedSessionIds);
  const togglePicked = useCompareStore((s) => s.togglePickedSession);
  const setPicked = useCompareStore((s) => s.setPickedSessionIds);

  const list = sessions ?? [];
  const liveIds = new Set(list.map((s) => s.id));
  const missingPicked = picked.filter((id) => !liveIds.has(id));

  return (
    <div
      data-testid="compare-session-rail"
      className="flex flex-col h-full overflow-hidden shrink-0"
      style={{
        width: 248,
        borderRight: '1px solid var(--border-hairline)',
        background: 'var(--surface-panel)',
      }}
    >
      <div
        className="flex items-center justify-between px-3 shrink-0"
        style={{
          height: 28,
          borderBottom: '1px solid var(--border-hairline)',
          fontSize: 'var(--text-xs)',
          letterSpacing: '0.05em',
          textTransform: 'uppercase',
          color: 'var(--text-muted)',
        }}
      >
        <span>Sessions</span>
        <span
          data-testid="compare-session-rail-count"
          style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-secondary)' }}
        >
          {picked.length}/6
        </span>
      </div>

      <div className="flex-1 overflow-y-auto">
        {isLoading && (
          <RailNote testid="compare-session-rail-loading">loading sessions…</RailNote>
        )}
        {!isLoading && list.length === 0 && (
          <RailNote testid="compare-session-rail-empty">
            No live sessions. Start a run (or fork one) and come back —
            Compare reads the session catalog.
          </RailNote>
        )}
        {list.map((s) => (
          <SessionRow
            key={s.id}
            summary={s}
            picked={picked.includes(s.id)}
            pickIndex={picked.indexOf(s.id)}
            disabled={!picked.includes(s.id) && picked.length >= 6}
            onToggle={() => togglePicked(s.id)}
          />
        ))}

        {missingPicked.length > 0 && (
          <div
            data-testid="compare-session-rail-missing"
            style={{
              padding: '6px 12px',
              fontSize: 'var(--text-xs)',
              color: 'var(--text-muted)',
              borderTop: '1px solid var(--border-hairline)',
            }}
          >
            {missingPicked.map((id) => (
              <div key={id} className="flex items-center justify-between" style={{ gap: 6 }}>
                <span style={{ fontFamily: 'var(--font-mono)' }}>{shortId(id)}</span>
                <span>no longer available</span>
                <button
                  type="button"
                  data-testid={`compare-session-rail-drop-${id}`}
                  onClick={() => togglePicked(id)}
                  style={{
                    fontSize: 'var(--text-xs)',
                    border: 'none',
                    background: 'transparent',
                    color: 'var(--text-secondary)',
                    cursor: 'pointer',
                    textDecoration: 'underline',
                  }}
                >
                  drop
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div
        className="flex items-center justify-between px-3 shrink-0"
        style={{
          height: 28,
          borderTop: '1px solid var(--border-hairline)',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-muted)',
        }}
      >
        <span>{picked.length < 2 ? 'pick at least 2' : `${picked.length} picked`}</span>
        {picked.length > 0 && (
          <button
            type="button"
            data-testid="compare-session-rail-clear"
            onClick={() => setPicked([])}
            style={{
              fontSize: 'var(--text-xs)',
              border: 'none',
              background: 'transparent',
              color: 'var(--text-secondary)',
              cursor: 'pointer',
              textDecoration: 'underline',
            }}
          >
            clear
          </button>
        )}
      </div>
    </div>
  );
}

function RailNote({ children, testid }: { children: React.ReactNode; testid: string }) {
  return (
    <div
      data-testid={testid}
      style={{
        padding: '10px 12px',
        fontSize: 'var(--text-sm)',
        color: 'var(--text-muted)',
        lineHeight: 1.5,
      }}
    >
      {children}
    </div>
  );
}

function SessionRow({
  summary,
  picked,
  pickIndex,
  disabled,
  onToggle,
}: {
  summary: SessionSummary;
  picked: boolean;
  pickIndex: number;
  disabled: boolean;
  onToggle: () => void;
}) {
  const isFork = summary.fork_point_tick !== null;
  const sharedTick = useCompareStore((s) => s.sharedTick);
  const togglePicked = useCompareStore((s) => s.togglePickedSession);
  const fork = useForkSession();
  const [forkError, setForkError] = useState<string | null>(null);

  // F8: the fork affordance exists ONLY at the exact archived ticks
  // (`forkable_ticks`) — no guessing, no clamping. The structured
  // error path below still runs honestly if a 1 Hz-stale summary
  // races an eviction.
  const forkableHere = picked && (summary.forkable_ticks ?? []).includes(sharedTick);

  const forkAtPlayhead = () => {
    setForkError(null);
    fork.mutate(
      { sessionId: summary.id, atTick: sharedTick },
      {
        onSuccess: (child) => {
          // Pull the new branch straight into the comparison (clamped
          // to the 2..6 window by the store).
          togglePicked(child.id);
        },
        onError: (e) => {
          const structured = parseForkAtTickError(
            e instanceof Error ? e.message : String(e),
          );
          setForkError(
            structured
              ? describeForkAtTickError(structured)
              : e instanceof Error
                ? e.message
                : String(e),
          );
        },
      },
    );
  };

  return (
    <label
      data-testid={`compare-session-rail-row-${summary.id}`}
      data-picked={picked ? 'true' : 'false'}
      className="flex items-start gap-2"
      style={{
        padding: '6px 12px',
        cursor: disabled ? 'not-allowed' : 'pointer',
        background: picked ? 'var(--accent-tint)' : 'transparent',
        opacity: summary.is_expired ? 0.55 : 1,
      }}
    >
      <input
        type="checkbox"
        checked={picked}
        disabled={disabled}
        onChange={onToggle}
        style={{ marginTop: 3, accentColor: 'var(--accent)' }}
      />
      <div className="flex-1 min-w-0">
        <div className="flex items-center" style={{ gap: 6 }}>
          {picked && (
            <span
              aria-hidden
              style={{
                width: 8,
                height: 8,
                borderRadius: 2,
                background: sessionStroke(pickIndex),
                flexShrink: 0,
              }}
            />
          )}
          <span
            style={{
              fontSize: 'var(--text-sm)',
              color: 'var(--text-primary)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {summary.label ?? shortId(summary.id)}
          </span>
        </div>
        <div
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-xs)',
            color: 'var(--text-muted)',
            marginTop: 1,
            display: 'flex',
            gap: 6,
            alignItems: 'center',
          }}
        >
          <span>tick {summary.tick}</span>
          <span>{summary.kind}</span>
          {isFork && (
            <span
              data-testid={`compare-session-fork-badge-${summary.id}`}
              title={`Forked from its parent at tick ${summary.fork_point_tick}`}
              style={{ color: 'var(--text-secondary)' }}
            >
              ⑂ fork @ {summary.fork_point_tick}
            </span>
          )}
          {summary.is_expired && <span>expired</span>}
        </div>
        {forkableHere && (
          <button
            type="button"
            data-testid={`compare-fork-here-${summary.id}`}
            disabled={fork.isPending}
            onClick={(e) => {
              e.preventDefault();
              forkAtPlayhead();
            }}
            title={`Fork this session at the playhead (tick ${sharedTick} is an archived snapshot) and add the branch to the comparison`}
            style={{
              marginTop: 3,
              fontSize: 'var(--text-xs)',
              padding: '1px 6px',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              background: 'transparent',
              color: 'var(--text-secondary)',
              cursor: fork.isPending ? 'default' : 'pointer',
            }}
          >
            {fork.isPending ? 'forking…' : `⑂ fork @ tick ${sharedTick}`}
          </button>
        )}
        {forkError && (
          <div
            data-testid={`compare-fork-error-${summary.id}`}
            style={{
              marginTop: 3,
              fontSize: 'var(--text-xs)',
              color: 'var(--severity-warning)',
              lineHeight: 1.4,
            }}
          >
            {forkError}
          </div>
        )}
      </div>
    </label>
  );
}
