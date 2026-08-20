/**
 * VerdictTimelinePanel — process-log (newest-first list) render + fetch tests.
 *
 * The panel was a horizontal time axis with per-case lanes; the calm pass
 * (turn 4, register item iv) retired the axis for a newest-first LIST of
 * verdict records + attestations, merged by timestamp. These tests cover:
 *   - loading / empty / error states
 *   - fetch command + params (workspace_uri regression pin kept)
 *   - records render as rows with verdict + geometry channel; drill preserved
 *   - external records: dashed provenance, ⚑ when stale, run-ref drill
 *   - static verdicts excluded (no archive)
 *   - attestations merged into the same list, newest-first, never a lane
 *   - pure helpers (mergeProcessEvents, tooltips, classification)
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import {
  VerdictTimelinePanel,
  buildMarkerTooltip,
  buildExternalTooltip,
  isArchivableEntry,
  isExternalEntry,
  mergeProcessEvents,
  type VerdictTimelineEntry,
  type VerdictTimelineResponse,
} from '../VerdictTimelinePanel';

// Attestations are fed by the shared workflow-state read. Mock it so they
// appear ONLY when the panel is given case element ids.
vi.mock('@/features/workflow/queries', () => ({
  useVerificationAttestations: (elementIds: string[]) => ({
    attestations:
      elementIds.length === 0
        ? []
        : [
            {
              element_id: 'VC-4.3',
              seq: 2,
              method: 'inspect',
              statement: 'clause-4 documentation trail reviewed',
              attested_commit: '9f2c31',
              actor: 'R. Millar',
              timestamp_ms: 1_500,
              superseded: true,
            },
            {
              element_id: 'VC-4.1',
              seq: 1,
              method: 'demo',
              statement: 'bench demo witnessed',
              attested_commit: '8d11f0',
              actor: 'K. Osei',
              timestamp_ms: 500,
              superseded: false,
            },
          ],
    isLoading: false,
    isError: false,
  }),
}));

// ── Test harness ─────────────────────────────────────────────────────

function makeClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

function Wrap({ client, children }: { client: QueryClient; children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

type FetchFn = typeof fetch;

function setFetch(fn: FetchFn | undefined) {
  (globalThis as unknown as { fetch: FetchFn | undefined }).fetch = fn;
}

/** Install a `globalThis.fetch` stub returning the given JSON body with status 200. */
function stubFetch(body: unknown) {
  const fn = vi.fn(async () => ({
    ok: true,
    status: 200,
    statusText: 'OK',
    json: async () => body,
  } as Response));
  setFetch(fn as unknown as FetchFn);
  return fn;
}

