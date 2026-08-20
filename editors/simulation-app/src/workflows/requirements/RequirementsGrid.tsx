/**
 * RequirementsGrid — grid mode of the document-shaped table (demo 1a/1c).
 *
 * The `method` column (B4, 2026-07-17) shows the DECLARED
 * @VerificationMethod union off the row's verifying cases — model
 * intent, deliberately neutral-styled so it never reads as a verdict
 * (and never as `evaluation_mode`). The `⚑` suspect column (v1.5) renders only
 * when a baseline is selected: a permanently-empty column would be
 * noise, and without a baseline "suspect" has no referent.
 *
 * Row scale: 24px data rows, 32px package dividers (contiguous
 * document-order runs — the outline contract). Selected row = accent
 * ink-bar + tint, the one current-item rendering (visual gate ruling).
 * Suspect ⚑ = bare warning-olive glyph, no pill (chip-family ruling);
 * clicking it opens the anchored popover (never a modal).
 *
 * Inline editing (v2 §7.5): double-click the text cell (doc statement)
 * or maturity chip enters edit in place; each group ends in the
 * `+ add …` guided-create row. Pending/failed cell badges ride the
 * six-step writeback loop in `useFieldEdit` — ONE edit in flight.
 */

import { useState } from 'react';
import type { CSSProperties } from 'react';
import type { SuspectRecord } from '@/features/baselines/suspect';
import type { RequirementGroup } from '@/features/requirements/rollup';
import { rowDisplayId } from '@/features/requirements/rollup';
import type { RequirementRow } from '@/features/requirements/types';
import { cellKey, useRequirementEditStore } from '@/features/requirements/editStore';
import { useRequirementCellEdit } from '@/features/requirements/useFieldEdit';
import { MaturityChip, MethodChip, VerifiedChip } from './RequirementChips';
import { SuspectPopover } from './SuspectPopover';
import { CellBadge, CellErrorLine, InlineTextEditor, MaturitySelect } from './InlineEditors';
import { AddRequirementRow } from './CreateRequirementPopover';

const ID_COL: CSSProperties = { width: 140, flex: 'none', padding: '0 12px' };
const MATURITY_COL: CSSProperties = { width: 90, flex: 'none' };
const METHOD_COL: CSSProperties = { width: 110, flex: 'none', paddingRight: 8 };
const VERIFIED_COL: CSSProperties = { width: 90, flex: 'none' };
const SUSPECT_COL: CSSProperties = { width: 44, flex: 'none', textAlign: 'center' };

export interface RequirementsGridProps {
  groups: RequirementGroup[];
  selectedId: string | null;
  onSelect: (row: RequirementRow) => void;
  /** Suspect records keyed by requirement id; null = no baseline selected
   *  (⚑ column hidden entirely). */
  suspects?: Map<string, SuspectRecord> | null;
  /** Name of the baseline the suspects were computed against. */
  baselineName?: string | null;
}

