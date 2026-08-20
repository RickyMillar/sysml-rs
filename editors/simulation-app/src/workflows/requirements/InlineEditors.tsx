/**
 * Inline cell editors + writeback badges (workbench design §7.5).
 *
 * The text editor is the crib-sheet affordance verbatim: the content
 * grows a border + tint + caret — never a dialog — with the two mono
 * hint lines (`unsaved — editor owns save` is verbatim from the brief).
 * Keyboard: Esc discards everywhere; multiline commits on ⌘Enter/⌘S
 * (plain Enter inserts a newline), single-line commits on Enter.
 *
 * Badges are the §4 "optimistic display + mono badge" states: pending =
 * amber (active family), failed = severity-error with the service
 * message. Confirmed has no badge — quiet is the confirmation.
 */

import { useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { STATUS_KINDS } from '@/features/requirements/fieldEdit';

const HINT_ROW: CSSProperties = {
  display: 'flex',
  gap: 10,
  marginTop: 4,
  fontFamily: 'var(--font-mono)',
  fontSize: 10,
};

export interface InlineTextEditorProps {
  initial: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
  /** Single-line: Enter commits (attribute values). Multiline (doc
   *  prose): Enter inserts a newline, ⌘Enter/⌘S commit. */
  multiline?: boolean;
  placeholder?: string;
  'data-testid'?: string;
}

export function InlineTextEditor({
  initial,
  onCommit,
  onCancel,
  multiline = false,
  placeholder,
  'data-testid': testid,
}: InlineTextEditorProps) {
  const [value, setValue] = useState(initial);
  const ref = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    ref.current?.focus();
    ref.current?.setSelectionRange(value.length, value.length);
    // Focus + caret-to-end on mount only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const commit = () => {
    const next = value.trim();
    if (next === '' || next === initial) {
      onCancel();
      return;
    }
    onCommit(next);
  };

  return (
    <div onClick={(e) => e.stopPropagation()} style={{ flex: 1, minWidth: 0 }}>
      <textarea
        ref={ref}
        data-testid={testid}
        value={value}
        placeholder={placeholder}
        rows={multiline ? Math.max(2, value.split('\n').length) : 1}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            onCancel();
          } else if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            commit();
          } else if (e.key === 's' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            commit();
          }
        }}
        style={{
          width: '100%',
          boxSizing: 'border-box',
          resize: 'none',
          border: '1px solid var(--accent)',
          borderRadius: 4,
          background: 'var(--accent-tint)',
          color: 'var(--text-primary)',
          font: 'inherit',
          lineHeight: 1.55,
          padding: multiline ? '6px 8px' : '2px 6px',
          outline: 'none',
        }}
      />
      <div style={HINT_ROW}>
        <span style={{ color: 'var(--text-muted)' }}>unsaved — editor owns save</span>
        <span style={{ flex: 1 }} />
        <span style={{ color: 'var(--text-disabled)' }}>
          {multiline ? 'esc to discard · ⌘S writes back' : 'esc to discard · enter writes back'}
        </span>
      </div>
    </div>
  );
}

export interface MaturitySelectProps {
  initial: string | null;
  onCommit: (status: string) => void;
  onCancel: () => void;
  'data-testid'?: string;
}

/** Closed-vocab maturity picker (spec StatusKind — the write boundary
 *  re-enforces the vocabulary server-side). Commits on change. */
export function MaturitySelect({
  initial,
  onCommit,
  onCancel,
  'data-testid': testid,
}: MaturitySelectProps) {
  const ref = useRef<HTMLSelectElement | null>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return (
    <select
      ref={ref}
      data-testid={testid}
      value={initial ?? ''}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => {
        const next = e.target.value;
        if (next === '' || next === initial) onCancel();
        else onCommit(next);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
      onBlur={onCancel}
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        background: 'var(--surface-panel)',
        color: 'var(--text-primary)',
        border: '1px solid var(--accent)',
        borderRadius: 4,
        padding: '1px 4px',
      }}
    >
      <option value="" disabled>
        maturity…
      </option>
      {STATUS_KINDS.map((s) => (
        <option key={s} value={s}>
          {s}
        </option>
      ))}
    </select>
  );
}

export interface CellBadgeProps {
  state: 'pending' | 'failed';
  /** Failure message (shown as the badge title; callers should also
   *  surface it as an inline error line — never a silent revert). */
  message?: string;
}

/** The §4 mono writeback badge. */
export function CellBadge({ state, message }: CellBadgeProps) {
  const pending = state === 'pending';
  return (
    <span
      data-testid={pending ? 'req-cell-pending' : 'req-cell-failed'}
      title={pending ? 'writing back…' : message ?? 'writeback failed'}
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        marginLeft: 6,
        color: pending ? 'var(--accent-fg)' : 'var(--severity-error)',
      }}
    >
      {pending ? '…' : '⚠'}
    </span>
  );
}

/** Inline error line under a failed cell (§7.5: guard mismatches and
 *  service errors surface verbatim — never a silent revert). */
export function CellErrorLine({ message }: { message: string }) {
  return (
    <div
      data-testid="req-cell-error"
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        color: 'var(--severity-error)',
        marginTop: 2,
        whiteSpace: 'pre-wrap',
      }}
    >
      {message}
    </div>
  );
}
