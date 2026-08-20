/**
 * Guided-create popover (workbench design §7.5) — the `+ add …`
 * group-end affordance from demo 1a. Anchored overlay (12px radius per
 * the ninebar overlay ruling), never a modal: declared name → optional
 * REQ id (the spec's declaredShortName) → optional statement text.
 *
 * Create-view posture: pure-logic validation up front, loud failure —
 * the commit routes through the same six-step writeback loop as cell
 * edits (`sysml.workspace.create_requirement` computes an anchored
 * insertion; the client guard-splices it into the buffer).
 */

import { useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { isValidRequirementName } from '@/features/requirements/fieldEdit';
import { cellKey, useRequirementEditStore } from '@/features/requirements/editStore';
import { useRequirementCellEdit } from '@/features/requirements/useFieldEdit';
import { CellBadge, CellErrorLine } from './InlineEditors';

const FIELD_LABEL: CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: 10,
  color: 'var(--text-muted)',
  marginBottom: 2,
};

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

export interface AddRequirementRowProps {
  /** The group's package (create parent). Null when rows have no
   *  package ancestor — the affordance is hidden there (no parent to
   *  insert into; scratch-package creation is out of 2d scope). */
  parentId: string | null;
  parentLabel: string;
}

/** The quiet `+ add …` row a group ends in, owning its popover. */
export function AddRequirementRow({ parentId, parentLabel }: AddRequirementRowProps) {
  const [open, setOpen] = useState(false);
  const key = parentId ? cellKey(parentId, 'create') : null;
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);

  if (parentId === null || key === null) return null;
  const pending = pendingKey === key;
  const failure = failed?.key === key ? failed.message : null;

  return (
    <div style={{ position: 'relative' }}>
      <button
        type="button"
        data-testid={`req-add-${parentId}`}
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
          width: '100%',
          height: 'var(--row-compact)',
          padding: '0 12px',
          border: 'none',
          borderBottom: '1px solid var(--border-hairline)',
          background: 'transparent',
          color: 'var(--text-muted)',
          fontSize: 'var(--text-sm)',
          cursor: 'pointer',
          textAlign: 'left',
        }}
      >
        + add requirement
        {pending && <CellBadge state="pending" />}
        {failure && <CellBadge state="failed" message={failure} />}
      </button>
      {failure && <CellErrorLine message={failure} />}
      {open && (
        <CreateRequirementPopover
          parentId={parentId}
          parentLabel={parentLabel}
          onClose={() => {
            setOpen(false);
            cancelEdit();
          }}
        />
      )}
    </div>
  );
}

export function CreateRequirementPopover({
  parentId,
  parentLabel,
  onClose,
}: {
  parentId: string;
  parentLabel: string;
  onClose: () => void;
}) {
  const [name, setName] = useState('');
  const [shortName, setShortName] = useState('');
  const [doc, setDoc] = useState('');
  const [validationError, setValidationError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const { commitCreate } = useRequirementCellEdit();

  // Light-dismiss (BaselinePicker pattern).
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [onClose]);

  const submit = () => {
    const trimmed = name.trim();
    if (!isValidRequirementName(trimmed)) {
      setValidationError(
        trimmed === ''
          ? 'a requirement needs a declared name'
          : `"${trimmed}" is not a valid identifier`,
      );
      return;
    }
    commitCreate(parentId, trimmed, shortName.trim() || null, doc.trim() || null);
    onClose();
  };

  return (
    <div
      ref={rootRef}
      data-testid="create-requirement-popover"
      role="dialog"
      aria-label={`New requirement in ${parentLabel}`}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) submit();
      }}
      style={{
        position: 'absolute',
        left: 12,
        bottom: 'calc(var(--row-compact) + 4px)',
        width: 320,
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
        new requirement · {parentLabel}
      </div>
      <div>
        <div style={FIELD_LABEL}>name</div>
        <input
          data-testid="create-req-name"
          autoFocus
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            setValidationError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit();
          }}
          placeholder="TripTime"
          style={FIELD_INPUT}
        />
      </div>
      <div>
        <div style={FIELD_LABEL}>requirement id (optional)</div>
        <input
          data-testid="create-req-short-name"
          value={shortName}
          onChange={(e) => setShortName(e.target.value)}
          placeholder="REQ-TRIP-01"
          style={{ ...FIELD_INPUT, fontFamily: 'var(--font-mono)' }}
        />
      </div>
      <div>
        <div style={FIELD_LABEL}>statement (optional)</div>
        <textarea
          data-testid="create-req-doc"
          value={doc}
          onChange={(e) => setDoc(e.target.value)}
          rows={2}
          placeholder="The breaker shall…"
          style={{ ...FIELD_INPUT, resize: 'none', lineHeight: 1.55 }}
        />
      </div>
      {validationError && <CellErrorLine message={validationError} />}
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
          data-testid="create-req-submit"
          onClick={submit}
          style={{
            border: '1px solid var(--accent)',
            background: 'var(--accent-tint)',
            color: 'var(--text-primary)',
            borderRadius: 4,
            padding: '3px 10px',
            fontSize: 'var(--text-sm)',
            cursor: 'pointer',
          }}
        >
          create
        </button>
      </div>
    </div>
  );
}
