/**
 * VerifyCasesList — the Cases sub-view list (Verify design 1a, left column).
 *
 * One row per verification case: a verdict glyph (ƒ — the tool's read), the
 * case name, its per-case evaluation mode, and a check count. Clicking a row
 * opens the case document (`VerifyCaseView`). Verdict colour appears ONLY on
 * the verdict glyph (§5.1); the mode glyph is neutral.
 *
 * Rendered inside the ninebar left-rail slot when the Cases sub-view is
 * active — it replaces the run-config rail (the case list IS the navigation
 * for this sub-view).
 */

import type { CSSProperties } from 'react';
import {
  caseIdOf,
  isBareObjectiveRow,
  normalizeCaseVerdict,
  type VerificationCaseRow,
} from './useVerificationCases';
import { normalizeEvaluationMode } from '@/components/EvaluationModeBadge';
import type { VerdictKind } from '@/components/VerdictBadge';

export interface VerifyCasesListProps {
  cases: VerificationCaseRow[];
  isLoading?: boolean;
  isError?: boolean;
  hasWorkspace: boolean;
  /** The currently open case (its id, per `caseIdOf`). */
  selectedCaseId: string | null;
  /** Open a case document. */
  onSelectCase: (caseId: string) => void;
  /** Current-model digest for the footer provenance note; absent ⇒ omitted. */
  modelDigest?: string;
}

export function VerifyCasesList({
  cases,
  isLoading = false,
  isError = false,
  hasWorkspace,
  selectedCaseId,
  onSelectCase,
  modelDigest,
}: VerifyCasesListProps) {
  return (
    <div data-testid="verify-cases-list" className="flex flex-col h-full min-h-0" style={{ color: 'var(--text-primary)' }}>
      <div
        className="flex items-center px-3 shrink-0"
        style={{ height: 24, fontSize: 10.5, color: 'var(--text-muted)', borderBottom: '1px solid var(--border-hairline)' }}
      >
        cases · verdict ƒ · mode
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto">
        {!hasWorkspace ? (
          <Empty text="No workspace loaded" />
        ) : isLoading ? (
          <Empty text="Discovering cases…" />
        ) : isError ? (
          <Empty text="Failed to load verification cases" tone="error" />
        ) : cases.length === 0 ? (
          <Empty text="No verification cases" />
        ) : (
          <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
            {cases.map((row) => {
              const id = caseIdOf(row);
              return (
                <CaseRow
                  key={id}
                  row={row}
                  selected={id === selectedCaseId}
                  onClick={() => onSelectCase(id)}
                />
              );
            })}
          </ul>
        )}
      </div>

      <div
        className="shrink-0"
        style={{ borderTop: '1px solid var(--border-hairline)', padding: '8px 12px', fontSize: 10.5, color: 'var(--text-muted)', lineHeight: 1.5 }}
      >
        verdicts ƒ are the tool’s read
        {modelDigest ? (
          <>
            {' '}at <span className="mono-text" title={modelDigest}>{modelDigest.slice(0, 7)}</span>
          </>
        ) : null}{' '}
        · static verdicts recompute on every model edit
      </div>
    </div>
  );
}

function CaseRow({ row, selected, onClick }: { row: VerificationCaseRow; selected: boolean; onClick: () => void }) {
  const name = row.case_name ?? row.case_id ?? '(anonymous)';
  const bare = isBareObjectiveRow(row);
  const verdict: VerdictKind | null = bare ? null : normalizeCaseVerdict(row.verdict);
  const mode = normalizeEvaluationMode(row.evaluation_mode);
  const total = typeof row.total_requirements === 'number' ? row.total_requirements : null;

  return (
    <li>
      <button
        type="button"
        data-testid={`verify-cases-row-${name}`}
        data-selected={selected || undefined}
        aria-current={selected || undefined}
        onClick={onClick}
        className="flex items-center gap-2 px-3 w-full"
        style={{
          height: 32,
          border: 'none',
          borderBottom: '1px solid var(--border-hairline)',
          background: selected ? 'var(--surface-raised)' : 'transparent',
          boxShadow: selected ? 'inset 2px 0 0 var(--accent)' : 'none',
          color: selected ? 'var(--text-primary)' : 'var(--text-secondary)',
          cursor: 'pointer',
          textAlign: 'left',
        }}
      >
        <CaseVerdictGlyph verdict={verdict} />
        <span className="mono-text truncate" style={{ fontSize: 11.5, flex: 1, minWidth: 0 }}>{name}</span>
        {total != null && total > 0 ? (
          <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-muted)' }}>
            {total} req
          </span>
        ) : null}
        {mode ? (
          <span
            className="mono-text"
            title={`evaluation mode: ${mode}`}
            aria-label={`evaluation mode: ${mode}`}
            style={{ fontSize: 10, color: 'var(--text-muted)', width: 12, textAlign: 'center' }}
          >
            {MODE_GLYPH[mode]}
          </span>
        ) : null}
      </button>
    </li>
  );
}

/** A small round verdict glyph — verdict colour lives ONLY here (§5.1). A
 *  bare objective mints no verdict, so it renders a neutral em-dash (1e). */
function CaseVerdictGlyph({ verdict }: { verdict: VerdictKind | null }) {
  if (!verdict) {
    return (
      <span aria-label="no verdict — bare objective" title="no verdict is minted for a case with no checks" style={{ width: 14, textAlign: 'center', color: 'var(--text-muted)', fontSize: 10 }}>
        —
      </span>
    );
  }
  const t = VERDICT_GLYPH[verdict];
  return (
    <span
      data-verdict={verdict}
      aria-label={`verdict: ${verdict}`}
      title={`verdict: ${verdict}`}
      className="mono-text"
      style={{
        width: 14,
        flex: 'none',
        textAlign: 'center',
        fontSize: 10,
        borderRadius: 999,
        color: t.solid ? 'var(--text-inverse)' : t.color,
        background: t.solid ? t.color : 'transparent',
        border: t.solid ? 'none' : `1px solid ${t.color}`,
        lineHeight: '12px',
      }}
    >
      {t.glyph}
    </span>
  );
}

const VERDICT_GLYPH: Record<VerdictKind, { glyph: string; color: string; solid: boolean }> = {
  pass: { glyph: '✓', color: 'var(--verdict-pass)', solid: true },
  fail: { glyph: '✗', color: 'var(--verdict-fail)', solid: true },
  inconclusive: { glyph: '?', color: 'var(--verdict-inconclusive)', solid: false },
  error: { glyph: '⨯', color: 'var(--verdict-error)', solid: false },
};

const MODE_GLYPH: Record<'static' | 'trajectory' | 'external', string> = {
  static: '=',
  trajectory: '∿',
  external: '↓',
};

function Empty({ text, tone }: { text: string; tone?: 'error' }) {
  return (
    <div className="px-3 py-4" style={{ fontSize: 11, color: tone === 'error' ? 'var(--verdict-fail)' : 'var(--text-muted)' }}>
      {text}
    </div>
  );
}
