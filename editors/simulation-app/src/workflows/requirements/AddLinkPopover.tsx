/**
 * AddLinkRow / AddLinkPopover — the R5 `+ add` affordance per writable
 * links-rail group (workbench design §7.6). Anchored popover (create-view
 * posture, CreateRequirementPopover pattern) hosting a FuzzyCombobox over
 * kind-filtered USER-AUTHORED candidates (`sysml.query` + the
 * `user_authored` filter — stdlib internals are never link targets).
 *
 * Commit routes through the same six-step splice loop as cell edits; the
 * computed edit lands in the PICKED element's file (cross-file is inherent
 * — satisfy writes into the part's body, verify into the case's objective
 * — and the loop is uri-agnostic). ONE edit in flight; the pending/failed
 * badge rides this row's group. Free-form text never commits: the input
 * must resolve to a known candidate (creating targets is a different act).
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';
import {
  useLinkTargetCandidates,
  type LinkTargetCandidate,
} from '@/features/requirements/queries';
import { cellKey, useRequirementEditStore } from '@/features/requirements/editStore';
import { CellBadge, CellErrorLine } from './InlineEditors';

const FIELD_INPUT: CSSProperties = {
  width: '100%',
  boxSizing: 'border-box',
  background: 'var(--surface-canvas)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '4px 6px',
  fontSize: 'var(--text-sm)',
  outline: 'none',
};

export interface AddLinkRowProps {
  /** Group label fragment: `+ add <label>`. */
  label: string;
  /** The selected requirement row (badge anchor + edit-store key). */
  rowId: string;
  /** Cell-key field, one per group (`link_satisfy` | `link_verify` | `link_derive` …). */
  field: string;
  /** Candidate element kinds (`sysml.query` kind filter). */
  kinds: string[];
  /** Candidates never offered: self + already-linked targets (the backend
   *  fails hard on duplicates anyway — this keeps the noise out). */
  excludeIds: string[];
  /** Commit with the picked candidate's element id. */
  onCommit: (targetId: string) => void;
}

/** The quiet `+ add …` row a links group ends in, owning its popover. */
export function AddLinkRow({ label, rowId, field, kinds, excludeIds, onCommit }: AddLinkRowProps) {
  const [open, setOpen] = useState(false);
  const key = cellKey(rowId, field);
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);

  const pending = pendingKey === key;
  const failure = failed?.key === key ? failed.message : null;

  return (
    <div style={{ position: 'relative' }}>
      <button
        type="button"
        data-testid={`req-add-link-${field}`}
        onClick={() => {
          if (open) {
            setOpen(false);
            cancelEdit();
          } else if (beginEdit(key)) {
            setOpen(true);
          }
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          border: 'none',
          background: 'transparent',
          color: 'var(--text-muted)',
          fontSize: 'var(--text-xs)',
          cursor: 'pointer',
          padding: '2px 0',
          textAlign: 'left',
        }}
      >
        + add {label}
        {pending && <CellBadge state="pending" />}
        {failure && <CellBadge state="failed" message={failure} />}
      </button>
      {failure && <CellErrorLine message={failure} />}
      {open && (
        <AddLinkPopover
          label={label}
          kinds={kinds}
          excludeIds={excludeIds}
          onCommit={(id) => {
            setOpen(false);
            onCommit(id);
          }}
          onClose={() => {
            setOpen(false);
            cancelEdit();
          }}
        />
      )}
    </div>
  );
}

interface PickerOption {
  value: string;
  detail?: string;
  id: string;
}

/** Candidate → combobox option. Value is the simple name; when two
 *  candidates share one, both fall back to their qualified name so the
 *  value → id mapping stays unambiguous. */
export function buildPickerOptions(
  candidates: LinkTargetCandidate[],
  excludeIds: string[],
): PickerOption[] {
  const excluded = new Set(excludeIds);
  const counts = new Map<string, number>();
  for (const c of candidates) {
    const n = c.name ?? c.id;
    counts.set(n, (counts.get(n) ?? 0) + 1);
  }
  return candidates
    .filter((c) => !excluded.has(c.id))
    .map((c) => {
      const name = c.name ?? c.id;
      const ambiguous = (counts.get(name) ?? 0) > 1;
      const value = ambiguous && c.qualified_name ? c.qualified_name : name;
      const detail =
        c.qualified_name && c.qualified_name !== value ? c.qualified_name : undefined;
      return { value, detail, id: c.id };
    });
}

export function AddLinkPopover({
  label,
  kinds,
  excludeIds,
  onCommit,
  onClose,
}: {
  label: string;
  kinds: string[];
  excludeIds: string[];
  onCommit: (targetId: string) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);
  const candidates = useLinkTargetCandidates(kinds);

  const options = useMemo(
    () => buildPickerOptions(candidates.data ?? [], excludeIds),
    [candidates.data, excludeIds],
  );
  const fuzzy: FuzzyCandidate[] = useMemo(
    () => options.map((o) => (o.detail ? { value: o.value, detail: o.detail } : o.value)),
    [options],
  );
  const picked = options.find((o) => o.value === value) ?? null;

  // Light-dismiss (BaselinePicker pattern).
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [onClose]);

  const submit = () => {
    if (picked) onCommit(picked.id);
  };

  return (
    <div
      ref={rootRef}
      data-testid={`add-link-popover-${label.replace(/\s+/g, '-')}`}
      role="dialog"
      aria-label={`Add ${label}`}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
        // Enter commits once a candidate is picked — the combobox
        // swallows Enter while its suggestion list is open.
        if (e.key === 'Enter') submit();
      }}
      style={{
        position: 'absolute',
        left: 0,
        top: 'calc(100% + 4px)',
        width: 280,
        background: 'var(--surface-panel)',
        border: '1px solid var(--border-default)',
        borderRadius: 12,
        boxShadow: '0 4px 16px rgba(0,0,0,0.4)',
        padding: 12,
        zIndex: 3,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          color: 'var(--text-muted)',
        }}
      >
        add {label}
      </div>
      <FuzzyCombobox
        value={value}
        onChange={setValue}
        candidates={fuzzy}
        placeholder={candidates.isLoading ? 'loading…' : 'type to search'}
        testId={`add-link-input-${label.replace(/\s+/g, '-')}`}
        inputStyle={FIELD_INPUT}
      />
      {candidates.isError && <CellErrorLine message="candidates unavailable" />}
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <button
          type="button"
          onClick={onClose}
          style={{
            border: '1px solid var(--border-default)',
            background: 'transparent',
            color: 'var(--text-secondary)',
            borderRadius: 4,
            padding: '3px 10px',
            fontSize: 'var(--text-sm)',
            cursor: 'pointer',
          }}
        >
          cancel
        </button>
        <button
          type="button"
          data-testid={`add-link-submit-${label.replace(/\s+/g, '-')}`}
          disabled={picked === null}
          onClick={submit}
          style={{
            border: '1px solid var(--accent)',
            background: picked ? 'var(--accent-tint)' : 'transparent',
            color: picked ? 'var(--text-primary)' : 'var(--text-disabled)',
            borderRadius: 4,
            padding: '3px 10px',
            fontSize: 'var(--text-sm)',
            cursor: picked ? 'pointer' : 'default',
          }}
        >
          add
        </button>
      </div>
    </div>
  );
}