/** Pull the body JSON from a recorded fetch call. */
function readBody(fn: ReturnType<typeof vi.fn>, callIndex = 0): Record<string, unknown> {
  const call = fn.mock.calls[callIndex];
  if (!call) throw new Error(`no fetch call at index ${callIndex}`);
  const init = call[1] as RequestInit | undefined;
  if (!init || init.body == null) throw new Error('fetch call had no body');
  return JSON.parse(String(init.body)) as Record<string, unknown>;
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// ── Pure-helper tests (no DOM) ───────────────────────────────────────

describe('VerdictTimelinePanel — pure helpers', () => {
  it('mergeProcessEvents merges records + attestations newest-first', () => {
    const entries: VerdictTimelineEntry[] = [
      { session_id: 's1', timestamp: 100, case_id: 'CaseA', verdict: 'pass' },
      { session_id: 's2', timestamp: 400, case_id: 'CaseA', verdict: 'fail' },
    ];
    const attestations = [
      { element_id: 'A', seq: 1, method: 'demo', statement: '', attested_commit: 'x', actor: 'K', timestamp_ms: 300, superseded: false },
    ];
    const events = mergeProcessEvents(entries, attestations as never);
    expect(events.map((e) => e.ts)).toEqual([400, 300, 100]);
    expect(events.map((e) => e.kind)).toEqual(['record', 'attestation', 'record']);
  });

  it('mergeProcessEvents keeps a record before an attestation on a timestamp tie', () => {
    const entries: VerdictTimelineEntry[] = [
      { session_id: 's1', timestamp: 500, case_id: 'CaseA', verdict: 'pass' },
    ];
    const attestations = [
      { element_id: 'A', seq: 1, method: 'demo', statement: '', attested_commit: 'x', actor: 'K', timestamp_ms: 500, superseded: false },
    ];
    const events = mergeProcessEvents(entries, attestations as never);
    expect(events.map((e) => e.kind)).toEqual(['record', 'attestation']);
  });

  it('buildMarkerTooltip includes case, verdict, timestamp, session id', () => {
    const tip = buildMarkerTooltip({
      session_id: 'sess-7',
      timestamp: 0,
      case_id: 'CaseZ',
      verdict: 'FAIL',
    });
    expect(tip).toContain('CaseZ');
    expect(tip).toContain('fail'); // lowercased
    expect(tip).toContain('sess-7');
    expect(tip).toContain('1970-01-01T00:00:00.000Z');
  });

  it('isExternalEntry / isArchivableEntry classify by evaluation_mode', () => {
    expect(isExternalEntry({ session_id: '', timestamp: 0, case_id: 'c', verdict: 'pass', evaluation_mode: 'external' })).toBe(true);
    expect(isExternalEntry({ session_id: '', timestamp: 0, case_id: 'c', verdict: 'pass', evaluation_mode: 'trajectory' })).toBe(false);
    expect(isArchivableEntry({ session_id: '', timestamp: 0, case_id: 'c', verdict: 'pass', evaluation_mode: 'static' })).toBe(false);
    expect(isArchivableEntry({ session_id: '', timestamp: 0, case_id: 'c', verdict: 'pass', evaluation_mode: 'trajectory' })).toBe(true);
  });

  it('buildExternalTooltip names the tool and marks staleness', () => {
    const tip = buildExternalTooltip({
      session_id: '',
      timestamp: 0,
      case_id: 'CaseX',
      verdict: 'pass',
      evaluation_mode: 'external',
      external: { tool: 'hil-bench-2', matches_current_model: false },
    });
    expect(tip).toContain('CaseX');
    expect(tip).toContain('hil-bench-2');
    expect(tip).toContain('older model');
  });
});

// ── Render + fetch integration tests ─────────────────────────────────

describe('VerdictTimelinePanel — render states', () => {
  beforeEach(() => {
    setFetch(undefined);
  });

  it('renders loading state before the fetch resolves', async () => {
    setFetch(vi.fn(() => new Promise(() => {})) as unknown as FetchFn);
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );
    expect(screen.getByTestId('verdict-timeline-panel-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no records and no attestations', async () => {
    const fetchFn = stubFetch({ entries: [] } satisfies VerdictTimelineResponse);
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );

    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-empty')).toBeInTheDocument();
    });
    expect(screen.getByTestId('verdict-timeline-panel-empty')).toHaveTextContent('No recorded acts yet');

    expect(fetchFn).toHaveBeenCalledTimes(1);
    const body = readBody(fetchFn);
    expect(body.command).toBe('sysml.verify.timeline');
    const params = body.params as Record<string, unknown>;
    // Regression pin (scope-collapse W7 follow-up): the request carries NO
    // workspace_uri — the archive stores run-scope uris, so an old
    // `loadedUris[0]` filter matched almost nothing.
    expect('workspace_uri' in params).toBe(false);
    expect(params.case_ids).toBeNull();
    expect(params.since_timestamp).toBeNull();
  });

  it('forwards case_ids and since_timestamp filters to the backend', async () => {
    const fetchFn = stubFetch({ entries: [] });
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel caseIds={['CaseA', 'CaseB']} sinceTimestamp={1_713_000_000_000} />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-empty')).toBeInTheDocument();
    });
    const body = readBody(fetchFn);
    const params = body.params as Record<string, unknown>;
    expect(params.case_ids).toEqual(['CaseA', 'CaseB']);
    expect(params.since_timestamp).toBe(1_713_000_000_000);
  });

  it('renders records as newest-first rows carrying verdict + external attrs', async () => {
    const entries: VerdictTimelineEntry[] = [
      { session_id: 's1', timestamp: 1_000, case_id: 'CaseA', verdict: 'pass', evaluation_mode: 'trajectory' },
      { session_id: 's2', timestamp: 3_000, case_id: 'CaseA', verdict: 'fail', evaluation_mode: 'trajectory' },
      { session_id: 's3', timestamp: 2_000, case_id: 'CaseB', verdict: 'error', evaluation_mode: 'trajectory' },
    ];
    stubFetch({ entries } satisfies VerdictTimelineResponse);
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );

    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel')).toBeInTheDocument();
    });

    // Newest first: s2 (3000), s3 (2000), s1 (1000).
    const r0 = screen.getByTestId('verdict-timeline-panel-record-0');
    const r1 = screen.getByTestId('verdict-timeline-panel-record-1');
    const r2 = screen.getByTestId('verdict-timeline-panel-record-2');
    expect(r0).toHaveAttribute('data-verdict', 'fail');
    expect(r1).toHaveAttribute('data-verdict', 'error');
    expect(r2).toHaveAttribute('data-verdict', 'pass');

    // Trajectory records are keyboard-activatable when a select handler exists;
    // here none is supplied, so they are inert (no tabindex). Verified below.
    expect(r0).not.toHaveAttribute('data-external');
  });

  it('fires onVerdictSelect when a trajectory record is clicked or keyboard-activated', async () => {
    const entries: VerdictTimelineEntry[] = [
      {
        session_id: 's1',
        timestamp: 10,
        case_id: 'CaseA',
        verdict: 'fail',
        evaluation_mode: 'trajectory',
        evidence: { session_id: 's1', tick: 42, element_id: 'Req::X' },
      },
    ];
    stubFetch({ entries });
    const client = makeClient();
    const onSelect = vi.fn();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel onVerdictSelect={onSelect} />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-record-0')).toBeInTheDocument();
    });
    const row = screen.getByTestId('verdict-timeline-panel-record-0');
    expect(row).toHaveAttribute('tabindex', '0');

    act(() => {
      fireEvent.click(row);
    });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0][0]).toMatchObject({
      session_id: 's1',
      case_id: 'CaseA',
      verdict: 'fail',
      evidence: { session_id: 's1', tick: 42, element_id: 'Req::X' },
    });

    act(() => {
      fireEvent.keyDown(row, { key: 'Enter' });
    });
    expect(onSelect).toHaveBeenCalledTimes(2);
    act(() => {
      fireEvent.keyDown(row, { key: ' ' });
    });
    expect(onSelect).toHaveBeenCalledTimes(3);
  });

  it('renders the error state when the fetch fails', async () => {
    setFetch(
      vi.fn(async () => ({
        ok: false,
        status: 500,
        statusText: 'boom',
        json: async () => ({ error: 'backend blew up' }),
      } as Response)) as unknown as FetchFn,
    );
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-error')).toBeInTheDocument();
    });
  });
});

