/**
 * VerifyCaseView — evidence hierarchy (J5).
 *
 * The journey: a reader opens a matrix row labelled "trajectory" and drills
 * into the case to inspect the run behind the verdict. What they got was the
 * STATIC desk check as the first and most prominent verdict, with the run
 * demoted to a sub-line carrying nothing but a truncated id and a relative
 * age — no session, no tick. So the number a reader would quote as "the
 * trajectory verdict" was a recomputation of the authored model, and the run
 * it was supposedly about could not be located.
 *
 * These gates pin the corrected hierarchy:
 *   - a recorded run leads, and carries session + tick + timestamp;
 *   - the desk check stays visible but is relabelled and demoted, so it can
 *     never be read as the run's verdict;
 *   - arriving from a trajectory row with NO stored run says so, rather than
 *     letting the desk check stand in for the run the reader came to see.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

vi.mock('../VerdictTimelinePanel', () => ({
  VerdictTimelinePanel: (props: { testId?: string }) => (
    <div data-testid={props.testId ?? 'verdict-timeline-panel'}>timeline stub</div>
  ),
}));

const latestData = vi.hoisted(() => ({ current: [] as unknown[] }));
vi.mock('../useExecutionHistory', async (orig) => ({
  ...(await orig<typeof import('../useExecutionHistory')>()),
  useLatestStatus: () => ({ data: latestData.current }),
}));

import { VerifyCaseView } from '../VerifyCaseView';
import type { VerificationCaseRow } from '../useVerificationCases';

/** A case whose STATIC verdict deliberately differs from its RUN verdict, so
 *  a mixed-up hierarchy is visible rather than coincidentally agreeing. */
const CASE: VerificationCaseRow = {
  case_id: 'SevereProtectionCase',
  element_id: 'e-severe',
  case_name: 'SevereProtectionCase',
  subject: 'pump',
  methods: [],
  evaluation_mode: 'static',
  verdict: 'Fail',
  display: 'FAIL (1/1 failed)',
  passed_requirements: 0,
  total_requirements: 1,
  requirements: [
    {
      requirement_id: 'ProtectsUnderSevere',
      requirement_name: 'ProtectsUnderSevere',
      requirement_element_id: 'r-protects',
      requirement_text: 'Exposure shall reach the relief trip level.',
      verdict: 'fail',
      message: 'exposure below trip at horizon',
    },
  ],
};

/**
 * @param evidence an object = the run is known; `null` = the server says this
 *   record has none; `'omitted'` = the server never sent the key at all, i.e.
 *   it predates the field. The three are deliberately distinguishable — see
 *   `LatestTrajectoryWire.evidence`.
 */
function latestWithRun(
  evidence:
    | { session_id: string; tick: number; time_ms?: number }
    | null
    | 'omitted',
) {
  return [
    {
      case_id: 'SevereProtectionCase',
      case_element_id: 'e-severe',
      latest: {
        trajectory: {
          verdict: 'pass',
          execution_id: 'fa911215-aaaa-bbbb-cccc-ddddeeeeffff',
          timestamp: Date.now() - 60_000,
          model_digest: 'abc1234def',
          ...(evidence === 'omitted' ? {} : { evidence }),
        },
        external: null,
      },
    },
  ];
}

function renderCase(props: Partial<React.ComponentProps<typeof VerifyCaseView>> = {}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <VerifyCaseView
          caseRow={CASE}
          onBack={vi.fn()}
          onEvaluateStatic={vi.fn()}
          onRunWithSimulation={vi.fn()}
          canRunWithSimulation={false}
          {...props}
        />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  latestData.current = [];
});

