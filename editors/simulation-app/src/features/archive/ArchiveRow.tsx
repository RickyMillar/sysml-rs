/**
 * ArchiveRow — one archived-session row inside `<ArchivePanel />` (R4.1).
 *
 * Composition:
 *   [★] label (origin chip)           [worst VerdictBadge]   [⋯]
 *   workspace_uri · N ticks · 12m ago
 *
 * The row itself is a `<button>` so keyboard navigation (Enter / Space)
 * triggers the default action (Restore). The three-dot menu is its own
 * button, stops propagation, and hosts the secondary actions (Mark /
 * Unmark Golden, Copy ID).
 *
 * `prefers-reduced-motion` is respected — the hover tint applies without
 * a transition when the user has reduced-motion enabled.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';
import {
  VerdictBadge,
  type VerdictKind,
} from '@/components/VerdictBadge';
import type { ArchivedSessionSummary } from './types';
import { ARCHIVE_ORIGIN_LABELS } from './types';
import { worstVerdict } from './filterArchive';

export interface ArchiveRowProps {
  entry: ArchivedSessionSummary;
  /** Default action when the row is clicked or Enter/Space pressed. */
  onRestore: (entry: ArchivedSessionSummary) => void;
  /** Three-dot menu → "Mark Golden". Required — panel provides. */
  onMarkGolden: (entry: ArchivedSessionSummary) => void;
  /** Three-dot menu → "Unmark Golden". Required — panel provides. */
  onUnmarkGolden: (entry: ArchivedSessionSummary) => void;
  /** Three-dot menu → "Copy ID". */
  onCopyId: (entry: ArchivedSessionSummary) => void;
  /**
   * Reference time used to render the relative timestamp. Injected so
   * tests can pin a deterministic "N minutes ago" without touching
   * `Date.now`.
   */
  now?: number;
}

/**
 * Render a human-readable relative timestamp ("3m ago", "2h ago",
 * "4d ago", "Apr 1"). Deliberately tiny — no date-fns dependency; the
 * archive surface only needs coarse buckets.
 */