export function RequirementsGrid({
  groups,
  selectedId,
  onSelect,
  suspects = null,
  baselineName = null,
}: RequirementsGridProps) {
  const showSuspect = suspects !== null && baselineName !== null;
  // Row id whose suspect popover is open (anchored to its ⚑ cell).
  const [openSuspectId, setOpenSuspectId] = useState<string | null>(null);

  // Cell-edit machinery (§7.5): double-click enters edit; ONE edit in
  // flight — beginEdit refuses while a commit is pending.
  const editingKey = useRequirementEditStore((s) => s.editingKey);
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitDoc, commitMaturity } = useRequirementCellEdit();

  return (
    <div
      data-testid="requirements-grid"
      style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          height: 'var(--row-compact)',
          borderBottom: '1px solid var(--border-default)',
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          color: 'var(--text-muted)',
          position: 'sticky',
          top: 0,
          background: 'var(--surface-canvas)',
          zIndex: 1,
        }}
      >
        <span style={ID_COL}>id</span>
        <span style={{ flex: 1 }}>requirement</span>
        <span style={MATURITY_COL}>maturity</span>
        <span
          style={METHOD_COL}
          title="Declared verification method (@VerificationMethod on the verifying cases) — model intent, distinct from how the Verified rollup was computed"
        >
          method
        </span>
        <span
          style={VERIFIED_COL}
          title="Verified rollup — filled red: contains a recorded fail · outline: incomplete (not all cases run) · filled green: all cases passed · —: no cases linked"
        >
          verified
        </span>
        {showSuspect && (
          <span
            style={SUSPECT_COL}
            data-testid="requirements-suspect-header"
            title={`Suspect — changed since baseline ${baselineName}`}
          >
            ⚑
          </span>
        )}
      </div>
      {groups.map((group, gi) => (
        <div key={`${group.packageId ?? 'none'}-${gi}`}>
          <div
            data-testid="requirements-grid-package"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              height: 'var(--row-default)',
              borderBottom: '1px solid var(--border-hairline)',
              background: 'var(--surface-panel)',
              fontSize: 'var(--text-sm)',
              color: 'var(--text-secondary)',
            }}
          >
            <span style={{ padding: '0 12px', fontWeight: 500 }}>{group.label}</span>
            <span
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--text-xs)',
                color: 'var(--text-muted)',
              }}
            >
              {group.rows.length}
            </span>
          </div>
          {group.rows.map((row) => {
            const selected = row.id === selectedId;
            const suspect = showSuspect ? suspects.get(row.id) ?? null : null;
            const docKey = cellKey(row.id, 'doc');
            const maturityKey = cellKey(row.id, 'maturity');
            const editingDoc = editingKey === docKey;
            const editingMaturity = editingKey === maturityKey;
            const rowFailure =
              failed && (failed.key === docKey || failed.key === maturityKey)
                ? failed.message
                : null;
            return (
              <div
                key={row.id}
                data-testid={`req-row-${rowDisplayId(row)}`}
                role="row"
                aria-selected={selected}
                // Rail link-chip navigation selects rows the user hasn't
                // scrolled to — keep the selected row visible.
                ref={
                  selected
                    ? (el) => el?.scrollIntoView?.({ block: 'nearest' })
                    : undefined
                }
                onClick={() => onSelect(row)}
                style={{
                  display: 'flex',
                  // Editors and error lines grow the row; the compact
                  // height is a floor, not a fixed size, while editing.
                  alignItems: editingDoc || rowFailure ? 'flex-start' : 'center',
                  minHeight: 'var(--row-compact)',
                  ...(editingDoc || rowFailure ? { padding: '4px 0' } : { height: 'var(--row-compact)' }),
                  borderBottom: '1px solid var(--border-hairline)',
                  cursor: 'pointer',
                  // Anchor context for the suspect popover's absolute
                  // positioning (beak points at this row's ⚑ cell).
                  position: 'relative',
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
                    ...ID_COL,
                    fontFamily: 'var(--font-mono)',
                    fontSize: 11.5,
                    // Calm pass (P1): the id is the row's durable identifier —
                    // the one bright anchor (primary), so the eye lands on it
                    // and the muted statement recedes beside it.
                    color: 'var(--text-primary)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    // Hierarchy reads off the LEADING edge: indent the
                    // clause id (a standards document indents its clause
                    // numbers) — indenting the mid-row statement text
                    // renders invisibly next to variable-length ids.
                    // 12 = ID_COL's own base padding (overridden here).
                    paddingLeft: 12 + row.outline_depth * 16,
                  }}
                >
                  {row.outline_depth > 0 && (
                    <span
                      aria-hidden
                      style={{ color: 'var(--text-disabled)', marginRight: 4 }}
                    >
                      └
                    </span>
                  )}
                  {rowDisplayId(row)}
                </span>
                <span
                  data-testid={`req-text-cell-${rowDisplayId(row)}`}
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    beginEdit(docKey);
                  }}
                  style={{
                    flex: 1,
                    fontSize: 'var(--text-sm)',
                    // Calm pass (P1): the statement recedes to muted so the
                    // element NAME (rendered primary+500 below) is the row's
                    // one bright anchor; selection lifts the whole cell.
                    color: selected ? 'var(--text-primary)' : 'var(--text-muted)',
                    paddingRight: 12,
                    ...(editingDoc || rowFailure
                      ? { minWidth: 0 }
                      : {
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }),
                  }}
                >
                  {editingDoc ? (
                    <InlineTextEditor
                      data-testid="req-doc-editor"
                      initial={row.text ?? ''}
                      multiline
                      placeholder="statement text (a doc comment)"
                      onCommit={(text) => commitDoc(row, text)}
                      onCancel={cancelEdit}
                    />
                  ) : (
                    <>
                      {/* The element NAME leads (links/derivation chips speak
                          in names — without it here a chip's target can't be
                          found in the table); statement text follows. When
                          the display id already IS the name, don't repeat. */}
                      {row.name && row.name !== rowDisplayId(row) && (
                        <span
                          style={{
                            color: 'var(--text-primary)',
                            fontWeight: 500,
                            marginRight: 8,
                          }}
                        >
                          {row.name}
                        </span>
                      )}
                      {row.text ?? row.qualified_name ?? ''}
                      {pendingKey === docKey && <CellBadge state="pending" />}
                      {failed?.key === docKey && (
                        <CellBadge state="failed" message={failed.message} />
                      )}
                      {rowFailure && <CellErrorLine message={rowFailure} />}
                    </>
                  )}
                </span>
                <span
                  style={MATURITY_COL}
                  data-testid={`req-maturity-cell-${rowDisplayId(row)}`}
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    beginEdit(maturityKey);
                  }}
                  onClick={(e) => {
                    // A chip is visibly a control: single click opens the
                    // select when the row is already selected (§7.5).
                    if (selected) {
                      e.stopPropagation();
                      beginEdit(maturityKey);
                    }
                  }}
                >
                  {editingMaturity ? (
                    <MaturitySelect
                      data-testid="req-maturity-select"
                      initial={row.maturity}
                      onCommit={(status) => commitMaturity(row, status)}
                      onCancel={cancelEdit}
                    />
                  ) : (
                    <>
                      <MaturityChip maturity={row.maturity} />
                      {pendingKey === maturityKey && <CellBadge state="pending" />}
                      {failed?.key === maturityKey && (
                        <CellBadge state="failed" message={failed.message} />
                      )}
                    </>
                  )}
                </span>
                <span style={METHOD_COL}>
                  <MethodChip methods={row.verification_methods} />
                </span>
                <span style={VERIFIED_COL}>
                  <VerifiedChip rollup={row.verification} />
                </span>
                {showSuspect && (
                  <span style={SUSPECT_COL}>
                    {suspect && (
                      <button
                        type="button"
                        data-testid={`suspect-flag-${rowDisplayId(row)}`}
                        aria-label={`Changed since baseline ${baselineName}`}
                        title={suspect.changeSummary}
                        onClick={(e) => {
                          e.stopPropagation();
                          setOpenSuspectId((cur) => (cur === row.id ? null : row.id));
                        }}
                        style={{
                          border: 'none',
                          background: 'transparent',
                          color: 'var(--severity-warning)',
                          fontSize: 11,
                          cursor: 'pointer',
                          padding: 0,
                          lineHeight: 1,
                        }}
                      >
                        ⚑
                      </button>
                    )}
                    {suspect && openSuspectId === row.id && baselineName && (
                      <SuspectPopover
                        row={row}
                        record={suspect}
                        baselineName={baselineName}
                        onClose={() => setOpenSuspectId(null)}
                      />
                    )}
                  </span>
                )}
              </div>
            );
          })}
          <AddRequirementRow parentId={group.packageId} parentLabel={group.label} />
        </div>
      ))}
    </div>
  );
}
