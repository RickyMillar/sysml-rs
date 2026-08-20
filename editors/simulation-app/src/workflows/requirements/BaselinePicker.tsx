/**
 * Baseline pill + dropdown (demo 1c §7) — the toolbar affordance that
 * picks which baseline suspect flags are computed against.
 *
 * Pill: mono label, border flips to accent + chevron ▾→▴ while the
 * dropdown is open. Dropdown: 300px anchored overlay (12px radius per
 * the ninebar overlay ruling), 32px rows, current row = accent ink-bar
 * + filled ● + the commit hash in muted mono; "+ New baseline…" is an
 * inline-input row (anchored overlays, never modals).
 *
 * Purely presentational — data + mutation come in as props.
 */

import { useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import type { BaselineMeta } from '@/features/baselines/types';

/** Short hash for display: commit ids are content hashes/uuids. */
export function shortCommit(commit: string): string {
  return commit.length > 8 ? commit.slice(0, 8) : commit;
}

export function formatBaselineDate(createdAtSecs: number): string {
  const d = new Date(createdAtSecs * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function BaselinePicker({
  baselines,
  selected,
  onSelect,
  onCreate,
  creating,
}: {
  baselines: BaselineMeta[];
  /** Currently selected baseline name (null = none picked / none exist). */
  selected: string | null;
  onSelect: (name: string) => void;
  /** Create a baseline at the latest commit with this name. */
  onCreate: (name: string) => void;
  /** True while a create mutation is in flight. */
  creating: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [naming, setNaming] = useState(false);
  const [draftName, setDraftName] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);

  // Light-dismiss: click anywhere outside closes the overlay.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
        setNaming(false);
      }
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  const current = baselines.find((b) => b.name === selected) ?? null;
  const pillLabel = current
    ? `Baseline ${current.name}`
    : baselines.length === 0
      ? 'No baseline'
      : 'Pick baseline';

  const submitDraft = () => {
    const name = draftName.trim();
    if (!name || creating) return;
    onCreate(name);
    setDraftName('');
    setNaming(false);
  };

  return (
    <div ref={rootRef} style={{ position: 'relative' }}>
      <button
        type="button"
        data-testid="baseline-pill"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color: current ? 'var(--text-primary)' : 'var(--text-muted)',
          background: 'transparent',
          border: `1px solid ${open ? 'var(--accent)' : 'var(--border-default)'}`,
          borderRadius: 'var(--radius-sm)',
          padding: '3px 10px',
          cursor: 'pointer',
        }}
      >
        {pillLabel}
        <span aria-hidden style={{ fontSize: 9 }}>{open ? '▴' : '▾'}</span>
      </button>

      {open && (
        <div
          data-testid="baseline-dropdown"
          role="listbox"
          aria-label="Baselines"
          style={{
            position: 'absolute',
            top: 'calc(100% + 6px)',
            right: 0,
            width: 300,
            background: 'var(--surface-overlay)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-lg)',
            boxShadow: 'var(--shadow-float)',
            zIndex: 30,
            overflow: 'hidden',
            paddingBottom: 2,
          }}
        >
          {baselines.length === 0 && (
            <div
              data-testid="baseline-dropdown-empty"
              style={{
                padding: '10px 12px',
                fontSize: 'var(--text-xs)',
                color: 'var(--text-muted)',
              }}
            >
              No baselines yet. A baseline pins the current model so later
              edits can be flagged as suspect.
            </div>
          )}
          {baselines.map((b) => {
            const isCurrent = b.name === selected;
            const rowStyle: CSSProperties = {
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              width: '100%',
              height: 32,
              padding: '0 12px',
              border: 'none',
              textAlign: 'left',
              cursor: 'pointer',
              fontSize: 'var(--text-sm)',
              background: isCurrent ? 'var(--accent-tint)' : 'transparent',
              boxShadow: isCurrent ? 'inset 2px 0 0 var(--accent)' : 'none',
              color: isCurrent ? 'var(--text-primary)' : 'var(--text-secondary)',
            };
            // Git provenance (B6): corroborating metadata in the row
            // tooltip; a dirty-tree capture gets a visible ± marker (the
            // SHA alone does not reproduce that content).
            const git = b.provenance ?? null;
            const gitTitle = git
              ? `git: ${git.branch ?? '(detached)'} @ ${shortCommit(git.sha)}${git.dirty ? ' — created from a dirty work tree' : ''}`
              : undefined;
            return (
              <button
                key={b.name}
                type="button"
                role="option"
                aria-selected={isCurrent}
                data-testid={`baseline-option-${b.name}`}
                title={gitTitle}
                onClick={() => {
                  onSelect(b.name);
                  setOpen(false);
                }}
                style={rowStyle}
              >
                <span
                  aria-hidden
                  style={{
                    color: isCurrent ? 'var(--accent)' : 'var(--text-muted)',
                    fontSize: 10,
                  }}
                >
                  {isCurrent ? '●' : '○'}
                </span>
                <span
                  style={{
                    flex: 1,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {b.name} {formatBaselineDate(b.created_at)}
                </span>
                {git?.dirty && (
                  <span
                    data-testid={`baseline-dirty-${b.name}`}
                    title="Created from a dirty git work tree — the recorded SHA alone does not reproduce this content (the baseline's own content commit does)"
                    style={{
                      color: 'var(--severity-warning)',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--text-xs)',
                    }}
                  >
                    ±
                  </span>
                )}
                <span
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--text-xs)',
                    color: 'var(--text-muted)',
                  }}
                >
                  {shortCommit(b.commit)}
                </span>
              </button>
            );
          })}
          {naming ? (
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                height: 32,
                padding: '0 12px',
                borderTop: '1px solid var(--border-hairline)',
              }}
            >
              <input
                autoFocus
                data-testid="baseline-name-input"
                value={draftName}
                disabled={creating}
                placeholder="baseline name"
                onChange={(e) => setDraftName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') submitDraft();
                  if (e.key === 'Escape') {
                    setNaming(false);
                    setDraftName('');
                  }
                }}
                style={{
                  flex: 1,
                  background: 'transparent',
                  border: 'none',
                  outline: 'none',
                  color: 'var(--text-primary)',
                  fontSize: 'var(--text-sm)',
                  fontFamily: 'var(--font-mono)',
                }}
              />
              <button
                type="button"
                data-testid="baseline-name-submit"
                disabled={creating || draftName.trim() === ''}
                onClick={submitDraft}
                style={{
                  border: '1px solid var(--border-default)',
                  borderRadius: 'var(--radius-sm)',
                  background: 'transparent',
                  color: 'var(--text-primary)',
                  fontSize: 'var(--text-xs)',
                  padding: '2px 8px',
                  cursor: creating ? 'default' : 'pointer',
                  opacity: creating || draftName.trim() === '' ? 0.5 : 1,
                }}
              >
                {creating ? 'creating…' : 'create'}
              </button>
            </div>
          ) : (
            <button
              type="button"
              data-testid="baseline-new"
              onClick={() => setNaming(true)}
              style={{
                display: 'flex',
                alignItems: 'center',
                width: '100%',
                height: 32,
                padding: '0 12px',
                border: 'none',
                borderTop: '1px solid var(--border-hairline)',
                background: 'transparent',
                color: 'var(--text-muted)',
                fontSize: 'var(--text-sm)',
                textAlign: 'left',
                cursor: 'pointer',
              }}
            >
              + New baseline…
            </button>
          )}
        </div>
      )}
    </div>
  );
}
