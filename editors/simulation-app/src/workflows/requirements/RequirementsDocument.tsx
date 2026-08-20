/**
 * RequirementsDocument — document mode of the table (demo 1b): the same
 * rows as flowing prose. One item store, two renderings — selection is
 * shared with grid mode, so toggling preserves it.
 *
 * Anatomy per the demo: centered 860px column · 32px package section
 * heading with a mono "§N · count" context value · 110px right-aligned
 * mono ID margin column · 13.5px/1.55 prose from `row.text` · a single
 * inline chip line beneath.
 *
 * Inline editing (v2 §7.5, demo 1b's REQ-SENS-03 affordance): double-
 * click grows the paragraph itself into the editor — border + tint +
 * caret, never a dialog. Groups end in the `+ add …` guided-create row.
 */

import type { RequirementGroup } from '@/features/requirements/rollup';
import { rowDisplayId } from '@/features/requirements/rollup';
import type { RequirementRow } from '@/features/requirements/types';
import { cellKey, useRequirementEditStore } from '@/features/requirements/editStore';
import { useRequirementCellEdit } from '@/features/requirements/useFieldEdit';
import { MaturityChip, VerifiedChip } from './RequirementChips';
import { CellBadge, CellErrorLine, InlineTextEditor, MaturitySelect } from './InlineEditors';
import { AddRequirementRow } from './CreateRequirementPopover';

export interface RequirementsDocumentProps {
  groups: RequirementGroup[];
  selectedId: string | null;
  onSelect: (row: RequirementRow) => void;
}

export function RequirementsDocument({
  groups,
  selectedId,
  onSelect,
}: RequirementsDocumentProps) {
  const editingKey = useRequirementEditStore((s) => s.editingKey);
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitDoc, commitMaturity } = useRequirementCellEdit();
  return (
    <div
      data-testid="requirements-document"
      style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '20px 0' }}
    >
      <div style={{ maxWidth: 860, margin: '0 auto', padding: '0 32px' }}>
        {groups.map((group, gi) => (
          <div key={`${group.packageId ?? 'none'}-${gi}`}>
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                gap: 10,
                height: 'var(--row-default)',
                borderBottom: '1px solid var(--border-default)',
                margin: gi === 0 ? '0 0 8px' : '16px 0 8px',
              }}
            >
              <span style={{ fontSize: 15, fontWeight: 500, color: 'var(--text-primary)' }}>
                {group.label}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--text-xs)',
                  color: 'var(--text-muted)',
                }}
              >
                {group.rows.length} requirements · §{gi + 1}
              </span>
            </div>
            {group.rows.map((row) => {
              const selected = row.id === selectedId;
              const docKey = cellKey(row.id, 'doc');
              const editingDoc = editingKey === docKey;
              const docFailure = failed?.key === docKey ? failed.message : null;
              const maturityKey = cellKey(row.id, 'maturity');
              const editingMaturity = editingKey === maturityKey;
              const maturityFailure = failed?.key === maturityKey ? failed.message : null;
              return (
                <div
                  key={row.id}
                  data-testid={`req-doc-${rowDisplayId(row)}`}
                  onClick={() => onSelect(row)}
                  style={{
                    display: 'flex',
                    gap: 20,
                    padding: '10px 0',
                    cursor: 'pointer',
                    ...(selected
                      ? {
                          background: 'var(--accent-tint)',
                          boxShadow: 'inset 2px 0 0 var(--accent)',
                        }
                      : {}),
                  }}
                >
                  <span
                    style={{
                      width: 110,
                      flex: 'none',
                      textAlign: 'right',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--text-xs)',
                      color: 'var(--text-secondary)',
                      paddingTop: 2,
                    }}
                  >
                    {rowDisplayId(row)}
                  </span>
                  <div style={{ flex: 1, minWidth: 0, paddingLeft: row.outline_depth * 16 }}>
                    {row.name && row.name !== rowDisplayId(row) && (
                      <div
                        style={{
                          fontSize: 13.5,
                          fontWeight: 600,
                          lineHeight: 1.55,
                          color: 'var(--text-primary)',
                        }}
                      >
                        {row.name}
                      </div>
                    )}
                    {editingDoc ? (
                      <div style={{ fontSize: 13.5 }}>
                        <InlineTextEditor
                          data-testid="req-doc-editor"
                          initial={row.text ?? ''}
                          multiline
                          placeholder="statement text (a doc comment)"
                          onCommit={(text) => commitDoc(row, text)}
                          onCancel={cancelEdit}
                        />
                      </div>
                    ) : (
                      <div
                        data-testid={`req-doc-text-${rowDisplayId(row)}`}
                        onDoubleClick={(e) => {
                          e.stopPropagation();
                          beginEdit(docKey);
                        }}
                        style={{
                          fontSize: 13.5,
                          lineHeight: 1.55,
                          color: 'var(--text-primary)',
                          whiteSpace: 'pre-line',
                        }}
                      >
                        {row.text ?? (
                          <span style={{ color: 'var(--text-muted)', fontStyle: 'italic' }}>
                            (no statement text — double-click to add one)
                          </span>
                        )}
                        {pendingKey === docKey && <CellBadge state="pending" />}
                        {docFailure && <CellBadge state="failed" message={docFailure} />}
                      </div>
                    )}
                    {!editingDoc && docFailure && <CellErrorLine message={docFailure} />}
                    <div
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 10,
                        marginTop: 6,
                      }}
                    >
                      <span
                        data-testid={`req-doc-maturity-${rowDisplayId(row)}`}
                        onDoubleClick={(e) => {
                          e.stopPropagation();
                          beginEdit(maturityKey);
                        }}
                        onClick={(e) => {
                          // A chip is visibly a control: single click opens the
                          // select when the row is already selected (§7.5) —
                          // parity with the grid's maturity cell.
                          if (selected) {
                            e.stopPropagation();
                            beginEdit(maturityKey);
                          }
                        }}
                        style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}
                      >
                        {editingMaturity ? (
                          <MaturitySelect
                            data-testid="req-doc-maturity-select"
                            initial={row.maturity}
                            onCommit={(status) => commitMaturity(row, status)}
                            onCancel={cancelEdit}
                          />
                        ) : (
                          <>
                            <MaturityChip maturity={row.maturity} />
                            {pendingKey === maturityKey && <CellBadge state="pending" />}
                            {maturityFailure && (
                              <CellBadge state="failed" message={maturityFailure} />
                            )}
                          </>
                        )}
                      </span>
                      <VerifiedChip rollup={row.verification} />
                    </div>
                    {!editingMaturity && maturityFailure && (
                      <CellErrorLine message={maturityFailure} />
                    )}
                  </div>
                </div>
              );
            })}
            <AddRequirementRow parentId={group.packageId} parentLabel={group.label} />
          </div>
        ))}
      </div>
    </div>
  );
}