// ── external / staleness / static exclusion / attestations ────────────

describe('VerdictTimelinePanel — external / staleness / attestations', () => {
  beforeEach(() => setFetch(undefined));

  it('renders an external record as dashed provenance with the tool + ⚑ when stale', async () => {
    const entries: VerdictTimelineEntry[] = [
      {
        session_id: '',
        timestamp: 2_000,
        case_id: 'TripAt5xComplianceCase',
        verdict: 'pass',
        evaluation_mode: 'external',
        external: { tool: 'hil-bench-2', run_ref: 'https://ci/run/8841', matches_current_model: false },
      },
    ];
    stubFetch({ entries } satisfies VerdictTimelineResponse);
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );

    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-record-0')).toBeInTheDocument();
    });
    const row = screen.getByTestId('verdict-timeline-panel-record-0');
    expect(row).toHaveAttribute('data-external', 'true');
    expect(row).toHaveAttribute('data-tool', 'hil-bench-2');
    expect(row).toHaveAttribute('data-stale', 'true');
    expect(row.textContent).toContain('hil-bench-2');
    expect(screen.getByTestId('verdict-timeline-panel-stale-0').textContent).toContain('older model');
  });

  it('opens the external run ref on activation (never a session drill)', async () => {
    const entries: VerdictTimelineEntry[] = [
      {
        session_id: '',
        timestamp: 2_000,
        case_id: 'ExtCase',
        verdict: 'pass',
        evaluation_mode: 'external',
        external: { tool: 'pytest-ci', run_ref: 'https://ci/run/9', matches_current_model: true },
      },
    ];
    stubFetch({ entries });
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null);
    const onSelect = vi.fn();
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel onVerdictSelect={onSelect} />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-record-0')).toBeInTheDocument();
    });
    act(() => {
      fireEvent.click(screen.getByTestId('verdict-timeline-panel-record-0'));
    });
    expect(openSpy).toHaveBeenCalledWith('https://ci/run/9', '_blank', 'noopener,noreferrer');
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('never renders static verdicts — they have no archive', async () => {
    const entries: VerdictTimelineEntry[] = [
      { session_id: 's-static', timestamp: 1_000, case_id: 'DeskCheck', verdict: 'pass', evaluation_mode: 'static' },
      { session_id: 's-traj', timestamp: 2_000, case_id: 'RunCase', verdict: 'pass', evaluation_mode: 'trajectory' },
    ];
    stubFetch({ entries });
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel')).toBeInTheDocument();
    });
    // Only the trajectory record renders (one row); the static one is filtered.
    expect(screen.getByTestId('verdict-timeline-panel-record-0')).toBeInTheDocument();
    expect(screen.queryByTestId('verdict-timeline-panel-record-1')).toBeNull();
    expect(screen.getByTestId('verdict-timeline-panel-footnote').textContent).toContain(
      'static verdicts never appear',
    );
  });

  it('merges attestations into the list newest-first, never a lane, only when given element ids', async () => {
    stubFetch({ entries: [] });
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel caseElementIds={['VC-4.1', 'VC-4.3']} />
      </Wrap>,
    );

    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-attestation-0')).toBeInTheDocument();
    });
    // Newest-first: R. Millar's superseded inspect (ts 1500) leads K. Osei (500).
    const first = screen.getByTestId('verdict-timeline-panel-attestation-0');
    expect(first).toHaveAttribute('data-actor', 'R. Millar');
    expect(first).toHaveAttribute('data-superseded', 'true');
    expect(first.textContent).toContain('superseded');
    const second = screen.getByTestId('verdict-timeline-panel-attestation-1');
    expect(second).toHaveAttribute('data-actor', 'K. Osei');
  });

  it('renders no attestations when no case element ids are supplied', async () => {
    stubFetch({
      entries: [
        { session_id: 's1', timestamp: 1_000, case_id: 'CaseA', verdict: 'pass', evaluation_mode: 'trajectory' },
      ],
    });
    const client = makeClient();
    render(
      <Wrap client={client}>
        <VerdictTimelinePanel />
      </Wrap>,
    );
    await waitFor(() => {
      expect(screen.getByTestId('verdict-timeline-panel-record-0')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('verdict-timeline-panel-attestation-0')).toBeNull();
  });
});