export function formatRelativeTime(tsMs: number, now: number = Date.now()): string {
  const delta = Math.max(0, now - tsMs);
  const sec = Math.floor(delta / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  const date = new Date(tsMs);
  const nowDate = new Date(now);
  const sameYear = date.getFullYear() === nowDate.getFullYear();
  const month = date.toLocaleString('en-US', { month: 'short' });
  return sameYear
    ? `${month} ${date.getDate()}`
    : `${month} ${date.getDate()}, ${date.getFullYear()}`;
}

/**
 * Build the `aria-label` text for the row button. Exported so tests can
 * assert the shape without replicating the logic.
 */
export function buildRowAriaLabel(
  entry: ArchivedSessionSummary,
  verdict: VerdictKind | null,
  now: number,
): string {
  const parts: string[] = [];
  if (entry.is_golden) parts.push('Golden');
  parts.push(entry.label);
  parts.push(`${ARCHIVE_ORIGIN_LABELS[entry.origin]} session`);
  if (verdict) parts.push(`worst verdict ${verdict}`);
  parts.push(`${entry.ticks} ticks`);
  parts.push(formatRelativeTime(entry.created_at, now));
  parts.push('press Enter to restore');
  return parts.join(', ');
}

// ── Component ────────────────────────────────────────────────────────

export function ArchiveRow({
  entry,
  onRestore,
  onMarkGolden,
  onUnmarkGolden,
  onCopyId,
  now = Date.now(),
}: ArchiveRowProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [hovered, setHovered] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const menuButtonRef = useRef<HTMLButtonElement | null>(null);

  const verdict = useMemo(() => worstVerdict(entry.verdict_counts), [entry.verdict_counts]);
  const reduceMotion = usePrefersReducedMotion();
  const ariaLabel = useMemo(
    () => buildRowAriaLabel(entry, verdict, now),
    [entry, verdict, now],
  );
  const relative = useMemo(() => formatRelativeTime(entry.created_at, now), [entry.created_at, now]);

  // ── Dismiss menu on outside click / Escape ──────────────────────────
  useEffect(() => {
    if (!menuOpen) return;
    const handlePointer = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (
        menuRef.current && target && menuRef.current.contains(target)
      ) {
        return;
      }
      if (
        menuButtonRef.current && target && menuButtonRef.current.contains(target)
      ) {
        return;
      }
      setMenuOpen(false);
    };
    const handleKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') setMenuOpen(false);
    };
    document.addEventListener('pointerdown', handlePointer);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('pointerdown', handlePointer);
      document.removeEventListener('keydown', handleKey);
    };
  }, [menuOpen]);

  const handleRestore = useCallback(() => {
    onRestore(entry);
  }, [entry, onRestore]);

  const handleRowKey = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        handleRestore();
      }
    },
    [handleRestore],
  );

  const toggleMenu = useCallback((event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setMenuOpen((prev) => !prev);
  }, []);

  const rowStyle: CSSProperties = {
    display: 'flex',
    alignItems: 'stretch',
    gap: 0,
    padding: 0,
    margin: 0,
    position: 'relative',
    width: '100%',
    background: hovered ? 'var(--surface-raised)' : 'transparent',
    border: 'none',
    borderBottom: '1px solid var(--border-default)',
    color: 'var(--text-primary)',
    textAlign: 'left',
    cursor: 'pointer',
    fontFamily: 'inherit',
    transition: reduceMotion ? undefined : 'background 120ms ease',
  };

  return (
    <div
      style={{ position: 'relative' }}
      data-testid={`archive-row-${entry.id}`}
      data-golden={entry.is_golden ? 'true' : 'false'}
    >
      <button
        type="button"
        aria-label={ariaLabel}
        onClick={handleRestore}
        onKeyDown={handleRowKey}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        data-testid="archive-row-button"
        style={rowStyle}
      >
        <div style={{ flex: 1, padding: '8px 10px', display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            {entry.is_golden ? (
              // Judgment call: a starred/golden marker is a persistent
              // user/system flag (akin to "pinned"), not a verdict or
              // severity. There's no dedicated starred token, but this
              // genuinely reads as primacy \u2014 "this one matters" \u2014 which
              // the mapping guide allows accent for.
              <span
                aria-hidden="true"
                data-testid="archive-row-golden-star"
                style={{ color: 'var(--accent-fg)', fontSize: 13, lineHeight: 1 }}
              >
                {'\u2605'}
              </span>
            ) : null}
            <strong
              style={{
                fontSize: 'var(--text-sm, 13px)',
                fontWeight: 600,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 180,
              }}
              title={entry.label}
            >
              {entry.label}
            </strong>
            <OriginChip origin={entry.origin} />
          </div>
          <div
            style={{
              display: 'flex',
              gap: 6,
              alignItems: 'center',
              fontSize: 'var(--text-xs, 11px)',
              color: 'var(--text-muted)',
            }}
          >
            <span
              style={{
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 140,
              }}
              title={entry.workspace_uri}
              data-testid="archive-row-workspace"
            >
              {entry.workspace_uri}
            </span>
            <Separator />
            <span>{entry.ticks} ticks</span>
            <Separator />
            <span data-testid="archive-row-timestamp">{relative}</span>
          </div>
        </div>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            padding: '0 8px',
          }}
        >
          {verdict ? (
            <VerdictBadge
              verdict={verdict}
              size="compact"
              showLabel={false}
              name={entry.label}
              reason={buildVerdictReason(entry.verdict_counts)}
              testId={`archive-row-verdict-${entry.id}`}
            />
          ) : null}
        </div>
      </button>
      <button
        ref={menuButtonRef}
        type="button"
        aria-label={`Actions for ${entry.label}`}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        data-testid="archive-row-menu-button"
        onClick={toggleMenu}
        style={{
          position: 'absolute',
          top: 6,
          right: 6,
          width: 22,
          height: 22,
          padding: 0,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: 'transparent',
          border: '1px solid transparent',
          borderRadius: 3,
          color: 'var(--text-muted)',
          cursor: 'pointer',
          fontSize: 14,
          lineHeight: 1,
        }}
      >
        <span aria-hidden="true">{'\u22EF'}</span>
      </button>
      {menuOpen ? (
        <div
          ref={menuRef}
          role="menu"
          data-testid="archive-row-menu"
          style={{
            position: 'absolute',
            top: 26,
            right: 6,
            minWidth: 160,
            background: 'var(--surface-raised)',
            border: '1px solid var(--border-default)',
            borderRadius: 4,
            boxShadow: '0 4px 14px rgba(0,0,0,0.35)',
            padding: 4,
            zIndex: 10,
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
          }}
        >
          <MenuItem
            testId="archive-row-menu-restore"
            onSelect={() => {
              setMenuOpen(false);
              onRestore(entry);
            }}
          >
            Restore
          </MenuItem>
          {entry.is_golden ? (
            <MenuItem
              testId="archive-row-menu-unmark-golden"
              onSelect={() => {
                setMenuOpen(false);
                onUnmarkGolden(entry);
              }}
            >
              Unmark Golden
            </MenuItem>
          ) : (
            <MenuItem
              testId="archive-row-menu-mark-golden"
              onSelect={() => {
                setMenuOpen(false);
                onMarkGolden(entry);
              }}
            >
              Mark Golden
            </MenuItem>
          )}
          <MenuItem
            testId="archive-row-menu-copy-id"
            onSelect={() => {
              setMenuOpen(false);
              onCopyId(entry);
            }}
          >
            Copy ID
          </MenuItem>
        </div>
      ) : null}
    </div>
  );
}

