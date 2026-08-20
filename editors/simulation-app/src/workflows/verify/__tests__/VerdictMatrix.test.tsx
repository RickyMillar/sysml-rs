/**
 * VerdictMatrix — the ninebar Verify hero (Phase 4).
 *
 * Covers the seven cell states (crib §6), the pending → not-run/running
 * lifecycle rows, the filter tabs, the rollup, and cell selection.
 */

import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { Verdict, VerdictKind } from '@/engine/types';
import { VerdictMatrix } from '../VerdictMatrix';

afterEach(cleanup);

function mkVerdict(caseName: string, verdict: VerdictKind, extra: Partial<Verdict> = {}): Verdict {
  return {
    verdict,
    actual: null,
    expected: null,
    margin: null,
    error: null,
    sensitivity: null,
    evidence: null,
    metadata: { case_name: caseName, ...(extra.metadata ?? {}) },
    ...extra,
  };
}

describe('VerdictMatrix', () => {
  it('renders one row per case with the verdict cell state', () => {
    const verdicts = [
      mkVerdict('CaseA', 'pass'),
      mkVerdict('CaseB', 'fail'),
      mkVerdict('CaseC', 'inconclusive'),
      mkVerdict('CaseD', 'error'),
    ];
    render(<VerdictMatrix verdicts={verdicts} />);

    expect(screen.getByTestId('verdict-matrix-row-CaseA')).toBeTruthy();
    // The case-level column key is the synthetic '∙'.
    const passCell = screen.getByTestId('verdict-matrix-cell-CaseA-∙').querySelector('[data-cell-state]');
    expect(passCell?.getAttribute('data-cell-state')).toBe('pass');
    expect(
      screen.getByTestId('verdict-matrix-cell-CaseD-∙').querySelector('[data-cell-state="error"]'),
    ).toBeTruthy();
  });

  it('renders not-run rows for pending cases when idle', () => {
    render(<VerdictMatrix verdicts={[]} pendingCaseNames={['PendingCase']} isRunning={false} />);
    const cell = screen.getByTestId('verdict-matrix-cell-PendingCase-∙').querySelector('[data-cell-state]');
    expect(cell?.getAttribute('data-cell-state')).toBe('not-run');
  });

  it('renders the running ninebar for the case being evaluated', () => {
    render(
      <VerdictMatrix
        verdicts={[]}
        pendingCaseNames={['RunningCase']}
        runningCaseName="RunningCase"
        isRunning
      />,
    );
    const cell = screen.getByTestId('verdict-matrix-cell-RunningCase-∙').querySelector('[data-cell-state]');
    expect(cell?.getAttribute('data-cell-state')).toBe('running');
    // The running cell hosts the ninebar meter.
    expect(cell?.querySelector('.nb-meter')).toBeTruthy();
  });

  it('filters to failing rows only', () => {
    const verdicts = [mkVerdict('PassCase', 'pass'), mkVerdict('FailCase', 'fail')];
    render(<VerdictMatrix verdicts={verdicts} />);
    fireEvent.click(screen.getByTestId('verdict-matrix-filter-failing'));
    expect(screen.queryByTestId('verdict-matrix-row-FailCase')).toBeTruthy();
    expect(screen.queryByTestId('verdict-matrix-row-PassCase')).toBeNull();
  });

  it('rollup counts every verdict kind plus not-run', () => {
    const verdicts = [mkVerdict('A', 'pass'), mkVerdict('B', 'pass'), mkVerdict('C', 'fail')];
    render(<VerdictMatrix verdicts={verdicts} pendingCaseNames={['D']} />);
    const rollup = screen.getByTestId('verdict-matrix-rollup');
    expect(rollup.textContent).toContain('2'); // pass
    expect(rollup.textContent).toContain('not run');
  });

  it('shows each row its own computed mode in the per-row `mode ƒ` column (B10, §2.1a(d))', () => {
    const verdicts = [
      mkVerdict('A', 'pass', { metadata: { case_name: 'A', evaluation_mode: 'static' } }),
      mkVerdict('B', 'fail', { metadata: { case_name: 'B', evaluation_mode: 'static' } }),
    ];
    render(<VerdictMatrix verdicts={verdicts} />);
    // The run-level deduped header badges are gone — the header carries the
    // `mode ƒ` column label and each row carries its own badge.
    expect(screen.getByTestId('verdict-matrix-mode-header')).toBeTruthy();
    expect(screen.getByTestId('verdict-matrix-rowmode-badge-A').getAttribute('data-evaluation-mode')).toBe('static');
    expect(screen.getByTestId('verdict-matrix-rowmode-badge-B').getAttribute('data-evaluation-mode')).toBe('static');
  });

  it('renders a mixed-mode matrix cleanly — trajectory rows beside a static row', () => {
    const verdicts = [
      mkVerdict('A', 'pass', { metadata: { case_name: 'A', evaluation_mode: 'trajectory' } }),
      mkVerdict('B', 'pass', { metadata: { case_name: 'B', evaluation_mode: 'trajectory' } }),
      mkVerdict('C', 'inconclusive', { metadata: { case_name: 'C', evaluation_mode: 'static' } }),
    ];
    render(<VerdictMatrix verdicts={verdicts} />);
    expect(screen.getByTestId('verdict-matrix-rowmode-badge-A').getAttribute('data-evaluation-mode')).toBe('trajectory');
    expect(screen.getByTestId('verdict-matrix-rowmode-badge-C').getAttribute('data-evaluation-mode')).toBe('static');
  });

  it('renders no per-row mode badge when a row carries no evaluation_mode', () => {
    render(<VerdictMatrix verdicts={[mkVerdict('A', 'pass')]} />);
    const cell = screen.getByTestId('verdict-matrix-rowmode-A');
    expect(cell.querySelector('[data-evaluation-mode]')).toBeNull();
  });

  it('footnote states the row-mode meaning and the external-rollup boundary', () => {
    render(<VerdictMatrix verdicts={[mkVerdict('A', 'pass')]} />);
    const footnote = screen.getByTestId('verdict-matrix-mode-footnote');
    expect(footnote.textContent).toContain('how the shown verdict was computed');
    expect(footnote.textContent).toContain('external verdicts never enter this rollup');
  });

  it('renders the bare-objective teaching state (no verdict chip) for a case with no checks', () => {
    const bare = mkVerdict('BareCase', 'inconclusive', {
      metadata: { case_name: 'BareCase', evaluation_mode: 'static', total_requirements: 0 },
    });
    render(<VerdictMatrix verdicts={[bare]} />);
    const cell = screen.getByTestId('verdict-matrix-cell-BareCase-∙');
    // No verdict button/pill — the honest teaching absence instead.
    expect(cell.querySelector('[data-cell-state="bare-objective"]')).toBeTruthy();
    expect(cell.querySelector('[data-verdict-kind]')).toBeNull();
    // A bare objective mints no verdict → no fabricated mode badge on the row.
    expect(
      screen.getByTestId('verdict-matrix-rowmode-BareCase').querySelector('[data-evaluation-mode]'),
    ).toBeNull();
  });

  it('fires onSelect with the verdict when a completed cell is activated', () => {
    const onSelect = vi.fn();
    const v = mkVerdict('CaseA', 'fail');
    render(<VerdictMatrix verdicts={[v]} onSelect={onSelect} />);
    fireEvent.click(screen.getByTestId('verdict-matrix-cell-CaseA-∙').querySelector('button')!);
    expect(onSelect).toHaveBeenCalledWith(v);
  });

  it('marks the selected failing cell with the selection echo', () => {
    const v = mkVerdict('CaseA', 'fail');
    render(<VerdictMatrix verdicts={[v]} selectedVerdict={v} onSelect={() => {}} />);
    const btn = screen.getByTestId('verdict-matrix-cell-CaseA-∙').querySelector('button');
    expect(btn?.getAttribute('data-selected')).toBe('true');
  });

  it('groups constraint verdicts (no case_name) by element/display rather than collapsing', () => {
    const c1: Verdict = { verdict: 'pass', actual: null, expected: null, margin: null, error: null, sensitivity: null, evidence: null, metadata: { source: 'constraint', element_id: 'c1' } };
    const c2: Verdict = { verdict: 'fail', actual: null, expected: null, margin: null, error: null, sensitivity: null, evidence: null, metadata: { source: 'constraint', element_id: 'c2' } };
    render(<VerdictMatrix verdicts={[c1, c2]} />);
    expect(screen.getByTestId('verdict-matrix-row-c1')).toBeTruthy();
    expect(screen.getByTestId('verdict-matrix-row-c2')).toBeTruthy();
  });
});
