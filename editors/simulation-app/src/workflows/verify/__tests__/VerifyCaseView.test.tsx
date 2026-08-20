/**
 * Tests for VerifyCaseView — the case-as-document surface (design 1a).
 *
 * Drives the real §4.1 payload (the ClauseFourReview whole-clause case that
 * fails through a nested referenced obligation three levels deep) and
 * asserts all three registers render, the nested subrequirement chain is
 * shown under the objective (never flat), the teaching states hold, `esc`
 * returns, and the two run affordances are labeled (never substituted).
 *
 * The embedded timeline (process register) is stubbed — its own suite
 * covers it, and it would otherwise pull the network into this render.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const navigateSpy = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', async (orig) => ({
  ...(await orig<typeof import('react-router-dom')>()),
  useNavigate: () => navigateSpy,
}));

// Stub the embedded timeline — it fetches over the network; this test is
// about the case document, not the archive.
vi.mock('../VerdictTimelinePanel', () => ({
  VerdictTimelinePanel: (props: { testId?: string }) => (
    <div data-testid={props.testId ?? 'verdict-timeline-panel'}>timeline stub</div>
  ),
}));

import { VerifyCaseView } from '../VerifyCaseView';
import type { VerificationCaseRow } from '../useVerificationCases';

const CLAUSE_FOUR: VerificationCaseRow = {
  case_id: 'c-clause4',
  element_id: 'e-clause4',
  case_name: 'ClauseFourReview',
  subject: 'bench',
  methods: [], // real demo model ships no @VerificationMethod → teaching state
  evaluation_mode: 'static',
  verdict: 'Fail',
  display: 'FAIL (1/1 failed)',
  passed_requirements: 0,
  total_requirements: 1,
  requirements: [
    {
      requirement_id: 'protectionSpec',
      requirement_name: 'protectionSpec',
      requirement_element_id: 'r-protection',
      requirement_text: 'Protection requirements for the GroupHead family.',
      verdict: 'fail',
      message: 'sub-requirements not satisfied: tripTime, sensing, emcCompliance',
      subrequirements: [
        { requirement_id: 'tripTime', verdict: 'inconclusive', message: 'no modeled pass criteria' },
        { requirement_id: 'sensing', verdict: 'fail', message: 'accuracy bound violated (±12 % > ±10 %)' },
        {
          requirement_id: 'emcCompliance',
          verdict: 'fail',
          message: 'fails via referenced obligation',
          subrequirements: [
            {
              requirement_id: 'iecEmc.radiatedLimit',
              verdict: 'fail',
              message: 'referenced obligation not satisfied',
            },
          ],
        },
      ],
    },
  ],
};

function renderCase(props: Partial<React.ComponentProps<typeof VerifyCaseView>> = {}) {
  const defaults = {
    caseRow: CLAUSE_FOUR,
    onBack: vi.fn(),
    onEvaluateStatic: vi.fn(),
    onRunWithSimulation: vi.fn(),
    canRunWithSimulation: false,
  };
  const merged = { ...defaults, ...props };
  // The view now owns react-query reads (latest_status per-mode lines +
  // the approval sidecar) — provide a client; no workspace root is set in
  // these tests, so the queries stay disabled and nothing fetches.
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <VerifyCaseView {...merged} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return merged;
}

afterEach(() => {
  cleanup();
  navigateSpy.mockReset();
});

describe('VerifyCaseView — three registers', () => {
  it('renders the header, model, computed, and process registers', () => {
    renderCase({ modelDigest: 'c7e2a1abcdef' });
    expect(screen.getByTestId('verify-case-view')).toBeInTheDocument();
    expect(screen.getByTestId('verify-case-header')).toHaveTextContent('ClauseFourReview');
    expect(screen.getByTestId('verify-case-subject')).toHaveTextContent('bench');
    expect(screen.getByTestId('verify-case-model-register')).toBeInTheDocument();
    expect(screen.getByTestId('verify-case-computed-register')).toBeInTheDocument();
    expect(screen.getByTestId('verify-case-process-register')).toBeInTheDocument();
    // The process register embeds the shipped timeline machinery.
    expect(screen.getByTestId('verify-case-timeline')).toBeInTheDocument();
  });

  it('nests the check occurrences and the recursive failure chain under the objective', () => {
    renderCase();
    const checks = screen.getByTestId('verify-case-checks');
    // Top check is the whole-clause requirement.
    const top = within(checks).getByTestId('verify-case-check-protectionSpec');
    expect(top).toHaveAttribute('data-check-depth', '0');
    // Its subrequirements nest under it (never a flat peer list, §5.4).
    const subs = within(top).getByTestId('verify-case-subs-protectionSpec');
    expect(within(subs).getByTestId('verify-case-check-tripTime')).toHaveAttribute('data-check-depth', '1');
    expect(within(subs).getByTestId('verify-case-check-sensing')).toBeInTheDocument();
    const emc = within(subs).getByTestId('verify-case-check-emcCompliance');
    // Three levels deep — the referenced obligation.
    const emcSubs = within(emc).getByTestId('verify-case-subs-emcCompliance');
    expect(within(emcSubs).getByTestId('verify-case-check-iecEmc.radiatedLimit')).toHaveAttribute('data-check-depth', '2');
  });

  it('shows the case verdict + static evidence + digest in the computed register', () => {
    renderCase({ modelDigest: 'c7e2a1abcdef' });
    const computed = screen.getByTestId('verify-case-computed-register');
    expect(within(computed).getByTestId('verify-case-verdict')).toHaveAttribute('data-verdict', 'fail');
    // Calm pass: a static desk check has no record, so it renders NO geometry
    // mark (its absence is the "desk check" signal — the "static read" line
    // label carries the mode). The mark appears only for trajectory/external.
    expect(within(computed).queryByTestId('verify-case-mode')).not.toBeInTheDocument();
    const evidence = within(computed).getByTestId('verify-case-evidence-static');
    expect(evidence).toHaveTextContent('computed against the current model');
    expect(evidence).toHaveTextContent('c7e2a1'); // truncated to 7
  });

  it('renders no digest chip when the server omits model_digest', () => {
    renderCase();
    expect(screen.getByTestId('verify-case-evidence-static')).not.toHaveTextContent('@');
  });
});

describe('VerifyCaseView — layer separation + links', () => {
  it('shows the honest "no @VerificationMethod declared" placeholder for empty methods', () => {
    renderCase();
    expect(screen.getByTestId('declared-methods-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('declared-methods')).not.toBeInTheDocument();
  });

  it('renders declared method chips when present', () => {
    renderCase({ caseRow: { ...CLAUSE_FOUR, methods: ['inspect', 'test'] } });
    expect(screen.getByTestId('declared-methods')).toHaveTextContent('inspect · test');
  });

  it('links a check to its Requirements-workbench row by element id', () => {
    renderCase();
    fireEvent.click(screen.getByTestId('verify-case-req-link-protectionSpec'));
    expect(navigateSpy).toHaveBeenCalledWith('/requirements?req=r-protection');
  });
});

describe('VerifyCaseView — run affordances (labeled, never substituted)', () => {
  it('labels both run affordances distinctly', () => {
    renderCase();
    expect(screen.getByTestId('verify-case-evaluate-static')).toHaveTextContent('Evaluate (static)');
    expect(screen.getByTestId('verify-case-run-simulation')).toHaveTextContent('Run with simulation');
  });

  it('gates "Run with simulation" honestly when no live session is available', () => {
    renderCase({ canRunWithSimulation: false });
    const simBtn = screen.getByTestId('verify-case-run-simulation');
    expect(simBtn).toBeDisabled();
    expect(simBtn).toHaveAttribute('data-gated', 'true');
    expect(simBtn.getAttribute('title')).toMatch(/live session/i);
  });

  it('fires the callbacks on click', () => {
    const props = renderCase({ canRunWithSimulation: true });
    fireEvent.click(screen.getByTestId('verify-case-evaluate-static'));
    expect(props.onEvaluateStatic).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('verify-case-run-simulation'));
    expect(props.onRunWithSimulation).toHaveBeenCalled();
  });

  it('returns via the esc/back affordance', () => {
    const props = renderCase();
    fireEvent.click(screen.getByTestId('verify-case-back'));
    expect(props.onBack).toHaveBeenCalled();
  });
});

describe('VerifyCaseView — teaching states', () => {
  it('bare objective mints no verdict — teaching text, no verdict chip', () => {
    renderCase({ caseRow: { ...CLAUSE_FOUR, total_requirements: 0, requirements: [] } });
    expect(screen.getByTestId('verify-case-bare-objective')).toHaveTextContent('a bare objective verifies nothing');
    expect(screen.queryByTestId('verify-case-verdict')).not.toBeInTheDocument();
    expect(screen.getByTestId('verify-case-computed-noverdict')).toBeInTheDocument();
  });

  it('renders the empty state when no case is selected', () => {
    renderCase({ caseRow: null });
    expect(screen.getByTestId('verify-case-view-empty')).toBeInTheDocument();
  });
});