// ── Internals ────────────────────────────────────────────────────────

function MenuItem({
  children,
  onSelect,
  testId,
}: {
  children: ReactNode;
  onSelect: () => void;
  testId?: string;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      data-testid={testId}
      onClick={onSelect}
      style={{
        display: 'block',
        width: '100%',
        background: 'transparent',
        border: 'none',
        color: 'var(--text-primary)',
        padding: '6px 8px',
        textAlign: 'left',
        cursor: 'pointer',
        fontSize: 'var(--text-xs, 11px)',
        borderRadius: 3,
      }}
    >
      {children}
    </button>
  );
}

function OriginChip({ origin }: { origin: ArchivedSessionSummary['origin'] }) {
  const label = ARCHIVE_ORIGIN_LABELS[origin];
  return (
    <span
      data-testid={`archive-row-origin-${origin}`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        height: 16,
        padding: '0 6px',
        borderRadius: 8,
        background: 'var(--surface-panel)',
        border: '1px solid var(--border-default)',
        color: 'var(--text-muted)',
        fontSize: 10,
        letterSpacing: 0.3,
        textTransform: 'uppercase',
      }}
    >
      {label}
    </span>
  );
}

function Separator() {
  return (
    <span aria-hidden="true" style={{ opacity: 0.4 }}>
      ·
    </span>
  );
}

function buildVerdictReason(
  counts: ArchivedSessionSummary['verdict_counts'],
): string | null {
  if (!counts) return null;
  const parts: string[] = [];
  if (counts.pass) parts.push(`${counts.pass} pass`);
  if (counts.fail) parts.push(`${counts.fail} fail`);
  if (counts.inconclusive) parts.push(`${counts.inconclusive} inconclusive`);
  if (counts.error) parts.push(`${counts.error} error`);
  return parts.length ? parts.join(' · ') : null;
}

/**
 * Hook — detects `prefers-reduced-motion: reduce` and updates on change.
 * SSR-safe: returns `false` when `window` or `matchMedia` is absent.
 */
function usePrefersReducedMotion(): boolean {
  const [reduce, setReduce] = useState<boolean>(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return false;
    }
    try {
      return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    } catch {
      return false;
    }
  });
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }
    let mql: MediaQueryList;
    try {
      mql = window.matchMedia('(prefers-reduced-motion: reduce)');
    } catch {
      return;
    }
    const onChange = (event: MediaQueryListEvent) => setReduce(event.matches);
    if (typeof mql.addEventListener === 'function') {
      mql.addEventListener('change', onChange);
      return () => mql.removeEventListener('change', onChange);
    }
    // Legacy Safari fallback (only triggers when the modern API is missing).
    const legacy = mql as unknown as {
      addListener?: (cb: (event: MediaQueryListEvent) => void) => void;
      removeListener?: (cb: (event: MediaQueryListEvent) => void) => void;
    };
    legacy.addListener?.(onChange);
    return () => legacy.removeListener?.(onChange);
  }, []);
  return reduce;
}
