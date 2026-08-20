/**
 * §6.2 provenance surfacing on the History Executions rows.
 *
 * The wire already carries the full B6 SessionProvenance per execution
 * (executions.rs serializes the whole struct); these tests pin the FE
 * rendering rules:
 *   - collapsed row: quiet `· N files` suffix ONLY when the manifest is
 *     non-empty; pre-§6.2 records render exactly as before (nothing)
 *   - expanded row: the VERIFIED AGAINST record block — solid square
 *     record geometry, workspace root, per-file `path — hash7` lines
 *   - empty/absent manifest: NO block, NO null-state chip (B10 honesty)
 */

import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { ExecutionRowLine, ProvenanceRecord } from '../VerifyHistoryView';
import type { ExecutionRowWire } from '../useExecutionHistory';

afterEach(cleanup);

function executionWire(overrides: Partial<ExecutionRowWire> = {}): ExecutionRowWire {
  return {
    execution_id: 'exec-1',
    origin: 'run',
    label: null,
    timestamp: Date.now() - 60_000,
    evaluation_mode: 'trajectory',
    provenance: {
      model_digest: 'aaaabbbbccccdddd',
      git: { commit: '1234567890ab', dirty: false },
      workspace_root: '/ws/demo',
      file_manifest: [
        { path: 'Main.sysml', content_hash: 'deadbeefcafe0000' },
        { path: 'sub/Verification.sysml', content_hash: 'feedface00001111' },
      ],
    },
    external: null,
    results: [],
    counts: { pass: 1, fail: 0, inconclusive: 0, error: 0 },
    ...overrides,
  };
}

describe('ExecutionRowLine manifest suffix', () => {
  it('shows `· N files` on the collapsed row when the manifest is populated', () => {
    render(<ExecutionRowLine execution={executionWire()} expanded={false} onToggle={() => {}} />);
    const count = screen.getByTestId('verify-execution-manifest-count-exec-1');
    expect(count.textContent).toContain('2 files');
  });

  it('renders nothing for the manifest on pre-§6.2 records (absent field)', () => {
    const wire = executionWire();
    wire.provenance = { model_digest: 'aaaabbbbccccdddd', git: null };
    render(<ExecutionRowLine execution={wire} expanded={false} onToggle={() => {}} />);
    expect(screen.queryByTestId('verify-execution-manifest-count-exec-1')).toBeNull();
    // the digest itself still renders — only the manifest is absent
    expect(screen.getByText('@ aaaabbb')).toBeInTheDocument();
  });

  it('renders nothing for an empty manifest (unit-test-shaped provenance)', () => {
    const wire = executionWire();
    wire.provenance = { model_digest: 'aaaabbbbccccdddd', file_manifest: [] };
    render(<ExecutionRowLine execution={wire} expanded={false} onToggle={() => {}} />);
    expect(screen.queryByTestId('verify-execution-manifest-count-exec-1')).toBeNull();
  });

  it('singularizes: `· 1 file`', () => {
    const wire = executionWire();
    wire.provenance!.file_manifest = [{ path: 'Only.sysml', content_hash: 'abc1234def' }];
    render(<ExecutionRowLine execution={wire} expanded={false} onToggle={() => {}} />);
    expect(
      screen.getByTestId('verify-execution-manifest-count-exec-1').textContent,
    ).toContain('1 file');
  });

  it('expanding the row reveals the VERIFIED AGAINST record', () => {
    let expanded = false;
    const { rerender } = render(
      <ExecutionRowLine
        execution={executionWire()}
        expanded={expanded}
        onToggle={() => {
          expanded = true;
        }}
      />,
    );
    expect(screen.queryByTestId('verify-execution-provenance-exec-1')).toBeNull();
    fireEvent.click(screen.getByRole('button'));
    rerender(
      <ExecutionRowLine execution={executionWire()} expanded={expanded} onToggle={() => {}} />,
    );
    expect(screen.getByTestId('verify-execution-provenance-exec-1')).toBeInTheDocument();
  });
});

describe('ProvenanceRecord', () => {
  it('renders the record: header with count + workspace root, per-file hash7 lines', () => {
    render(
      <ProvenanceRecord
        provenance={executionWire().provenance}
        executionId="exec-1"
      />,
    );
    const record = screen.getByTestId('verify-execution-provenance-exec-1');
    expect(record.textContent).toContain('VERIFIED AGAINST · 2 files');
    expect(record.textContent).toContain('/ws/demo');
    expect(record.textContent).toContain('Main.sysml');
    expect(record.textContent).toContain('deadbee'); // hash7, full hash in title
    expect(record.textContent).toContain('sub/Verification.sysml');
    // solid square record geometry — never dashed (that means ingested),
    // never a pill
    expect(record.style.borderRadius).toBe('4px');
    expect(record.style.border).toContain('solid');
  });

  it('omits the workspace-root segment when absent, without a placeholder', () => {
    const provenance = executionWire().provenance!;
    provenance.workspace_root = null;
    render(<ProvenanceRecord provenance={provenance} executionId="exec-1" />);
    const record = screen.getByTestId('verify-execution-provenance-exec-1');
    expect(record.textContent).toContain('VERIFIED AGAINST · 2 files');
    expect(record.textContent).not.toContain('null');
  });

  it('renders NOTHING for an empty or absent manifest — no null-state chip', () => {
    const { container: c1 } = render(
      <ProvenanceRecord provenance={{ model_digest: 'x', file_manifest: [] }} executionId="e" />,
    );
    expect(c1.innerHTML).toBe('');
    const { container: c2 } = render(<ProvenanceRecord provenance={null} executionId="e" />);
    expect(c2.innerHTML).toBe('');
  });
});
