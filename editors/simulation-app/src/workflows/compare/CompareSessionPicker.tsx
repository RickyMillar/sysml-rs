/**
 * CompareSessionPicker — left sidebar multi-select session list.
 *
 * Shows every archived session (plus any live sessions surfaced via
 * `useSessionList`) as a row with a checkbox. The picker enforces 2..6
 * picked sessions through `useCompareStore.togglePickedSession` +
 * `clampPicks`. Rows show tick count and kind so the user can spot the
 * "longest" session that will drive the shared playhead.
 *
 * Archive source: `listArchivedSessions` from `@/shared/data/SessionArchive`.
 * If Agent V ships a new `useArchiveList` hook under `features/archive/`
 * that returns the same `ArchivedSessionSummary` shape, the import can be
 * swapped in a follow-up without changing this component's props.
 */

import { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import {
  listArchivedSessions,
  type ArchivedSessionSummary,
} from '@/shared/data/SessionArchive';
import { useCompareStore } from './useCompareStore';

export interface CompareSessionPickerProps {
  /**
   * Optional override — if Agent V's `useArchiveList` hook is wired in
   * later, the caller can pass a pre-fetched list here and the picker
   * skips its own IndexedDB read.
   */
  archive?: ArchivedSessionSummary[];
}

export function CompareSessionPicker({ archive }: CompareSessionPickerProps) {
  const [internalArchive, setInternalArchive] = useState<ArchivedSessionSummary[]>([]);
  const [loading, setLoading] = useState(archive == null);

  useEffect(() => {
    if (archive != null) {
      setInternalArchive(archive);
      setLoading(false);
      return;
    }
    let cancelled = false;
    listArchivedSessions()
      .then((list) => {
        if (cancelled) return;
        setInternalArchive(list);
        setLoading(false);
      })
      .catch(() => {
        if (cancelled) return;
        setInternalArchive([]);
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [archive]);

  const sessions = archive ?? internalArchive;
  const picked = useCompareStore((s) => s.pickedSessionIds);
  const togglePicked = useCompareStore((s) => s.togglePickedSession);
  const setPicked = useCompareStore((s) => s.setPickedSessionIds);

  const pickedCount = picked.length;

  const headerStyle: CSSProperties = {
    fontSize: 10,
    fontWeight: 700,
    textTransform: 'uppercase',
    letterSpacing: '0.06em',
    color: 'var(--outline)',
    padding: '8px 12px',
    borderBottom: '1px solid var(--outline-variant)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
  };

  return (
    <div
      data-testid="compare-session-picker"
      className="flex flex-col h-full overflow-hidden"
      style={{ width: 260, borderRight: '1px solid var(--outline-variant)' }}
    >
      <div style={headerStyle}>
        <span>Sessions</span>
        <span
          data-testid="compare-session-picker-count"
          className="mono-text"
          style={{
            color:
              pickedCount < 2
                ? 'var(--severity-error)'
                : pickedCount >= 6
                  ? 'var(--tertiary)'
                  : 'var(--primary)',
          }}
        >
          {pickedCount}/6
        </span>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading && (
          <div
            style={{
              padding: 12,
              fontSize: 12,
              color: 'var(--outline)',
            }}
          >
            Loading archive…
          </div>
        )}
        {!loading && sessions.length === 0 && (
          <div
            data-testid="compare-session-picker-empty"
            style={{
              padding: 12,
              fontSize: 12,
              color: 'var(--outline)',
              lineHeight: 1.4,
            }}
          >
            No archived sessions yet. Run a session to capture data, then
            return here to compare.
          </div>
        )}
        {!loading &&
          sessions.map((s) => {
            const isPicked = picked.includes(s.id);
            const pickIndex = picked.indexOf(s.id);
            return (
              <label
                key={s.id}
                data-testid={`compare-session-picker-row-${s.id}`}
                data-picked={isPicked ? 'true' : 'false'}
                className="flex items-start gap-2 cursor-pointer"
                style={{
                  padding: '8px 12px',
                  background: isPicked
                    ? 'var(--primary-container, #004b7a33)'
                    : 'transparent',
                  borderBottom: '1px solid var(--outline-variant)',
                }}
              >
                <input
                  type="checkbox"
                  checked={isPicked}
                  disabled={!isPicked && pickedCount >= 6}
                  onChange={() => togglePicked(s.id)}
                  style={{ marginTop: 3 }}
                />
                <div className="flex-1 min-w-0">
                  <div
                    style={{
                      fontSize: 12,
                      fontWeight: 600,
                      color: 'var(--on-surface)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {s.label ?? s.id.slice(0, 8)}
                  </div>
                  <div
                    className="mono-text"
                    style={{
                      fontSize: 10,
                      color: 'var(--outline)',
                      marginTop: 2,
                    }}
                  >
                    tick {s.tick} · {s.kind}
                    {isPicked && (
                      <span style={{ color: 'var(--primary)', marginLeft: 6 }}>
                        #{pickIndex + 1}
                      </span>
                    )}
                  </div>
                </div>
              </label>
            );
          })}
      </div>

      {pickedCount > 0 && (
        <div
          style={{
            padding: '6px 12px',
            borderTop: '1px solid var(--outline-variant)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}
        >
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)' }}
          >
            {pickedCount < 2 ? 'Pick at least 2 sessions' : `${pickedCount} picked`}
          </span>
          <button
            type="button"
            data-testid="compare-session-picker-clear"
            onClick={() => setPicked([])}
            style={{
              fontSize: 10,
              padding: '2px 8px',
              background: 'var(--surface-container-high)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 3,
              color: 'var(--on-surface-variant)',
              cursor: 'pointer',
            }}
          >
            Clear
          </button>
        </div>
      )}
    </div>
  );
}
