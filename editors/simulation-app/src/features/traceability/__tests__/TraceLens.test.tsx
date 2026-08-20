/**
 * Trace lenses — the matrix says which question it asked, and offers a better
 * one when its answer is empty.
 *
 * Measured on `espresso-production-cell` (the J1 fixture) against a live
 * backend on 2026-08-19:
 *
 *   PartUsage                  · satisfy · RequirementUsage       -> 0
 *   PartUsage                  · satisfy · RequirementDefinition  -> 0
 *   VerificationCaseDefinition · verify  · RequirementDefinition  -> 8
 *   VerificationCaseUsage      · verify  · RequirementUsage       -> 0
 *
 * So the Browse trace view rendered an empty grid for a workspace with eight
 * modelled verify links, and nothing on screen named the question that had
 * produced the emptiness. The fixture numbers above are what these tests
 * stand in for.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

/** Edge counts per (source·relation·target), mirroring the fixture. */
const EDGES = vi.hoisted(() => ({
  current: {} as Record<string, number>,
}));

vi.mock('@/shared/api/http', () => ({
  httpGet: (path: string) => {
    const q = new URLSearchParams(path.split('?')[1] ?? '');
    const key = `${q.get('source_kind')}|${q.get('relation_kind')}|${q.get('target_kind')}`;
    const n = EDGES.current[key] ?? 0;
    return Promise.resolve(
      Array.from({ length: n }, (_, i) => ({
        source: `s-${i}`,
        source_name: `Case${i}`,
        target: `t-${i}`,
        target_name: `Requirement${i}`,
        relationship: q.get('relation_kind'),
      })),
    );
  },
  httpPost: () => Promise.resolve({}),
}));

vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (sel: (s: { workspaceRoot: string | null }) => unknown) =>
    sel({ workspaceRoot: '/ws/espresso-production-cell' }),
}));

// The viewer pulls in selection + source-preview plumbing; this suite is about
// the panel's lens bar and empty state.
vi.mock('../TraceabilityMatrixViewer', () => ({
  TraceabilityMatrixViewer: () => <div data-testid="trace-matrix-viewer">viewer</div>,
}));

import { TraceabilityMatrixPanel } from '../TraceabilityMatrixPanel';

const SATISFY_USAGE = 'PartUsage|satisfy|RequirementUsage';
const VERIFY_DEF = 'VerificationCaseDefinition|verify|RequirementDefinition';

function renderPanel(props = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <TraceabilityMatrixPanel {...props} />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  EDGES.current = {};
});

describe('trace matrix — the active lens is visible', () => {
  it('names the lens and its triple even when the matrix has rows', async () => {
    EDGES.current = { [SATISFY_USAGE]: 3 };
    renderPanel();

    await waitFor(() => expect(screen.getByTestId('trace-matrix-viewer')).toBeInTheDocument());
    expect(screen.getByTestId('trace-matrix-lens-select')).toHaveValue('satisfy-usage');
    expect(screen.getByTestId('trace-matrix-lens-triple')).toHaveTextContent(
      'PartUsage · satisfy · RequirementUsage',
    );
  });

  // The default is deliberately unchanged: "which parts satisfy which
  // requirements" stays the opening question. Fixing this fixture by moving
  // the global default would trade the bug for its mirror image.
  it('still opens on the satisfy lens', async () => {
    EDGES.current = { [VERIFY_DEF]: 8 };
    renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId('trace-matrix-lens-select')).toHaveValue('satisfy-usage'),
    );
  });
});

describe('trace matrix — an empty lens is actionable', () => {
  it('names what was searched for, not just "no data"', async () => {
    EDGES.current = {};
    renderPanel();

    const empty = await screen.findByTestId('trace-matrix-empty');
    expect(empty).toHaveTextContent('satisfy');
    expect(empty).toHaveTextContent('PartUsage');
    expect(empty).toHaveTextContent('RequirementUsage');
  });

  it("offers the fixture's verify lens when satisfy is empty", async () => {
    EDGES.current = { [VERIFY_DEF]: 8 };
    renderPanel();

    const suggestion = await screen.findByTestId('trace-matrix-suggest-verify-def');
    expect(suggestion).toHaveTextContent('Cases verify requirement defs');
    // The count is the evidence that makes the suggestion worth taking.
    expect(suggestion).toHaveTextContent('8 links');
  });

  it('switches to the suggested lens on click and renders the matrix', async () => {
    EDGES.current = { [VERIFY_DEF]: 8 };
    renderPanel();

    fireEvent.click(await screen.findByTestId('trace-matrix-suggest-verify-def'));

    await waitFor(() => expect(screen.getByTestId('trace-matrix-viewer')).toBeInTheDocument());
    expect(screen.getByTestId('trace-matrix-lens-select')).toHaveValue('verify-def');
    expect(screen.getByTestId('trace-matrix-lens-triple')).toHaveTextContent(
      'VerificationCaseDefinition · verify · RequirementDefinition',
    );
  });

  it('says so plainly when no lens finds anything', async () => {
    EDGES.current = {};
    renderPanel();

    expect(await screen.findByTestId('trace-matrix-empty-everywhere')).toHaveTextContent(
      /no modelled traceability/i,
    );
    expect(screen.queryByTestId('trace-matrix-suggestions')).not.toBeInTheDocument();
  });

  it('does not claim emptiness everywhere before the probes land', async () => {
    EDGES.current = { [VERIFY_DEF]: 8 };
    renderPanel();
    // Whatever the intermediate state, the "nothing anywhere" claim must never
    // appear for a workspace that does have links.
    await screen.findByTestId('trace-matrix-suggest-verify-def');
    expect(screen.queryByTestId('trace-matrix-empty-everywhere')).not.toBeInTheDocument();
  });
});

describe('trace matrix — a pinned caller keeps its question', () => {
  // Requirements embeds this panel with its own triple. That caller is asking
  // a specific question, so the picker must not appear and the suggestion must
  // not silently move it somewhere else.
  const PINNED = {
    selectors: {
      source_kind: 'VerificationCaseDefinition',
      target_kind: 'RequirementDefinition',
      relation_kind: 'verify',
    },
  };

  it('shows the lens as fixed text, with no picker', async () => {
    EDGES.current = { [VERIFY_DEF]: 8 };
    renderPanel(PINNED);

    await waitFor(() => expect(screen.getByTestId('trace-matrix-viewer')).toBeInTheDocument());
    expect(screen.getByTestId('trace-matrix-lens-pinned')).toHaveTextContent(
      'Cases verify requirement defs',
    );
    expect(screen.queryByTestId('trace-matrix-lens-select')).not.toBeInTheDocument();
  });

  it('still explains an empty result, but cannot be switched away', async () => {
    EDGES.current = { [SATISFY_USAGE]: 3 };
    renderPanel(PINNED);

    await screen.findByTestId('trace-matrix-empty');
    const suggestion = await screen.findByTestId('trace-matrix-suggest-satisfy-usage');
    expect(suggestion).toBeDisabled();
  });
});