describe('VerifyCaseView — the run leads when one exists', () => {
  it('puts the run above the static desk check in the document', () => {
    latestData.current = latestWithRun({ session_id: 'fa911215-1111-2222-3333-444455556666', tick: 3819 });
    renderCase({ entryMode: 'trajectory' });

    const register = screen.getByTestId('verify-case-computed-register');
    const run = within(register).getByTestId('verify-case-latest-run');
    const desk = within(register).getByTestId('verify-case-static-line');

    // DOM order is the hierarchy: the run must precede the desk check.
    expect(run.compareDocumentPosition(desk) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('shows the session, tick, run id and age with the run verdict', () => {
    latestData.current = latestWithRun({ session_id: 'fa911215-1111-2222-3333-444455556666', tick: 3819 });
    renderCase({ entryMode: 'trajectory' });

    const record = screen.getByTestId('verify-case-run-record');
    // Session and tick are the two things that make the verdict checkable.
    expect(record).toHaveTextContent('session fa911215');
    expect(screen.getByTestId('verify-case-run-tick')).toHaveTextContent('tick 3819');
    expect(record).toHaveTextContent('run fa911215');
    expect(record).toHaveTextContent(/ago|just now/);
    expect(record).toHaveTextContent('@ abc1234');

    expect(screen.getByTestId('verify-case-latest-run-verdict')).toHaveTextContent(/pass/i);
  });

  it('relabels the desk check so it cannot read as the run verdict', () => {
    latestData.current = latestWithRun({ session_id: 's-1', tick: 12 });
    renderCase({ entryMode: 'trajectory' });

    const desk = screen.getByTestId('verify-case-static-line');
    expect(desk).toHaveTextContent('static desk check');
    // It is still present and still shows its own (different) verdict — the
    // point is separation, not suppression.
    expect(within(desk).getByTestId('verify-case-verdict')).toHaveTextContent(/fail/i);
  });

  it('marks which line the reader opened the case from', () => {
    latestData.current = latestWithRun({ session_id: 's-1', tick: 12 });
    renderCase({ entryMode: 'trajectory' });
    expect(screen.getByTestId('verify-case-entry-marker')).toBeInTheDocument();
  });

  it('does not claim an entry line when the reader browsed in', () => {
    latestData.current = latestWithRun({ session_id: 's-1', tick: 12 });
    renderCase({ entryMode: null });
    expect(screen.queryByTestId('verify-case-entry-marker')).not.toBeInTheDocument();
    // The run still leads — ordering follows the evidence, not the route in.
    const register = screen.getByTestId('verify-case-computed-register');
    const run = within(register).getByTestId('verify-case-latest-run');
    const desk = within(register).getByTestId('verify-case-static-line');
    expect(run.compareDocumentPosition(desk) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe('VerifyCaseView — honest absence', () => {
  it('says no run is stored when the reader arrived from a trajectory row', () => {
    latestData.current = [];
    renderCase({ entryMode: 'trajectory' });

    expect(screen.getByTestId('verify-case-no-stored-run')).toHaveTextContent(
      /no stored run/i,
    );
    expect(screen.queryByTestId('verify-case-latest-run')).not.toBeInTheDocument();
    // And the desk check reverts to leading, under its own name.
    expect(screen.getByTestId('verify-case-static-line')).toHaveTextContent('static read');
  });

  it('stays quiet about runs when the reader did not come from a run row', () => {
    latestData.current = [];
    renderCase({ entryMode: null });
    expect(screen.queryByTestId('verify-case-no-stored-run')).not.toBeInTheDocument();
    expect(screen.queryByTestId('verify-case-latest-run')).not.toBeInTheDocument();
  });

  it('says the record predates evidence capture when the server says so', () => {
    // An explicit null: the server looked and this pre-B10 execution has no
    // session/tick. Fabricating either would be worse than admitting it.
    latestData.current = latestWithRun(null);
    renderCase({ entryMode: 'trajectory' });

    expect(screen.getByTestId('verify-case-run-record-absent')).toHaveTextContent(
      /predates evidence capture/i,
    );
    expect(screen.queryByTestId('verify-case-run-tick')).not.toBeInTheDocument();
    // The run line itself still leads — the verdict is real, only its
    // locating detail is missing.
    expect(screen.getByTestId('verify-case-latest-run')).toBeInTheDocument();
  });

  // The regression this pair exists for. A brand-new run at tick 5001 was
  // labelled "predates evidence capture" because the server answering had been
  // built before the evidence field existed — so the key was missing, and the
  // UI read missing as "this record is legacy data". It is not: nothing is
  // known about the record either way, and the copy must not assert otherwise.
  it('does not call a run legacy data when the server simply never sent the field', () => {
    latestData.current = latestWithRun('omitted');
    renderCase({ entryMode: 'trajectory' });

    const unreported = screen.getByTestId('verify-case-run-record-unreported');
    expect(unreported).toHaveTextContent(/not reported by this server/i);
    // Crucially NOT the legacy-record wording.
    expect(screen.queryByTestId('verify-case-run-record-absent')).not.toBeInTheDocument();
    expect(unreported.textContent).not.toMatch(/predates/i);
    expect(screen.getByTestId('verify-case-latest-run')).toBeInTheDocument();
  });

  it('shows session, tick and simulated time for a freshly minted run', () => {
    // The J5 repro shape: whole-workspace session advanced to tick 5001, then
    // verified. Both fallbacks must be absent.
    latestData.current = latestWithRun({
      session_id: '79e31c34-49a4-48de-a9c8-a61c88f19efa',
      tick: 5001,
      time_ms: 10002,
    });
    renderCase({ entryMode: 'trajectory' });

    expect(screen.getByTestId('verify-case-run-record')).toHaveTextContent('session 79e31c34');
    expect(screen.getByTestId('verify-case-run-tick')).toHaveTextContent('tick 5001');
    // Model clock, distinct from the tick count (dt = 2 ms here).
    expect(screen.getByTestId('verify-case-run-time')).toHaveTextContent('t = 10.00 s');
    expect(screen.queryByTestId('verify-case-run-record-absent')).not.toBeInTheDocument();
    expect(screen.queryByTestId('verify-case-run-record-unreported')).not.toBeInTheDocument();
  });

  // A tick is not a time. Older records carry no clock, and computing one from
  // dt would be a guess dressed as provenance — so the tick stands alone.
  it('shows the tick alone when the record carries no simulated time', () => {
    latestData.current = latestWithRun({ session_id: 's-1', tick: 5001 });
    renderCase({ entryMode: 'trajectory' });

    expect(screen.getByTestId('verify-case-run-tick')).toHaveTextContent('tick 5001');
    expect(screen.queryByTestId('verify-case-run-time')).not.toBeInTheDocument();
    // And no fallback: the run IS located, just not on the model clock.
    expect(screen.queryByTestId('verify-case-run-record-absent')).not.toBeInTheDocument();
  });

  it('renders sub-second simulated time in milliseconds', () => {
    latestData.current = latestWithRun({ session_id: 's-1', tick: 3, time_ms: 6 });
    renderCase({ entryMode: 'trajectory' });
    expect(screen.getByTestId('verify-case-run-time')).toHaveTextContent('t = 6 ms');
  });
});
