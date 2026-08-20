import { afterEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { AnalysisCasesLanding } from '../AnalysisCasesLanding';
import { useWorkspaceStore } from '@/store/workspace';

const httpPost = vi.fn();
vi.mock('@/shared/api/http', () => ({ httpPost: (...args: unknown[]) => httpPost(...args) }));

const originalWorkspaceState = useWorkspaceStore.getState();

afterEach(() => {
  cleanup();
  httpPost.mockReset();
  useWorkspaceStore.setState(originalWorkspaceState, true);
});

function renderLanding(initialEntries = ['/analyze']) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={initialEntries}>
        <AnalysisCasesLanding />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

function seedWorkspace() {
  useWorkspaceStore.setState({
    loadedFiles: new Map([
      ['file:///model-a.sysml', { uri: 'file:///model-a.sysml', source: '', dirty: false, tree: [] }],
      ['file:///model-b.sysml', { uri: 'file:///model-b.sysml', source: '', dirty: false, tree: [] }],
    ]),
  });
}

describe('AnalysisCasesLanding', () => {
  it('prompts for a workspace when none is loaded', () => {
    renderLanding();
    expect(screen.getByTestId('analysis-cases-empty')).toHaveTextContent(/Load a workspace/i);
    expect(httpPost).not.toHaveBeenCalled();
  });

  it('lists workspace analysis cases from ONE call and shows selected detail', async () => {
    // Regression pin (scope-collapse follow-up, 2026-07-17): the landing
    // used to issue one identical `sysml.evaluate.analysis_cases` call per
    // loaded uri and tag each duplicated row with that uri — with two files
    // loaded, every case rendered twice under fabricated provenance.
    seedWorkspace();
    httpPost.mockResolvedValueOnce([
      {
        element_id: 'case-a',
        case_name: 'ThermalAnalysis',
        display: '✓ 2 outputs',
        subject: 'Battery',
        objective: 'Keep temperature below limit',
        tool_name: 'thermal-solver',
        tool_uri: 'tool://thermal',
        parameters: [{ name: 'ambient', direction: 'in', value: 22 }],
        constraints: [{ expression: 'temp < 350' }],
        result_expression: 'max(temp)',
        diagnostics: [],
      },
      {
        element_id: 'case-b',
        case_name: 'MassAnalysis',
        display: '✓ 1 outputs',
        subject: 'Vehicle',
        objective: 'Estimate mass',
        tool_name: null,
        tool_uri: null,
        parameters: [],
        constraints: [],
        result_expression: null,
        diagnostics: [],
      },
    ]);

    renderLanding(['/analyze?case_id=case-b']);

    expect(await screen.findByTestId('analysis-cases-landing')).toBeInTheDocument();
    // ONE workspace call despite two loaded files.
    expect(httpPost).toHaveBeenCalledTimes(1);
    expect(httpPost).toHaveBeenCalledWith('/api/command', {
      command: 'sysml.evaluate.analysis_cases',
      params: {},
    });
    // Each case renders exactly once in the sidebar.
    expect(screen.getAllByTestId('analysis-case-case-a')).toHaveLength(1);
    expect(screen.getAllByTestId('analysis-case-case-b')).toHaveLength(1);

    const detail = screen.getByTestId('analysis-case-detail');
    expect(within(detail).getByText('MassAnalysis')).toBeInTheDocument();
    expect(within(detail).getByText('Vehicle')).toBeInTheDocument();
  });

  it('runs the selected analysis case and displays outputs', async () => {
    seedWorkspace();
    httpPost
      .mockResolvedValueOnce([
        {
          element_id: 'case-a',
          case_name: 'ThermalAnalysis',
          display: 'ready',
          subject: 'Battery',
          objective: 'Keep temperature below limit',
          tool_name: 'thermal-solver',
          tool_uri: null,
          parameters: [
            { name: 'ambient', direction: 'in', default_value: '22' },
            { name: 'maxTemp', direction: 'out' },
          ],
          constraints: [],
          result_expression: null,
          diagnostics: [],
        },
      ])
      .mockResolvedValueOnce({
        case_name: 'ThermalAnalysis',
        tool_name: 'thermal-solver',
        input_parameters: [],
        outputs: { maxTemp: '342' },
        converged: true,
        iterations: 3,
      });

    renderLanding(['/analyze?case_id=case-a']);

    const override = await screen.findByTestId('analysis-override-ambient');
    expect(override).toHaveValue('22');
    expect(screen.queryByTestId('analysis-override-maxTemp')).not.toBeInTheDocument();

    fireEvent.change(override, { target: { value: '25' } });
    fireEvent.click(screen.getByTestId('analysis-case-run'));

    await waitFor(() => expect(httpPost).toHaveBeenCalledTimes(2));
    expect(httpPost).toHaveBeenLastCalledWith('/api/command', {
      command: 'sysml.analysis.run',
      params: { case_name: 'ThermalAnalysis', overrides: [['ambient', '25']] },
    });
    const result = await screen.findByTestId('analysis-run-result');
    expect(within(result).getByText('true')).toBeInTheDocument();
    expect(within(result).getByText(/maxTemp/)).toBeInTheDocument();
    expect(within(result).getByText(/342/)).toBeInTheDocument();
  });

  it('shows the guided activity surface when no AnalysisCases are found', async () => {
    // Phase 5 residual: the bare "No AnalysisCases found" empty state grew
    // into the guided landing — authoring snippet + per-method availability
    // cards derived from the backend capability profile.
    seedWorkspace();
    useWorkspaceStore.setState({
      capabilities: {
        hasStateMachines: true,
        hasActionFlows: false,
        hasOdeDynamics: true,
        hasPortFlows: false,
        hasMultipleSubsystems: false,
        hasConstraints: false,
        hasRequirements: false,
        hasTradeStudies: false,
        stateMachineNames: ['TripUnitSM', 'SensorSM'],
        actionFlowNames: [],
        tradeStudyNames: [],
      },
    });
    httpPost.mockResolvedValue([]);

    renderLanding();

    const landing = await screen.findByTestId('analyze-guided-landing');
    expect(landing).toHaveTextContent(/declares no AnalysisCases yet/i);
    // Teaching snippet — source is gospel, no in-app creation wizard.
    expect(landing).toHaveTextContent(/analysis def ThermalMargin/);

    // Availability derives from capabilities, honestly per method.
    expect(screen.getByTestId('analyze-method-card-sweep')).toHaveAttribute('data-available', 'true');
    expect(screen.getByTestId('analyze-method-card-sweep')).toHaveTextContent(/ODE dynamics detected/);
    expect(screen.getByTestId('analyze-method-card-montecarlo')).toHaveTextContent(/2 state machines/);
    expect(screen.getByTestId('analyze-method-card-trade-study')).toHaveAttribute('data-available', 'false');
    expect(screen.getByTestId('analyze-method-card-trade-study')).toHaveTextContent(/No analysis cases declared/);
    expect(screen.getByTestId('analyze-method-card-sensitivity')).toHaveAttribute('data-available', 'false');
    expect(screen.getByTestId('analyze-method-card-sensitivity')).toHaveTextContent(/No constraints/);

    // Cards route to their method surfaces.
    expect(screen.getByTestId('analyze-method-card-sweep')).toHaveAttribute('href', '/analyze/sweep');
  });

  it('groups the guided-landing cards under evaluates-over bands — teaching, never routing (turn 2)', async () => {
    seedWorkspace();
    useWorkspaceStore.setState({
      capabilities: {
        hasStateMachines: true,
        hasActionFlows: false,
        hasOdeDynamics: true,
        hasPortFlows: false,
        hasMultipleSubsystems: false,
        hasConstraints: true,
        hasRequirements: false,
        hasTradeStudies: false,
        stateMachineNames: [],
        actionFlowNames: [],
        tradeStudyNames: [],
      },
    });
    httpPost.mockResolvedValue([]);

    renderLanding();
    await screen.findByTestId('analyze-guided-landing');

    // Two bands; the straddlers (Sweep, Monte Carlo) appear ONCE, in the
    // "target decides" band, never duplicated into the static band.
    const staticBand = screen.getByTestId('analyze-evaluates-band-static');
    const bothBand = screen.getByTestId('analyze-evaluates-band-both');
    expect(within(staticBand).getByTestId('analyze-method-card-trade-study')).toBeInTheDocument();
    expect(within(staticBand).getByTestId('analyze-method-card-sensitivity')).toBeInTheDocument();
    expect(within(bothBand).getByTestId('analyze-method-card-sweep')).toBeInTheDocument();
    expect(within(bothBand).getByTestId('analyze-method-card-montecarlo')).toBeInTheDocument();
    expect(screen.getAllByTestId('analyze-method-card-sweep')).toHaveLength(1);

    // Straddler cards carry BOTH mode badges; static-band cards only bare static.
    const sweep = within(bothBand).getByTestId('analyze-method-card-sweep');
    expect(within(sweep).getByTestId('evaluation-mode-badge-static')).toBeInTheDocument();
    expect(within(sweep).getByTestId('evaluation-mode-badge-trajectory')).toBeInTheDocument();
    const trade = within(staticBand).getByTestId('analyze-method-card-trade-study');
    expect(within(trade).queryByTestId('evaluation-mode-badge-trajectory')).not.toBeInTheDocument();

    // Headers teach — they are static text, not links/buttons (routing frozen).
    expect(staticBand).toHaveTextContent(/OVER CURRENT VALUES/);
    expect(within(staticBand).queryByRole('button')).not.toBeInTheDocument();
    // Routes byte-identical to Phase 5.
    expect(within(bothBand).getByTestId('analyze-method-card-montecarlo')).toHaveAttribute('href', '/analyze/montecarlo');
  });

  it('renders the objective-verdict slot only when the run carries one (turn 2, 2b)', async () => {
    seedWorkspace();
    httpPost
      .mockResolvedValueOnce([
        {
          element_id: 'case-an9',
          case_name: 'EmcHeadroomAnalysis',
          display: 'ready',
          subject: 'emcStack',
          objective: 'Find the rework headroom',
          tool_name: null,
          tool_uri: null,
          parameters: [],
          constraints: [],
          result_expression: null,
          diagnostics: [],
        },
      ])
      .mockResolvedValueOnce({
        case_name: 'EmcHeadroomAnalysis',
        tool_name: 'bisection',
        input_parameters: [],
        outputs: { headroom_db: '0.5' },
        converged: true,
        iterations: 23,
        objective_verdict: {
          verdict: 'Pass',
          evaluation_mode: 'static',
          summary: { pass: 1, fail: 0, inconclusive: 0, error: 0, overall: 'Pass' },
          requirements: [
            {
              requirement_id: 'HeadroomWithinRework',
              verdict: 'Pass',
              message: 'headroom_db = 0.5 <= 1.0',
            },
          ],
        },
      });

    renderLanding(['/analyze?case_id=case-an9']);
    fireEvent.click(await screen.findByTestId('analysis-case-run'));

    const slot = await screen.findByTestId('analysis-objective-verdict');
    // Verdict colours are allowed here — it IS a verdict — beside the bare
    // static badge; the requirement line names the case's own objective bar.
    expect(within(slot).getByTestId('analysis-objective-verdict-badge')).toBeInTheDocument();
    expect(within(slot).getByTestId('analysis-objective-mode')).toHaveAttribute('data-evaluation-mode', 'static');
    expect(slot).toHaveTextContent(/HeadroomWithinRework/);
    expect(slot).toHaveTextContent(/PASS \(1\/1\)/);
  });

  it('renders NO objective-verdict chrome when the case declares no verify’d objective', async () => {
    seedWorkspace();
    httpPost
      .mockResolvedValueOnce([
        {
          element_id: 'case-a',
          case_name: 'ThermalAnalysis',
          display: 'ready',
          subject: 'Battery',
          objective: null,
          tool_name: null,
          tool_uri: null,
          parameters: [],
          constraints: [],
          result_expression: null,
          diagnostics: [],
        },
      ])
      .mockResolvedValueOnce({
        case_name: 'ThermalAnalysis',
        tool_name: null,
        input_parameters: [],
        outputs: { maxTemp: '342' },
        converged: true,
        iterations: 3,
      });

    renderLanding(['/analyze?case_id=case-a']);
    fireEvent.click(await screen.findByTestId('analysis-case-run'));

    await screen.findByTestId('analysis-run-result');
    // Absent means absent — no slot, no header, no null-state chip.
    expect(screen.queryByTestId('analysis-objective-verdict')).not.toBeInTheDocument();
  });
});
