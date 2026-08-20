/**
 * TraceabilityMatrixPanel — mount + data-flow tests (R6.2).
 *
 * Covers:
 *   - Loading state renders while the query is in flight.
 *   - Populated state renders the viewer with the grouped matrix.
 *   - No-workspace state renders the "load a workspace" copy.
 *   - Error state renders a retry button.
 *   - Registry: descriptor appended at id 'traceabilityMatrix',
 *     position 'detail'.
 *
 * Backend traffic is mocked at the `fetch` boundary. The trace matrix uses
 * the REST `GET /models/:uri/trace` facade so it is available in both browser
 * and Tauri-sidecar transports.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor, fireEvent } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { TraceabilityMatrixPanel } from '../TraceabilityMatrixPanel';
import { panelRegistry, findPanel } from '@/shared/panels/registry';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import type { TraceMatrixRow } from '../types';

interface HarnessOpts {
  rows?: TraceMatrixRow[];
  failFetch?: boolean;
  neverResolve?: boolean;
  workspaceUri?: string | null;
}

function withQueryClient(children: ReactNode) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

function mountPanel(opts: HarnessOpts = {}) {
  const fetchMock = vi.fn(async (url: string) => {
    if (opts.neverResolve) {
      return new Promise(() => {}) as unknown as Response;
    }
    if (opts.failFetch) {
      return new Response(JSON.stringify({ error: 'backend down' }), { status: 500 });
    }
    if (url.includes('/trace?')) {
      return new Response(JSON.stringify(opts.rows ?? []), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response(JSON.stringify([]), { status: 200 });
  });
  vi.stubGlobal('fetch', fetchMock);

  if (opts.workspaceUri !== undefined) {
    useWorkspaceUIStore.setState({ workspaceRoot: opts.workspaceUri });
  }

  const utils = render(withQueryClient(<TraceabilityMatrixPanel />));
  return { ...utils, fetchMock };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useWorkspaceUIStore.setState({ workspaceRoot: null });
});

describe('TraceabilityMatrixPanel', () => {
  it('renders the no-workspace empty state when no workspace is loaded', () => {
    mountPanel({ workspaceUri: null });
    expect(screen.getByTestId('trace-matrix-panel-no-workspace')).toBeDefined();
  });

  it('renders loading state while the query is in flight', () => {
    mountPanel({
      workspaceUri: 'file:///ws/test.sysml',
      neverResolve: true,
    });
    expect(screen.getByTestId('trace-matrix-panel-loading')).toBeDefined();
  });

  it('renders the populated matrix after fetch resolves', async () => {
    const { fetchMock } = mountPanel({
      workspaceUri: 'file:///ws/test.sysml',
      // Wire direction: source = satisfier (part), target = requirement.
      rows: [
        {
          source: 'P1',
          source_name: 'Main breaker',
          target: 'R1',
          target_name: 'Brake force',
          relationship: 'rel-1',
        },
      ],
    });
    await waitFor(() => {
      expect(screen.getByTestId('trace-matrix-viewer')).toBeDefined();
    });
    expect(screen.getByTestId('trace-matrix-row-R1')).toBeDefined();
    expect(fetchMock).toHaveBeenCalledWith(
      '/models/__workspace__/trace?source_kind=PartUsage&relation_kind=satisfy&target_kind=RequirementUsage',
      undefined,
    );
  });

  it('renders the error state with a retry when the fetch fails', async () => {
    mountPanel({
      workspaceUri: 'file:///ws/test.sysml',
      failFetch: true,
    });
    await waitFor(() => {
      expect(screen.getByTestId('trace-matrix-panel-error')).toBeDefined();
    });
    const retry = screen.getByTestId('trace-matrix-panel-retry');
    expect(retry).toBeDefined();
    // Smoke: clicking retry doesn't throw — the query will still fail
    // because `failFetch` is set, but the click handler must wire up.
    fireEvent.click(retry);
  });
});

describe('panelRegistry — traceability descriptor', () => {
  it('includes a traceabilityMatrix descriptor at detail position', () => {
    const panel = findPanel('traceabilityMatrix');
    expect(panel).toBeDefined();
    expect(panel!.defaultPosition).toBe('detail');
    expect(panel!.title).toBe('Traceability');
  });

  it('is applicable by default (empty-state handled inside the panel)', () => {
    const panel = findPanel('traceabilityMatrix');
    expect(panel).toBeDefined();
    // applicableWhen is a pure predicate — pass minimal stubs.
    const caps = {
      hasStateMachines: false,
      hasODEs: false,
      hasConstraints: false,
      hasRequirements: false,
      hasVerification: false,
      hasAnalysisCases: false,
      hasFlows: false,
      hasPlots: false,
      hasActionFlows: false,
      smCount: 0,
      smInstanceCount: 0,
      odeCount: 0,
      flowCount: 0,
      constraintCount: 0,
      requirementCount: 0,
      verificationCount: 0,
      analysisCaseCount: 0,
      partCount: 0,
      actionDefCount: 0,
      sessionType: 'sm' as const,
      isMultiFile: false,
    };
    const session = {
      phase: 'idle' as const,
      activeSessionId: null,
      hasStreamingData: false,
    };
    expect(panel!.applicableWhen(caps, session)).toBe(true);
  });

  it('appears in the panelRegistry after archive (R4.1)', () => {
    const ids = panelRegistry.map((p) => p.id);
    const archiveIdx = ids.indexOf('archive');
    const traceIdx = ids.indexOf('traceabilityMatrix');
    expect(archiveIdx).toBeGreaterThanOrEqual(0);
    expect(traceIdx).toBeGreaterThan(archiveIdx);
  });
});
