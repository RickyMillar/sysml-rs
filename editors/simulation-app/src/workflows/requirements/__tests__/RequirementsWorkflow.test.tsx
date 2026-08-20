/**
 * RequirementsWorkflow — mount + data-flow tests (Phase 7.5 v1).
 *
 * Backend traffic is mocked at the `fetch` boundary, routed by
 * `body.command` (the app-wide pattern). The shell's rail/strip portal
 * targets are created by hand so `<LeftRailContent>`/
 * `<BottomStripContent>` have somewhere to land outside AppShell.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { RequirementsWorkflow } from '../RequirementsWorkflow';
import { useRequirementsSelectionStore } from '../requirementsSelectionStore';
import { REQUIREMENTS_LINKS_CONTEXT_ID } from '../requirementsLinksRailContext';
import { getRailContext } from '@/app/rail/railRegistry';
import { useRightRailStore } from '@/app/rail/railStore';
import { BOTTOM_STRIP_SLOT_ID, LEFT_RAIL_SLOT_ID } from '@/app/slots';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useBaselineStore } from '@/features/baselines/store';
import { useWorkflowActorStore } from '@/features/workflow/actorStore';
import type { BaselineMeta } from '@/features/baselines/types';
import type { ElementWorkflowStateWire } from '@/features/workflow/types';
import type { SuspectRecordWire } from '@/features/baselines/suspect';
import type {
  RequirementDetail,
  RequirementRow,
  RequirementRowsResult,
} from '@/features/requirements/types';
import type { FieldEditComputed } from '@/features/requirements/fieldEdit';
import { useRequirementEditStore } from '@/features/requirements/editStore';
import { useWorkspaceStore } from '@/store/workspace';

// ── Fixture ──────────────────────────────────────────────────────────

const pkg = (id: string, name: string) => ({ id, name, kind: 'Package' });

function fixtureRow(overrides: Partial<RequirementRow>): RequirementRow {
  return {
    id: 'e-0',
    kind: 'RequirementUsage',
    req_id: null,
    name: null,
    text: null,
    qualified_name: null,
    owning_package: null,
    source_span: null,
    outline_depth: 0,
    maturity: null,
    satisfied_by: [],
    verified_by: [],
    verification: { state: 'incomplete', cases_total: 0, cases_passed: 0 },
    verification_methods: [],
    derived_from: [],
    derives: [],
    refines: [],
    ...overrides,
  };
}

const FIXTURE_ROWS: RequirementRow[] = [
  fixtureRow({
    id: 'e-1',
    req_id: 'REQ-TRIP-01',
    name: 'TripTime',
    text: 'The breaker shall trip within 40 ms.',
    source_span: { file: '/ws/demo/TripUnit.sysml', start: 120, end: 260, line: 7 },
    owning_package: pkg('p-trip', 'TripUnit'),
    maturity: 'done',
    satisfied_by: [{ id: 'e-p1', name: 'sensing_coil', kind: 'PartUsage' }],
    verified_by: [{ id: 'e-v1', name: 'VC-TRIP-01', kind: 'VerificationCaseUsage' }],
    verification: { state: 'pass', cases_total: 2, cases_passed: 2 },
    verification_methods: ['test', 'analyze'],
    derives: [{ id: 'e-2', name: 'SensThreshold', kind: 'RequirementUsage' }],
  }),
  fixtureRow({
    id: 'e-2',
    req_id: 'REQ-SENS-01',
    name: 'SensThreshold',
    text: 'The flow sensor shall detect a branch-flow imbalance ≥ 15 mL/s.',
    owning_package: pkg('p-sens', 'Sensing'),
    maturity: 'tbc',
    verification: { state: 'fail', cases_total: 4, cases_passed: 1 },
    derived_from: [{ id: 'e-1', name: 'TripTime', kind: 'RequirementUsage' }],
  }),
  fixtureRow({
    id: 'e-3',
    req_id: 'REQ-SENS-02',
    name: 'SensAccuracy',
    text: 'Sensing accuracy shall be within ±10 %.',
    owning_package: pkg('p-sens', 'Sensing'),
    maturity: 'tbd',
    verification: { state: 'incomplete', cases_total: 2, cases_passed: 1 },
  }),
  fixtureRow({
    id: 'e-4',
    req_id: 'REQ-USER-01',
    name: 'Indicator',
    text: 'The trip indicator shall be visible from 1 m.',
    owning_package: pkg('p-user', 'UserProtection'),
    verification: { state: 'incomplete', cases_total: 0, cases_passed: 0 },
  }),
];

function rowsResult(rows: RequirementRow[]): RequirementRowsResult {
  return {
    rows,
    total_estimate: rows.length,
    cursor: null,
    cursor_invalidated: false,
    revision: 7,
  };
}

// ── Harness ──────────────────────────────────────────────────────────

interface HarnessOpts {
  rows?: RequirementRow[];
  failFetch?: boolean;
  neverResolve?: boolean;
  workspaceRoot?: string | null;
  /** Baselines the mock store reports (default: none). */
  baselines?: BaselineMeta[];
  /** Suspect records vs the selected baseline (default: none). */
  suspects?: SuspectRecordWire[];
  /** Requirement contract detail (default: empty contract). */
  detail?: RequirementDetail;
  /** Folded workflow state the mock reports (default: pristine). */
  workflowState?: ElementWorkflowStateWire;
  /** Computed edit returned for any `edit_*`/`add_*`/`create_requirement`
   *  command (v2 §7.5 writeback tests). */
  fieldEdit?: FieldEditComputed;
  /** Make the field-edit command itself fail (service error message). */
  fieldEditError?: string;
  /** Link-picker candidates `sysml.query` returns (R5, design §7.6). */
  linkCandidates?: Array<{
    id: string;
    name: string | null;
    qualified_name: string | null;
    kind: string;
  }>;
}

function emptyDetail(id: string): RequirementDetail {
  return {
    id,
    subject: null,
    assumed_constraints: [],
    required_constraints: [],
    inherited_assumed_constraints: [],
    inherited_required_constraints: [],
    instantiated_by: [],
    framed_concerns: [],
    actors: [],
    stakeholders: [],
    referenced_attributes: [],
    rationale: null,
    verification_methods: [],
  };
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

function mountWorkflow(opts: HarnessOpts = {}) {
  // Stand-in portal targets for the shell's rail + strip slots.
  for (const id of [LEFT_RAIL_SLOT_ID, BOTTOM_STRIP_SLOT_ID]) {
    if (!document.getElementById(id)) {
      const target = document.createElement('div');
      target.id = id;
      document.body.appendChild(target);
    }
  }

  const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
    if (opts.neverResolve) {
      return new Promise(() => {}) as unknown as Response;
    }
    if (opts.failFetch) {
      return new Response(JSON.stringify({ error: 'backend down' }), { status: 500 });
    }
    const body = init?.body ? JSON.parse(String(init.body)) : {};
    if (body.command === 'sysml.workspace.requirement_rows') {
      return new Response(JSON.stringify(rowsResult(opts.rows ?? FIXTURE_ROWS)), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.trace_matrix') {
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.store.save_workspace') {
      return new Response(
        JSON.stringify({ commit: 'digest-abc', parent: null, message: 'workspace snapshot', timestamp: 1_700_000_000 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (body.command === 'sysml.store.baseline.list') {
      return new Response(JSON.stringify(opts.baselines ?? []), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.store.baseline.create') {
      return new Response(
        JSON.stringify({ name: body.params?.name ?? '?', commit: 'digest-abc', created_at: 1_700_000_000 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (body.command === 'sysml.workspace.requirement_suspects') {
      return new Response(JSON.stringify(opts.suspects ?? []), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.workflow.attest_suspect_clearing') {
      return new Response(
        JSON.stringify({
          seq: 1,
          schema_version: 1,
          project: body.params?.project,
          element_id: body.params?.element_id,
          actor: body.params?.actor,
          timestamp_ms: 1_700_000_000_000,
          kind: 'suspect_clearing_attestation',
          baseline_name: body.params?.baseline,
          baseline_commit: 'digest-base',
          attested_commit: 'digest-abc',
          rationale: body.params?.rationale,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (body.command === 'sysml.workflow.log') {
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.workflow.state') {
      return new Response(
        JSON.stringify(
          opts.workflowState ?? {
            approval: null,
            assignee: null,
            sign_offs: [],
            suspect_clearings: [],
            verification_attestations: [],
            comment_count: 0,
            orphaned: false,
          },
        ),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (
      body.command === 'sysml.workflow.comment' ||
      body.command === 'sysml.workflow.assign' ||
      body.command === 'sysml.workflow.set_approval' ||
      body.command === 'sysml.workflow.sign_off'
    ) {
      // The mutations only need a successful event envelope back.
      return new Response(
        JSON.stringify({
          seq: 9,
          schema_version: 1,
          project: body.params?.project,
          element_id: body.params?.element_id,
          actor: body.params?.actor,
          timestamp_ms: 1_700_000_000_000,
          kind: 'comment',
          body: 'echo',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (body.command === 'sysml.workspace.verify') {
      return new Response(JSON.stringify({ total_cases: 0, passed: 0, failed: 0 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.get_source') {
      return new Response(
        JSON.stringify({
          text: "requirement <'REQ-TRIP-01'> TripTime {\n  doc /* trip fast */\n}",
          start: 120,
          end: 260,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (body.command === 'sysml.workspace.requirement_detail') {
      const detail = opts.detail ?? emptyDetail(String(body.params?.element_id ?? ''));
      return new Response(JSON.stringify(detail), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.query') {
      return new Response(JSON.stringify({ rows: opts.linkCandidates ?? [] }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (
      /^sysml\.workspace\.(edit|add)_/.test(body.command) ||
      body.command === 'sysml.workspace.create_requirement'
    ) {
      if (opts.fieldEditError) {
        return new Response(JSON.stringify({ error: opts.fieldEditError }), { status: 400 });
      }
      return new Response(JSON.stringify(opts.fieldEdit), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    if (body.command === 'sysml.load_source') {
      return new Response(JSON.stringify(null), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({}), { status: 200 });
  });
  vi.stubGlobal('fetch', fetchMock);

  useWorkspaceUIStore.setState({
    workspaceRoot: opts.workspaceRoot === undefined ? '/ws/demo' : opts.workspaceRoot,
  });

  const utils = render(withQueryClient(<RequirementsWorkflow />));
  return { ...utils, fetchMock };
}

/** Land on the grid: wait out loading, then enter via a landing card. */
async function mountOnGrid(opts: HarnessOpts = {}) {
  const utils = mountWorkflow(opts);
  await waitFor(() => {
    expect(screen.getByTestId('requirements-landing')).toBeDefined();
  });
  fireEvent.click(screen.getByTestId('requirements-landing-maturity'));
  await waitFor(() => {
    expect(screen.getByTestId('requirements-grid')).toBeDefined();
  });
  return utils;
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  window.localStorage.clear();
  useRequirementEditStore.setState({ editingKey: null, pendingKey: null, failed: null });
  useWorkspaceStore.setState({ loadedFiles: new Map() });
  useWorkspaceUIStore.setState({ workspaceRoot: null });
  useBaselineStore.setState({ selected: null });
  useWorkflowActorStore.setState({ actor: null });
  useRequirementsSelectionStore.setState({ selectedRow: null });
  useRightRailStore.setState({ pinned: null, transient: null });
  for (const id of [LEFT_RAIL_SLOT_ID, BOTTOM_STRIP_SLOT_ID]) {
    document.getElementById(id)?.remove();
  }
});

// ── Tests ────────────────────────────────────────────────────────────

describe('RequirementsWorkflow — pre-table states', () => {
  it('renders the no-workspace state when nothing is loaded', () => {
    mountWorkflow({ workspaceRoot: null });
    expect(screen.getByTestId('requirements-no-workspace')).toBeDefined();
  });

  it('renders the loading state while the query is in flight', () => {
    mountWorkflow({ neverResolve: true });
    expect(screen.getByTestId('requirements-loading')).toBeDefined();
  });

  it('renders the error state with a retry button on fetch failure', async () => {
    mountWorkflow({ failFetch: true });
    await waitFor(() => {
      expect(screen.getByTestId('requirements-error')).toBeDefined();
    });
    expect(screen.getByTestId('requirements-retry')).toBeDefined();
  });

  it('renders the teaching empty state when the model has no requirements', async () => {
    mountWorkflow({ rows: [] });
    await waitFor(() => {
      expect(screen.getByTestId('requirements-empty')).toBeDefined();
    });
    // The authoring example snippet is the point of the empty state.
    expect(screen.getByTestId('requirements-empty').textContent).toContain(
      'requirement def',
    );
  });
});

describe('RequirementsWorkflow — landing (R15)', () => {
  it('lands on the activity state with honest counts', async () => {
    mountWorkflow();
    await waitFor(() => {
      expect(screen.getByTestId('requirements-landing')).toBeDefined();
    });
    expect(screen.getByText('This model declares 4 requirements')).toBeDefined();
    // unverified = total − passed = 3; failing = 1.
    expect(screen.getByTestId('requirements-landing-unverified').textContent).toContain('3');
    expect(screen.getByTestId('requirements-landing-failing').textContent).toContain('1');
  });

  it('the unverified card enters the grid pre-filtered', async () => {
    mountWorkflow();
    await waitFor(() => {
      expect(screen.getByTestId('requirements-landing')).toBeDefined();
    });
    fireEvent.click(screen.getByTestId('requirements-landing-unverified'));
    await waitFor(() => {
      expect(screen.getByTestId('requirements-grid')).toBeDefined();
    });
    expect(screen.getByTestId('requirements-count').textContent).toBe('3 of 4');
    expect(screen.queryByTestId('req-row-REQ-TRIP-01')).toBeNull();
    expect(screen.getByTestId('req-row-REQ-SENS-01')).toBeDefined();
  });
});

describe('RequirementsWorkflow — grid mode (demo 1a)', () => {
  it('renders package dividers and all rows in document order', async () => {
    await mountOnGrid();
    const dividers = screen.getAllByTestId('requirements-grid-package');
    expect(dividers.map((d) => d.textContent)).toEqual([
      'TripUnit1',
      'Sensing2',
      'UserProtection1',
    ]);
    expect(screen.getByTestId('req-row-REQ-TRIP-01')).toBeDefined();
    expect(screen.getByTestId('req-row-REQ-USER-01')).toBeDefined();
  });

  it('renders the four verified-chip states with in-UI labeling', async () => {
    await mountOnGrid();
    const chips = screen.getAllByTestId('req-verified-chip');
    const variants = chips.map((c) => c.getAttribute('data-variant'));
    expect(variants).toEqual(['pass', 'fail', 'outline', 'none']);
    // The §5 ruling: the three-state chip MUST be labeled in-UI.
    for (const chip of chips) {
      expect(chip.getAttribute('title')).toBeTruthy();
    }
    // Calm pass: the rollup reads by colour + glyph on the bare ground (no
    // filled pill). Pass already carried ✓; fail now leads with ✗ so the
    // state stays colour-blind-safe without the red fill.
    expect(chips[0].textContent).toBe('2/2 ✓');
    expect(chips[1].textContent).toBe('✗ 1/4');
  });

  it('renders the declared-method chip (B4) only where a case declares one', async () => {
    await mountOnGrid();
    // Only TripTime's verifying cases declare methods; the chip is
    // neutral (model intent) and labeled against evaluation_mode.
    const chips = screen.getAllByTestId('req-method-chip');
    expect(chips).toHaveLength(1);
    expect(chips[0].textContent).toBe('test · analyze');
    expect(chips[0].getAttribute('title')).toContain('model intent');
  });

  it('row click selects the row and opens the links rail context', async () => {
    await mountOnGrid();
    fireEvent.click(screen.getByTestId('req-row-REQ-SENS-01'));
    expect(useRequirementsSelectionStore.getState().selectedRow?.req_id).toBe('REQ-SENS-01');
    expect(useRightRailStore.getState().transient).toBe(REQUIREMENTS_LINKS_CONTEXT_ID);
  });
});

describe('RequirementsWorkflow — document mode (demo 1b)', () => {
  it('toggles to document mode, preserving rows and selection', async () => {
    await mountOnGrid();
    fireEvent.click(screen.getByTestId('req-row-REQ-TRIP-01'));
    fireEvent.click(screen.getByTestId('requirements-mode-document'));
    await waitFor(() => {
      expect(screen.getByTestId('requirements-document')).toBeDefined();
    });
    expect(screen.getByText('The breaker shall trip within 40 ms.')).toBeDefined();
    // Selection survives the toggle (one item store, two renderings).
    expect(
      screen.getByTestId('req-doc-REQ-TRIP-01').getAttribute('style'),
    ).toContain('inset 2px 0 0');
  });

  it('edits the statement text in document view (double-click the paragraph)', async () => {
    seedTripBuffer();
    const newBody = 'The breaker shall trip within 25 ms.';
    const { fetchMock } = await mountOnGrid({ fieldEdit: docEditFixture(newBody) });
    fireEvent.click(screen.getByTestId('requirements-mode-document'));
    await screen.findByTestId('requirements-document');

    fireEvent.doubleClick(screen.getByTestId('req-doc-text-REQ-TRIP-01'));
    const editor = await screen.findByTestId('req-doc-editor');
    fireEvent.change(editor, { target: { value: newBody } });
    fireEvent.keyDown(editor, { key: 'Enter', metaKey: true });
    await waitFor(() => {
      const computes = commandCalls(fetchMock, 'sysml.workspace.edit_requirement_doc');
      expect(computes.length).toBe(1);
      expect(computes[0].params).toEqual({ element_id: 'e-1', new_text: newBody });
    });
  });

  it('edits maturity in document view — parity with the grid (§7.7 build set)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountOnGrid({ fieldEdit: maturityInsertFixture() });
    fireEvent.click(screen.getByTestId('requirements-mode-document'));
    await screen.findByTestId('requirements-document');

    // REQ-USER-01 (e-4) has null maturity → routes to add_requirement_maturity.
    fireEvent.doubleClick(screen.getByTestId('req-doc-maturity-REQ-USER-01'));
    const select = await screen.findByTestId('req-doc-maturity-select');
    fireEvent.change(select, { target: { value: 'tbd' } });
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_requirement_maturity');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ element_id: 'e-4', status: 'tbd' });
    });
  });
});

describe('RequirementsWorkflow — trace mode (coverage sub-view, R7)', () => {
  it('mounts the shared trace-matrix panel in place of the table', async () => {
    await mountOnGrid();
    fireEvent.click(screen.getByTestId('requirements-mode-trace'));
    await waitFor(() => {
      expect(screen.queryByTestId('requirements-grid')).toBeNull();
    });
    // The fenced panel owns its own internal testids — assert it
    // mounted without pinning which state it settled in.
    expect(document.querySelector('[data-testid^="trace-matrix"]')).toBeTruthy();
  });
});

describe('RequirementsWorkflow — left rail + strip', () => {
  it('portals the package list and view presets into the rail slot', async () => {
    await mountOnGrid();
    expect(screen.getByTestId('requirements-rail')).toBeDefined();
    expect(screen.getByTestId('requirements-rail-package-Sensing')).toBeDefined();
    // Clicking a package filters the table.
    fireEvent.click(screen.getByTestId('requirements-rail-package-Sensing'));
    expect(screen.getByTestId('requirements-count').textContent).toBe('2 of 4');
    // Clicking a view preset composes with the package filter.
    fireEvent.click(screen.getByTestId('requirements-rail-view-failing'));
    expect(screen.getByTestId('requirements-count').textContent).toBe('1 of 4');
  });

  it('renders the coverage strip with honest numbers', async () => {
    await mountOnGrid();
    const strip = screen.getByTestId('requirements-strip');
    expect(strip.textContent).toContain('Coverage 25%');
    expect(strip.textContent).toContain('3 unverified');
    expect(strip.textContent).toContain('1 failing');
    expect(strip.textContent).toContain('4 requirements');
  });
});

describe('requirements-links rail context', () => {
  it('is registered with the rail registry at module load', () => {
    expect(getRailContext(REQUIREMENTS_LINKS_CONTEXT_ID)).toBeDefined();
  });

  /** Mount the grid (installs the fetch stub), select a row, render the
   *  registered rail context body the way AppShell would, and wait for
   *  the contract fetch to have gone out (so absence assertions are
   *  post-settle, not pre-fetch). */
  async function mountRailBody(opts: HarnessOpts, rowTestId: string) {
    const { fetchMock } = await mountOnGrid(opts);
    fireEvent.click(screen.getByTestId(rowTestId));
    const ctx = getRailContext(REQUIREMENTS_LINKS_CONTEXT_ID);
    if (!ctx) throw new Error('rail context not registered');
    render(withQueryClient(ctx.render()));
    await waitFor(() => {
      expect(screen.getByTestId('requirements-links-body')).toBeDefined();
      expect(
        fetchMock.mock.calls.some(([, init]) =>
          String(init?.body ?? '').includes('sysml.workspace.requirement_detail'),
        ),
      ).toBe(true);
    });
    // One settle pass so the resolved detail has rendered (or provably
    // rendered nothing) before assertions run.
    await new Promise((resolve) => setTimeout(resolve, 0));
    return { fetchMock };
  }

  const CONTRACT_DETAIL: RequirementDetail = {
    id: 'e-1',
    subject: { id: 's-1', name: 'breaker', kind: 'SubjectMembership' },
    assumed_constraints: [
      {
        id: 'c-a1',
        name: null,
        text: 'ambientTemp <= 40 [degC]',
        referenced_definition: null,
      },
    ],
    required_constraints: [
      { id: 'c-r1', name: null, text: 'actualTripTime <= maxTripTime', referenced_definition: null },
      {
        id: 'c-r2',
        name: null,
        text: null,
        referenced_definition: { id: 'd-1', name: 'MassLimit', kind: 'ConstraintDefinition' },
      },
    ],
    inherited_assumed_constraints: [],
    inherited_required_constraints: [],
    instantiated_by: [],
    framed_concerns: [{ id: 'fc-1', name: 'safety', kind: 'FramedConcernMembership' }],
    actors: [{ id: 'a-1', name: 'driver', kind: 'ActorMembership' }],
    stakeholders: [],
    referenced_attributes: [
      { id: 'at-1', name: 'maxTripTime', value: '40 [ms]', live_value: null },
    ],
    rationale: 'Threshold from the 2025 trade study.',
    verification_methods: [],
  };

  it('renders the verdict-input block adjacent to the verified chips (R18)', async () => {
    await mountRailBody({ detail: CONTRACT_DETAIL }, 'req-row-REQ-TRIP-01');
    const contract = await screen.findByTestId('requirements-verdict-inputs');
    expect(contract.textContent).toContain('breaker');
    expect(contract.textContent).toContain('ambientTemp <= 40 [degC]');
    expect(contract.textContent).toContain('actualTripTime <= maxTripTime');
    // Reference form shows its linked definition, not fake body text.
    expect(contract.textContent).toContain(': MassLimit');
    expect(contract.textContent).toContain('maxTripTime = 40 [ms]');
    // Verbatim-source honesty label rides on the constraint rows.
    const assume = screen.getByTestId('requirements-constraint-assume');
    expect(assume.getAttribute('title')).toBe('verbatim source text');
    // Adjacency: the contract block sits immediately before the
    // "verified by" chips in the rail flow (§2.1 placement ruling).
    const body = screen.getByTestId('requirements-links-body');
    const flow = body.textContent ?? '';
    expect(flow.indexOf('contract')).toBeLessThan(flow.indexOf('verified by'));
    expect(flow.indexOf('satisfied by')).toBeLessThan(flow.indexOf('contract'));
  });

  it('keeps narrative roles OUT of the verdict block (§2.1 bucket separation)', async () => {
    await mountRailBody({ detail: CONTRACT_DETAIL }, 'req-row-REQ-TRIP-01');
    const contract = await screen.findByTestId('requirements-verdict-inputs');
    expect(contract.textContent).not.toContain('driver');
    expect(contract.textContent).not.toContain('safety');
    const narrative = screen.getByTestId('requirements-narrative');
    expect(narrative.textContent).toContain('driver');
    expect(narrative.textContent).toContain('safety');
    expect(screen.getByTestId('requirements-rationale').textContent).toContain(
      'Threshold from the 2025 trade study.',
    );
  });

  it('an empty contract still hosts the authoring affordances (§7.7), with no inputs shown', async () => {
    await mountRailBody({}, 'req-row-REQ-USER-01');
    // The contract + narrative blocks now always render to host the add
    // affordances; they just show no subject/constraint/attribute/rationale
    // rows for an empty contract.
    expect(screen.getByTestId('req-add-attribute')).toBeDefined();
    expect(screen.getByTestId('req-add-rationale')).toBeDefined();
    // The subject/actor/etc. pickers are present (empty contract → the
    // add-subject affordance shows), but no actual input rows.
    expect(screen.getByTestId('req-add-role-subject')).toBeDefined();
    expect(screen.queryByTestId('requirements-rationale')).toBeNull();
    const contract = screen.getByTestId('requirements-verdict-inputs');
    expect(contract.querySelector('[data-testid^="requirements-constraint-"]')).toBeNull();
  });

  it('adds an actor through the role picker — type pick pre-fills the name (§7.7)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody(
      {
        detail: emptyDetail('e-1'),
        linkCandidates: [
          { id: 'p-inst', name: 'Installer', qualified_name: 'ProtectionSpec::Installer', kind: 'PartDefinition' },
          { id: 'p-reg', name: 'Regulator', qualified_name: 'ProtectionSpec::Regulator', kind: 'PartDefinition' },
        ],
        fieldEdit: {
          uri: TRIP_URI,
          element_id: 'e-1',
          field: 'add_requirement_role',
          edit: {
            line_start: 3,
            col_start: 0,
            line_end: 3,
            col_end: 0,
            new_text: '\t\tactor installer : ProtectionSpec::Installer;\n',
          },
        },
      },
      'req-row-REQ-TRIP-01',
    );
    fireEvent.click(screen.getByTestId('req-add-role-actor'));
    const typeInput = await screen.findByTestId('req-role-actor-type');
    // Type a partial, wait for candidates to load into the dropdown, then pick
    // the suggestion (which fires onChange with the full value + loaded options).
    fireEvent.focus(typeInput);
    fireEvent.change(typeInput, { target: { value: 'Inst' } });
    const suggestions = await screen.findByTestId('req-role-actor-type-suggestions');
    fireEvent.mouseDown(within(suggestions).getByText('Installer'));
    // Picking the type pre-fills the name field (lowercased), shown editable.
    const nameInput = screen.getByTestId('req-role-actor-name') as HTMLInputElement;
    await waitFor(() => expect(nameInput.value).toBe('installer'));
    fireEvent.click(screen.getByTestId('req-role-actor-submit'));
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_requirement_role');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({
        requirement_id: 'e-1',
        role: 'actor',
        type_id: 'p-inst',
        name: 'installer',
      });
    });
    // The subject picker only shows when the requirement has no subject.
    expect(screen.getByTestId('req-add-role-subject')).toBeDefined();
  });

  it('adds a require constraint through the contract-block affordance (§7.7)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody(
      {
        detail: emptyDetail('e-1'),
        fieldEdit: {
          uri: TRIP_URI,
          element_id: 'e-1',
          field: 'add_constraint',
          edit: {
            line_start: 3,
            col_start: 0,
            line_end: 3,
            col_end: 0,
            new_text: '\t\trequire constraint fastEnough { maxTripTime <= 50 }\n',
          },
        },
      },
      'req-row-REQ-TRIP-01',
    );
    fireEvent.click(screen.getByTestId('req-add-constraint'));
    fireEvent.change(await screen.findByTestId('req-add-constraint-name'), {
      target: { value: 'fastEnough' },
    });
    fireEvent.change(screen.getByTestId('req-add-constraint-expr'), {
      target: { value: 'maxTripTime <= 50' },
    });
    fireEvent.click(screen.getByTestId('req-add-constraint-submit'));
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_constraint');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({
        element_id: 'e-1',
        kind: 'require',
        expr: 'maxTripTime <= 50',
        name: 'fastEnough',
      });
    });
  });

  it('will not commit a constraint expression containing braces (client guard)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody({ detail: emptyDetail('e-1') }, 'req-row-REQ-TRIP-01');
    fireEvent.click(screen.getByTestId('req-add-constraint'));
    fireEvent.change(await screen.findByTestId('req-add-constraint-expr'), {
      target: { value: 'x > { 0 }' },
    });
    fireEvent.click(screen.getByTestId('req-add-constraint-submit'));
    expect(commandCalls(fetchMock, 'sysml.workspace.add_constraint').length).toBe(0);
  });

  it('adds an attribute through the contract-block affordance (§7.7)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody(
      {
        detail: emptyDetail('e-1'),
        fieldEdit: {
          uri: TRIP_URI,
          element_id: 'e-1',
          field: 'add_attribute',
          edit: {
            line_start: 3,
            col_start: 0,
            line_end: 3,
            col_end: 0,
            new_text: '\t\tattribute resetTime = 5;\n',
          },
        },
      },
      'req-row-REQ-TRIP-01',
    );
    fireEvent.click(screen.getByTestId('req-add-attribute'));
    fireEvent.change(await screen.findByTestId('req-add-attribute-name'), {
      target: { value: 'resetTime' },
    });
    fireEvent.change(screen.getByTestId('req-add-attribute-value'), { target: { value: '5' } });
    fireEvent.click(screen.getByTestId('req-add-attribute-submit'));
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_attribute');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ element_id: 'e-1', name: 'resetTime', value: '5' });
    });
  });

  it('rejects an invalid attribute name client-side without posting', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody({ detail: emptyDetail('e-1') }, 'req-row-REQ-TRIP-01');
    fireEvent.click(screen.getByTestId('req-add-attribute'));
    fireEvent.change(await screen.findByTestId('req-add-attribute-name'), {
      target: { value: 'has space' },
    });
    fireEvent.click(screen.getByTestId('req-add-attribute-submit'));
    expect(commandCalls(fetchMock, 'sysml.workspace.add_attribute').length).toBe(0);
  });

  it('adds a rationale through the narrative-bucket affordance (§7.7)', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRailBody(
      {
        detail: emptyDetail('e-1'),
        fieldEdit: {
          uri: TRIP_URI,
          element_id: 'e-1',
          field: 'add_rationale',
          edit: {
            line_start: 3,
            col_start: 0,
            line_end: 3,
            col_end: 0,
            new_text: '\t\t@Rationale { text = "Because reasons."; }\n',
          },
        },
      },
      'req-row-REQ-TRIP-01',
    );
    fireEvent.click(screen.getByTestId('req-add-rationale'));
    const editor = await screen.findByTestId('req-rationale-editor');
    fireEvent.change(editor, { target: { value: 'Because reasons.' } });
    fireEvent.keyDown(editor, { key: 'Enter' });
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_rationale');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ element_id: 'e-1', text: 'Because reasons.' });
    });
  });

  it('shows the declared method under the verified-by chips, hidden when absent (B4)', async () => {
    await mountRailBody({}, 'req-row-REQ-TRIP-01');
    const method = screen.getByTestId('requirements-rail-method');
    expect(method.textContent).toContain('declared method');
    expect(method.textContent).toContain('test · analyze');
    cleanup();
    await mountRailBody({}, 'req-row-REQ-USER-01');
    expect(screen.queryByTestId('requirements-rail-method')).toBeNull();
  });

  it('never sends the check-occurrence reveal flag (Verify-lens concern, not this table)', async () => {
    const { fetchMock } = await mountOnGrid();
    const specs = fetchMock.mock.calls
      .map(([, init]) => JSON.parse(String(init?.body ?? '{}')))
      .filter((b) => b.command === 'sysml.workspace.requirement_rows')
      .map((b) => b.params.spec);
    expect(specs.length).toBeGreaterThan(0);
    expect(specs.every((s) => s.include_verification_occurrences === undefined)).toBe(true);
  });

  it('link chips navigate to rows that exist in the table', async () => {
    await mountRailBody({}, 'req-row-REQ-TRIP-01');
    // REQ-TRIP-01 derives SensThreshold (e-2) — a real table row, so its
    // chip is a navigation button; clicking selects that row.
    const link = await screen.findByTestId('rail-link-e-2');
    fireEvent.click(link);
    expect(useRequirementsSelectionStore.getState().selectedRow?.id).toBe('e-2');
    // verified_by targets a case (e-v1) — not a table row, stays plain.
    expect(screen.queryByTestId('rail-link-e-v1')).toBeNull();
  });

  it('shows the declaration source behind a collapsed toggle', async () => {
    const { fetchMock } = await mountRailBody({}, 'req-row-REQ-TRIP-01');
    const toggle = screen.getByTestId('requirements-source-toggle');
    expect(toggle.textContent).toContain('TripUnit.sysml:7');
    // Collapsed: no fetch yet.
    expect(
      fetchMock.mock.calls.some(([, init]) =>
        String(init?.body ?? '').includes('sysml.get_source'),
      ),
    ).toBe(false);
    fireEvent.click(toggle);
    const text = await screen.findByTestId('requirements-source-text');
    expect(text.textContent).toContain("requirement <'REQ-TRIP-01'>");
  });

  it('lists content instantiations on the def detail', async () => {
    const detail: RequirementDetail = {
      ...emptyDetail('e-1'),
      instantiated_by: [
        { id: 'u-1', name: 'tripReq', kind: 'RequirementUsage' },
      ],
    };
    await mountRailBody({ detail }, 'req-row-REQ-TRIP-01');
    const group = await screen.findByTestId('requirements-instantiated-by');
    expect(group.textContent).toContain('instantiated by');
    expect(group.textContent).toContain('tripReq');
  });

  it('labels inherited constraints with their typing-target provenance', async () => {
    const detail: RequirementDetail = {
      ...emptyDetail('e-1'),
      inherited_required_constraints: [
        {
          id: 'c-i1',
          name: 'mustTrip',
          text: 'tripped == true',
          referenced_definition: null,
          inherited_from: { id: 'd-9', name: 'TripAt5xRated', kind: 'RequirementDefinition' },
        },
      ],
    };
    await mountRailBody({ detail }, 'req-row-REQ-TRIP-01');
    const contract = await screen.findByTestId('requirements-verdict-inputs');
    expect(contract.textContent).toContain('tripped == true');
    // Provenance labeling is binding — the row says where it came from.
    const from = screen.getByTestId('requirements-constraint-inherited-from');
    expect(from.textContent).toContain('from TripAt5xRated');
  });

  it('gates workflow writes on the actor identity, then posts typed commands', async () => {
    const { fetchMock } = await mountRailBody({}, 'req-row-REQ-TRIP-01');
    expect(screen.getByTestId('requirements-workflow-controls')).toBeDefined();
    // No actor yet → the one-time identity prompt, no write inputs.
    expect(screen.queryByTestId('workflow-comment-input')).toBeNull();
    fireEvent.change(screen.getByTestId('workflow-actor-input'), {
      target: { value: 'Ricky' },
    });
    fireEvent.click(screen.getByTestId('workflow-actor-save'));

    // Comment: Enter records (trimmed, signed); the input clears.
    const commentInput = screen.getByTestId('workflow-comment-input') as HTMLInputElement;
    fireEvent.change(commentInput, { target: { value: '  looks good  ' } });
    fireEvent.keyDown(commentInput, { key: 'Enter' });
    await waitFor(() => {
      const calls = fetchMock.mock.calls
        .map(([, init]) => JSON.parse(String(init?.body ?? '{}')))
        .filter((b) => b.command === 'sysml.workflow.comment');
      expect(calls).toHaveLength(1);
      expect(calls[0].params).toMatchObject({
        element_id: 'e-1',
        body: 'looks good',
        actor: 'Ricky',
      });
    });
    expect(commentInput.value).toBe('');

    // Sign-off goes to its own typed command.
    const signOffInput = screen.getByTestId('workflow-signoff-input');
    fireEvent.change(signOffInput, { target: { value: 'reviewed rev B' } });
    fireEvent.keyDown(signOffInput, { key: 'Enter' });
    await waitFor(() => {
      const calls = fetchMock.mock.calls
        .map(([, init]) => JSON.parse(String(init?.body ?? '{}')))
        .filter((b) => b.command === 'sysml.workflow.sign_off');
      expect(calls).toHaveLength(1);
      expect(calls[0].params).toMatchObject({
        element_id: 'e-1',
        statement: 'reviewed rev B',
        actor: 'Ricky',
      });
    });
  });

  it('drives the approval stepper from folded state and posts to-only transitions', async () => {
    useWorkflowActorStore.setState({ actor: 'Ricky' });
    const { fetchMock } = await mountRailBody(
      {
        workflowState: {
          approval: ['in_review', 'sam', 1_700_000_000_000],
          assignee: 'sam',
          sign_offs: [],
          suspect_clearings: [],
          verification_attestations: [],
          comment_count: 0,
          orphaned: false,
        },
      },
      'req-row-REQ-TRIP-01',
    );
    // Approval is a STEPPER in the process zone (register ruling), never
    // a select/chip. The current step reflects folded SERVER state, not
    // the vocabulary default.
    const inReview = await screen.findByTestId('workflow-approval-step-in_review');
    await waitFor(() => expect(inReview.getAttribute('data-current')).toBe('true'));
    expect(
      screen.getByTestId('workflow-approval-step-draft').getAttribute('data-current'),
    ).toBeNull();
    // The latest assignee shows as the assign input's resting text.
    expect(
      (screen.getByTestId('workflow-assign-input') as HTMLInputElement).placeholder,
    ).toBe('sam');

    fireEvent.click(screen.getByTestId('workflow-approval-step-approved'));
    await waitFor(() => {
      const calls = fetchMock.mock.calls
        .map(([, init]) => JSON.parse(String(init?.body ?? '{}')))
        .filter((b) => b.command === 'sysml.workflow.set_approval');
      expect(calls).toHaveLength(1);
      expect(calls[0].params).toMatchObject({
        element_id: 'e-1',
        to: 'approved',
        actor: 'Ricky',
      });
      // `from` is server-derived — the client must never claim it.
      expect(calls[0].params.from).toBeUndefined();
    });
  });

  it('zones the rail into computed / model / process registers', async () => {
    useWorkflowActorStore.setState({ actor: 'Ricky' });
    await mountRailBody({}, 'req-row-REQ-TRIP-01');
    const body = screen.getByTestId('requirements-links-body');
    const digest = screen.getByTestId('requirements-computed-digest');
    const model = screen.getByTestId('requirements-model-zone');
    const process = screen.getByTestId('requirements-process-zone');
    // Zone order is the register geography: computed top, model middle,
    // process bottom.
    const order = [...body.children];
    expect(order.indexOf(digest)).toBeLessThan(order.indexOf(model));
    expect(order.indexOf(model)).toBeLessThan(order.indexOf(process));
    // The computed digest carries the tool's read — the verified rollup
    // chip lives there, marked `ƒ`, with no edit affordances.
    expect(within(digest).getByTestId('req-verified-chip')).toBeDefined();
    expect(digest.textContent).toContain('ƒ');
    expect(within(digest).queryAllByRole('button')).toHaveLength(0);
    // Model content (contract, links, source) is inside the model zone;
    // workflow controls + history are inside the process zone.
    expect(within(model).getByTestId('requirements-verdict-inputs')).toBeDefined();
    expect(within(model).getByTestId('requirements-source-toggle')).toBeDefined();
    expect(within(process).getByTestId('requirements-workflow-controls')).toBeDefined();
    expect(within(process).getByTestId('requirements-history')).toBeDefined();
    expect(within(process).getByTestId('workflow-approval-stepper')).toBeDefined();
    // Maturity stays a mono chip in the MODEL contract — never in the
    // process zone (the maturity-vs-approval ruling).
    expect(within(model).getByTestId('req-rail-maturity')).toBeDefined();
    expect(within(process).queryByTestId('req-rail-maturity')).toBeNull();
  });
});

// ── v1.5: baselines + suspect ────────────────────────────────────────

const FIXTURE_BASELINES: BaselineMeta[] = [
  {
    name: 'B2',
    commit: 'a41c9f00aa',
    created_at: 1_751_414_400, // newest first
    // B6: dirty-tree capture — recorded honestly, marked in the picker.
    provenance: { sha: 'f00dfacefeedbeeff00dfacefeedbeeff00dface', dirty: true, branch: 'main' },
  },
  { name: 'B1', commit: '8d21e400bb', created_at: 1_747_180_800 },
];

const FIXTURE_SUSPECTS: SuspectRecordWire[] = [
  {
    requirement: 'e-1',
    causes: [
      {
        kind: 'text_changed',
        element: 'doc-1',
        from: 'The breaker shall trip within 60 ms.',
        to: 'The breaker shall trip within 40 ms.',
      },
    ],
  },
  { requirement: 'e-2', causes: [{ kind: 'upstream_suspect', via: 'e-1' }] },
];

describe('RequirementsWorkflow — baselines + suspect (v1.5)', () => {
  it('shows a neutral pill and no suspect column when no baseline exists', async () => {
    await mountOnGrid();
    expect(screen.getByTestId('baseline-pill').textContent).toContain('No baseline');
    expect(screen.queryByTestId('requirements-suspect-header')).toBeNull();
    expect(screen.queryByTestId('requirements-rail-view-suspect')).toBeNull();
    expect(screen.queryByTestId('requirements-strip-suspect')).toBeNull();
  });

  it('auto-selects the newest baseline and lights up flags, strip and view', async () => {
    await mountOnGrid({ baselines: FIXTURE_BASELINES, suspects: FIXTURE_SUSPECTS });
    await waitFor(() => {
      expect(screen.getByTestId('baseline-pill').textContent).toContain('Baseline B2');
    });
    await waitFor(() => {
      expect(screen.getByTestId('requirements-suspect-header')).toBeDefined();
    });
    // Direct change + upstream propagation both flagged.
    expect(screen.getByTestId('suspect-flag-REQ-TRIP-01')).toBeDefined();
    expect(screen.getByTestId('suspect-flag-REQ-SENS-01')).toBeDefined();
    // Untouched row carries no flag.
    expect(screen.queryByTestId('suspect-flag-REQ-USER-01')).toBeNull();
    // Strip: count + baseline label.
    expect(screen.getByTestId('requirements-strip-suspect').textContent).toContain('2');
    expect(screen.getByTestId('requirements-strip-baseline').textContent).toContain(
      'Baseline B2',
    );
    // Rail: the Suspect view filters to the flagged rows.
    const suspectView = screen.getByTestId('requirements-rail-view-suspect');
    expect(suspectView.textContent).toContain('Suspect since B2');
    fireEvent.click(suspectView);
    expect(screen.getByTestId('requirements-count').textContent).toBe('2 of 4');
  });

  it('opens the anchored popover with diff excerpt and downstream impact', async () => {
    await mountOnGrid({ baselines: FIXTURE_BASELINES, suspects: FIXTURE_SUSPECTS });
    await waitFor(() => {
      expect(screen.getByTestId('suspect-flag-REQ-TRIP-01')).toBeDefined();
    });
    fireEvent.click(screen.getByTestId('suspect-flag-REQ-TRIP-01'));
    const popover = screen.getByTestId('suspect-popover');
    expect(popover.textContent).toContain('Changed since baseline B2');
    expect(popover.textContent).toContain('The breaker shall trip within 60 ms.');
    expect(popover.textContent).toContain('The breaker shall trip within 40 ms.');
    // Downstream impact from row link refs (verified_by + derives).
    expect(popover.textContent).toContain('1 verification results to re-check');
    expect(popover.textContent).toContain('1 derived requirements');
    // Close.
    fireEvent.click(screen.getByTestId('suspect-popover-close'));
    expect(screen.queryByTestId('suspect-popover')).toBeNull();
  });

  it('renders prop_text_changed causes as labeled before/after deltas (W4)', async () => {
    const suspects: SuspectRecordWire[] = [
      {
        requirement: 'e-1',
        causes: [
          {
            kind: 'prop_text_changed',
            element: 'con-1',
            element_kind: 'RequirementConstraintMembership',
            key: 'constraint',
            from: 'actualTime <= 60 [ms]',
            to: 'actualTime <= 40 [ms]',
          },
        ],
      },
    ];
    await mountOnGrid({ baselines: FIXTURE_BASELINES, suspects });
    await waitFor(() => {
      expect(screen.getByTestId('suspect-flag-REQ-TRIP-01')).toBeDefined();
    });
    fireEvent.click(screen.getByTestId('suspect-flag-REQ-TRIP-01'));
    const delta = screen.getByTestId('suspect-prop-delta');
    expect(delta.textContent).toContain('constraint');
    expect(delta.textContent).toContain('actualTime <= 60 [ms]');
    expect(delta.textContent).toContain('actualTime <= 40 [ms]');
    // A rendered delta means no fallback summary line.
    expect(screen.queryByTestId('suspect-nontext-change')).toBeNull();
  });

  it('collects the actor once, requires a rationale, and posts the attestation', async () => {
    const { fetchMock } = await mountOnGrid({
      baselines: FIXTURE_BASELINES,
      suspects: FIXTURE_SUSPECTS,
    });
    await waitFor(() => {
      expect(screen.getByTestId('suspect-flag-REQ-TRIP-01')).toBeDefined();
    });
    fireEvent.click(screen.getByTestId('suspect-flag-REQ-TRIP-01'));

    // No actor yet → the identity prompt, no attest button.
    expect(screen.queryByTestId('suspect-attest')).toBeNull();
    fireEvent.change(screen.getByTestId('workflow-actor-input'), {
      target: { value: 'Ricky' },
    });
    fireEvent.click(screen.getByTestId('workflow-actor-save'));

    // Actor set → rationale is required before attesting.
    const attest = screen.getByTestId('suspect-attest') as HTMLButtonElement;
    expect(attest.disabled).toBe(true);
    fireEvent.change(screen.getByTestId('suspect-rationale-input'), {
      target: { value: 'tightened timing, intent unchanged' },
    });
    expect((screen.getByTestId('suspect-attest') as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByTestId('suspect-attest'));

    await waitFor(() => {
      const calls = fetchMock.mock.calls
        .map(([, init]) => (init?.body ? JSON.parse(String(init.body)) : {}))
        .filter((b) => b.command === 'sysml.workflow.attest_suspect_clearing');
      expect(calls.length).toBe(1);
      expect(calls[0].params).toMatchObject({
        project: 'demo',
        element_id: 'e-1',
        baseline: 'B2',
        rationale: 'tightened timing, intent unchanged',
        actor: 'Ricky',
      });
    });
    // Success closes the popover (the flag will drop on refetch).
    await waitFor(() => {
      expect(screen.queryByTestId('suspect-popover')).toBeNull();
    });
  });

  it('cleared_by rows carry no flag (non-superseded attestation applied)', async () => {
    await mountOnGrid({
      baselines: FIXTURE_BASELINES,
      suspects: [
        { ...FIXTURE_SUSPECTS[0], cleared_by: 7 },
        FIXTURE_SUSPECTS[1],
      ],
    });
    await waitFor(() => {
      expect(screen.getByTestId('suspect-flag-REQ-SENS-01')).toBeDefined();
    });
    expect(screen.queryByTestId('suspect-flag-REQ-TRIP-01')).toBeNull();
    expect(screen.getByTestId('requirements-strip-suspect').textContent).toContain('1');
  });

  it('renders identity-changed honestly, never a fake before/after (ADR-009)', async () => {
    await mountOnGrid({
      baselines: FIXTURE_BASELINES,
      suspects: [{ requirement: 'e-3', causes: [{ kind: 'not_in_baseline' }] }],
    });
    await waitFor(() => {
      expect(screen.getByTestId('suspect-flag-REQ-SENS-02')).toBeDefined();
    });
    fireEvent.click(screen.getByTestId('suspect-flag-REQ-SENS-02'));
    const popover = screen.getByTestId('suspect-popover');
    expect(screen.getByTestId('suspect-identity-changed')).toBeDefined();
    expect(popover.textContent).toContain('identity changed');
  });

  it('creates a baseline through the dropdown inline input', async () => {
    const { fetchMock } = await mountOnGrid({ baselines: FIXTURE_BASELINES });
    await waitFor(() => {
      expect(screen.getByTestId('baseline-pill').textContent).toContain('Baseline B2');
    });
    fireEvent.click(screen.getByTestId('baseline-pill'));
    expect(screen.getByTestId('baseline-option-B1')).toBeDefined();
    // B6 provenance: the dirty-capture baseline is marked, the git info
    // rides the row tooltip; a provenance-less baseline shows neither.
    expect(screen.getByTestId('baseline-dirty-B2')).toBeDefined();
    expect(screen.getByTestId('baseline-option-B2').getAttribute('title')).toContain(
      'main @ f00dface',
    );
    expect(screen.queryByTestId('baseline-dirty-B1')).toBeNull();
    expect(screen.getByTestId('baseline-option-B1').getAttribute('title')).toBeNull();
    fireEvent.click(screen.getByTestId('baseline-new'));
    fireEvent.change(screen.getByTestId('baseline-name-input'), {
      target: { value: 'B3 — CDR' },
    });
    fireEvent.click(screen.getByTestId('baseline-name-submit'));
    await waitFor(() => {
      const calls = fetchMock.mock.calls
        .map(([, init]) => (init?.body ? JSON.parse(String(init.body)) : {}))
        .filter((b) => b.command === 'sysml.store.baseline.create');
      expect(calls.length).toBe(1);
      expect(calls[0].params.name).toBe('B3 — CDR');
      expect(calls[0].params.project).toBe('demo');
    });
  });
});

// ── Inline editing (v2 §7.5 — the six-step writeback loop) ───────────

const TRIP_URI = '/ws/demo/TripUnit.sysml';
const OLD_DOC = ' The breaker shall trip within 40 ms. ';
const TRIP_SOURCE = [
  'package TripUnit {',
  "\trequirement <'REQ-TRIP-01'> TripTime {",
  `\t\tdoc /*${OLD_DOC}*/`,
  '\t}',
  '}',
  '',
].join('\n');

function seedTripBuffer(source = TRIP_SOURCE) {
  useWorkspaceStore.setState({
    loadedFiles: new Map([[TRIP_URI, { uri: TRIP_URI, source, dirty: false, tree: [] }]]),
  });
}

/** The doc-body edit `edit_requirement_doc` would compute for e-1. */
function docEditFixture(newBody: string): FieldEditComputed {
  return {
    uri: TRIP_URI,
    element_id: 'e-1',
    field: 'doc',
    edit: {
      line_start: 2,
      col_start: 8, // after "\t\tdoc /*"
      line_end: 2,
      col_end: 8 + OLD_DOC.length,
      new_text: ` ${newBody} `,
      expected_old_text: OLD_DOC,
    },
  };
}

/** Guard-less @StatusInfo insertion (add_requirement_maturity shape). */
function maturityInsertFixture(): FieldEditComputed {
  return {
    uri: TRIP_URI,
    element_id: 'e-4',
    field: 'maturity',
    edit: {
      line_start: 3,
      col_start: 0,
      line_end: 3,
      col_end: 0,
      new_text: '\t\t@StatusInfo { status = StatusKind::tbd; }\n',
    },
  };
}

function commandCalls(fetchMock: ReturnType<typeof vi.fn>, command: string) {
  return fetchMock.mock.calls
    .map((call) => {
      const init = call[1] as RequestInit | undefined;
      return init?.body ? JSON.parse(String(init.body)) : {};
    })
    .filter((b: { command?: string }) => b.command === command);
}

describe('RequirementsWorkflow — inline editing (v2 §7.5)', () => {
  it('double-click opens the doc editor; Esc discards without any write', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountOnGrid();
    fireEvent.doubleClick(screen.getByTestId('req-text-cell-REQ-TRIP-01'));
    const editor = await screen.findByTestId('req-doc-editor');
    fireEvent.keyDown(editor, { key: 'Escape' });
    expect(screen.queryByTestId('req-doc-editor')).toBeNull();
    expect(commandCalls(fetchMock, 'sysml.workspace.edit_requirement_doc').length).toBe(0);
    expect(useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!.source).toBe(TRIP_SOURCE);
  });

  it('commits a doc edit: compute → guard → splice → load_source → badge clears', async () => {
    seedTripBuffer();
    const newBody = 'The breaker shall trip within 25 ms.';
    const { fetchMock } = await mountOnGrid({ fieldEdit: docEditFixture(newBody) });

    fireEvent.doubleClick(screen.getByTestId('req-text-cell-REQ-TRIP-01'));
    const editor = await screen.findByTestId('req-doc-editor');
    fireEvent.change(editor, { target: { value: newBody } });
    fireEvent.keyDown(editor, { key: 'Enter', metaKey: true });

    await waitFor(() => {
      // 1. the typed command was issued with the row id + new text
      const computes = commandCalls(fetchMock, 'sysml.workspace.edit_requirement_doc');
      expect(computes.length).toBe(1);
      expect(computes[0].params).toEqual({ element_id: 'e-1', new_text: newBody });
      // 4. the buffer was guard-spliced and is dirty (editor owns save)
      const file = useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!;
      expect(file.source).toContain(`doc /* ${newBody} */`);
      expect(file.source).not.toContain('40 ms');
      expect(file.dirty).toBe(true);
      // 5. the reparse sync carried the FULL spliced text
      const syncs = commandCalls(fetchMock, 'sysml.load_source');
      expect(syncs.length).toBe(1);
      expect(syncs[0].params.uri).toBe(TRIP_URI);
      expect(syncs[0].params.source).toBe(file.source);
    });
    // 6. quiet confirmation: pending badge gone, no failure state
    await waitFor(() => {
      expect(screen.queryByTestId('req-cell-pending')).toBeNull();
    });
    expect(screen.queryByTestId('req-cell-failed')).toBeNull();
    // editor-owns-save stays visible
    expect(screen.getByTestId('requirements-unsaved-edits').textContent).toContain(
      '1 unsaved edit',
    );
  });

  it('fails loudly on a stale guard and writes nothing', async () => {
    // Buffer diverged from what the service computed against.
    seedTripBuffer(TRIP_SOURCE.replace('40 ms', '99 ms'));
    const staleSource = useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!.source;
    const { fetchMock } = await mountOnGrid({ fieldEdit: docEditFixture('new text') });

    fireEvent.doubleClick(screen.getByTestId('req-text-cell-REQ-TRIP-01'));
    const editor = await screen.findByTestId('req-doc-editor');
    fireEvent.change(editor, { target: { value: 'new text' } });
    fireEvent.keyDown(editor, { key: 'Enter', metaKey: true });

    const failedBadge = await screen.findByTestId('req-cell-failed');
    expect(failedBadge.getAttribute('title')).toContain('stale buffer');
    expect(screen.getByTestId('req-cell-error').textContent).toContain('stale buffer');
    // Nothing written: buffer byte-identical, no reparse sync issued.
    const file = useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!;
    expect(file.source).toBe(staleSource);
    expect(file.dirty).toBe(false);
    expect(commandCalls(fetchMock, 'sysml.load_source').length).toBe(0);
  });

  it('maturity routes to add_requirement_maturity when the row has none', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountOnGrid({ fieldEdit: maturityInsertFixture() });
    fireEvent.doubleClick(screen.getByTestId('req-maturity-cell-REQ-USER-01'));
    const select = await screen.findByTestId('req-maturity-select');
    fireEvent.change(select, { target: { value: 'tbd' } });
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_requirement_maturity');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ element_id: 'e-4', status: 'tbd' });
    });
    // edit_ variant was NOT used (the row's maturity is null).
    expect(commandCalls(fetchMock, 'sysml.workspace.edit_requirement_maturity').length).toBe(0);
  });

  it('guided create posts create_requirement from the + add row', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountOnGrid({ fieldEdit: maturityInsertFixture() });
    fireEvent.click(screen.getByTestId('req-add-p-trip'));
    fireEvent.change(await screen.findByTestId('create-req-name'), {
      target: { value: 'ResetTime' },
    });
    fireEvent.change(screen.getByTestId('create-req-short-name'), {
      target: { value: 'REQ-TRIP-02' },
    });
    fireEvent.click(screen.getByTestId('create-req-submit'));
    await waitFor(() => {
      const creates = commandCalls(fetchMock, 'sysml.workspace.create_requirement');
      expect(creates.length).toBe(1);
      expect(creates[0].params).toEqual({
        parent_id: 'p-trip',
        name: 'ResetTime',
        short_name: 'REQ-TRIP-02',
      });
    });
  });

  it('rejects an invalid create name client-side without posting', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountOnGrid();
    fireEvent.click(screen.getByTestId('req-add-p-trip'));
    fireEvent.change(await screen.findByTestId('create-req-name'), {
      target: { value: 'has space' },
    });
    fireEvent.click(screen.getByTestId('create-req-submit'));
    expect(screen.getByTestId('req-cell-error').textContent).toContain('not a valid identifier');
    expect(commandCalls(fetchMock, 'sysml.workspace.create_requirement').length).toBe(0);
  });
});

// ── R5 link writing (v2 §7.6 — rail pickers) ─────────────────────────

const PART_URI = '/ws/demo/BreakerImpl.sysml';
const PART_SOURCE = ['package BreakerImpl {', '\tpart breaker : Breaker;', '}', ''].join('\n');

/** The cross-file satisfy insertion `add_satisfy_link` would compute:
 *  the part's `;` declaration grows a braced body in the PART's file. */
function satisfyLinkFixture(): FieldEditComputed {
  return {
    uri: PART_URI,
    element_id: 'e-p9',
    field: 'add_satisfy_link',
    edit: {
      line_start: 1,
      col_start: 23,
      line_end: 1,
      col_end: 24,
      new_text: ' {\n\t\tsatisfy TripUnit::TripTime;\n\t}',
      expected_old_text: ';',
    },
  };
}

const LINK_CANDIDATES = [
  { id: 'e-p9', name: 'breaker', qualified_name: 'BreakerImpl::breaker', kind: 'PartUsage' },
  { id: 'e-p1', name: 'sensing_coil', qualified_name: 'Sensing::sensing_coil', kind: 'PartUsage' },
];

describe('RequirementsWorkflow — link writing (v2 §7.6)', () => {
  async function mountRail(opts: HarnessOpts) {
    const { fetchMock } = await mountOnGrid(opts);
    fireEvent.click(screen.getByTestId('req-row-REQ-TRIP-01'));
    const ctx = getRailContext(REQUIREMENTS_LINKS_CONTEXT_ID);
    if (!ctx) throw new Error('rail context not registered');
    render(withQueryClient(ctx.render()));
    await waitFor(() => {
      expect(screen.getByTestId('requirements-links-body')).toBeDefined();
    });
    return { fetchMock };
  }

  it('adds a satisfying part through the picker: cross-file compute → splice → sync', async () => {
    useWorkspaceStore.setState({
      loadedFiles: new Map([
        [TRIP_URI, { uri: TRIP_URI, source: TRIP_SOURCE, dirty: false, tree: [] }],
        [PART_URI, { uri: PART_URI, source: PART_SOURCE, dirty: false, tree: [] }],
      ]),
    });
    const { fetchMock } = await mountRail({
      fieldEdit: satisfyLinkFixture(),
      linkCandidates: LINK_CANDIDATES,
    });

    fireEvent.click(screen.getByTestId('req-add-link-link_satisfy'));
    const input = await screen.findByTestId('add-link-input-satisfying-part');
    fireEvent.change(input, { target: { value: 'breaker' } });
    fireEvent.click(screen.getByTestId('add-link-submit-satisfying-part'));

    await waitFor(() => {
      // Typed command with both element ids.
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_satisfy_link');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ requirement_id: 'e-1', subject_id: 'e-p9' });
      // The splice landed in the PICKED PART's file, not the requirement's.
      const file = useWorkspaceStore.getState().loadedFiles.get(PART_URI)!;
      expect(file.source).toContain('satisfy TripUnit::TripTime;');
      expect(file.dirty).toBe(true);
      const syncs = commandCalls(fetchMock, 'sysml.load_source');
      expect(syncs.length).toBe(1);
      expect(syncs[0].params.uri).toBe(PART_URI);
      expect(syncs[0].params.source).toBe(file.source);
    });
    // The requirement's own file was never touched.
    expect(useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!.dirty).toBe(false);

    // Candidate sourcing: kind-filtered AND user-authored (design §7.6).
    const queries = commandCalls(fetchMock, 'sysml.query');
    expect(queries.length).toBe(1);
    const filters = queries[0].params.spec.filter.filters as Array<Record<string, unknown>>;
    expect(filters).toContainEqual({ type: 'kind', kinds: ['PartUsage'] });
    expect(filters).toContainEqual({ type: 'user_authored' });
  });

  it('excludes self and already-linked targets; derived-to swaps the roles', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRail({
      fieldEdit: maturityInsertFixture(),
      linkCandidates: [
        { id: 'e-1', name: 'TripTime', qualified_name: null, kind: 'RequirementUsage' },
        { id: 'e-2', name: 'SensThreshold', qualified_name: null, kind: 'RequirementUsage' },
        { id: 'e-3', name: 'SensAccuracy', qualified_name: null, kind: 'RequirementUsage' },
      ],
    });

    // Row e-1 already derives-to e-2; self (e-1) is never a candidate.
    fireEvent.click(screen.getByTestId('req-add-link-link_derive_to'));
    const input = await screen.findByTestId('add-link-input-derived-requirement');
    fireEvent.change(input, { target: { value: 'Sens' } });
    const suggestions = await screen.findByTestId('add-link-input-derived-requirement-suggestions');
    expect(suggestions.textContent).toContain('SensAccuracy');
    expect(suggestions.textContent).not.toContain('SensThreshold');
    expect(suggestions.textContent).not.toContain('TripTime');

    fireEvent.change(input, { target: { value: 'SensAccuracy' } });
    fireEvent.click(screen.getByTestId('add-link-submit-derived-requirement'));
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_derive_link');
      expect(adds.length).toBe(1);
      // derived-to: the PICKED requirement is the derived end, the row the original.
      expect(adds[0].params).toEqual({ requirement_id: 'e-3', original_id: 'e-1' });
    });
  });

  it('a free-form value that matches no candidate cannot commit', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRail({ linkCandidates: LINK_CANDIDATES });
    fireEvent.click(screen.getByTestId('req-add-link-link_satisfy'));
    const input = await screen.findByTestId('add-link-input-satisfying-part');
    fireEvent.change(input, { target: { value: 'noSuchPart' } });
    const submit = screen.getByTestId('add-link-submit-satisfying-part');
    expect((submit as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(submit);
    expect(commandCalls(fetchMock, 'sysml.workspace.add_satisfy_link').length).toBe(0);
  });

  it('a failed link add surfaces the service error on the affordance row', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRail({
      fieldEditError: "'VC-TRIP-01' already verifies 'TripTime' — the link exists",
      linkCandidates: [
        { id: 'e-v1', name: 'VC-TRIP-01', qualified_name: null, kind: 'VerificationCaseUsage' },
        { id: 'e-v2', name: 'VC-TRIP-02', qualified_name: null, kind: 'VerificationCaseUsage' },
      ],
    });
    fireEvent.click(screen.getByTestId('req-add-link-link_verify'));
    const input = await screen.findByTestId('add-link-input-verifying-case');
    fireEvent.change(input, { target: { value: 'VC-TRIP-02' } });
    fireEvent.click(screen.getByTestId('add-link-submit-verifying-case'));
    await screen.findByTestId('req-cell-failed');
    expect(screen.getByTestId('req-cell-error').textContent).toContain('already verifies');
    expect(commandCalls(fetchMock, 'sysml.load_source').length).toBe(0);
  });
});

describe('RequirementsWorkflow — refine link writing (v2 §7.6, added surface)', () => {
  async function mountRail(opts: HarnessOpts) {
    const { fetchMock } = await mountOnGrid(opts);
    fireEvent.click(screen.getByTestId('req-row-REQ-TRIP-01'));
    const ctx = getRailContext(REQUIREMENTS_LINKS_CONTEXT_ID);
    if (!ctx) throw new Error('rail context not registered');
    render(withQueryClient(ctx.render()));
    await waitFor(() => {
      expect(screen.getByTestId('requirements-links-body')).toBeDefined();
    });
    return { fetchMock };
  }

  it('adds a refined requirement through the (formerly read-only) refines group', async () => {
    seedTripBuffer();
    const { fetchMock } = await mountRail({
      fieldEdit: {
        uri: TRIP_URI,
        element_id: 'e-1',
        field: 'add_refine_link',
        edit: {
          line_start: 4,
          col_start: 0,
          line_end: 4,
          col_end: 1,
          new_text: '\tprivate import ModelingMetadata::*;\n\n\tdependency from TripTime to SensThreshold {\n\t\t@Refinement;\n\t}\n}',
          expected_old_text: '}',
        },
      },
      linkCandidates: [
        { id: 'e-1', name: 'TripTime', qualified_name: null, kind: 'RequirementUsage' },
        { id: 'e-2', name: 'SensThreshold', qualified_name: null, kind: 'RequirementUsage' },
        { id: 'e-3', name: 'SensAccuracy', qualified_name: null, kind: 'RequirementUsage' },
      ],
    });

    fireEvent.click(screen.getByTestId('req-add-link-link_refine'));
    const input = await screen.findByTestId('add-link-input-refined-requirement');
    // 'Sens' matches both non-excluded requirements; self (e-1 = TripTime)
    // is never offered — typing 'Trip' would yield an empty dropdown.
    fireEvent.change(input, { target: { value: 'Sens' } });
    const suggestions = await screen.findByTestId('add-link-input-refined-requirement-suggestions');
    expect(suggestions.textContent).toContain('SensThreshold');
    expect(suggestions.textContent).toContain('SensAccuracy');
    expect(suggestions.textContent).not.toContain('TripTime');

    fireEvent.change(input, { target: { value: 'SensThreshold' } });
    fireEvent.click(screen.getByTestId('add-link-submit-refined-requirement'));
    await waitFor(() => {
      const adds = commandCalls(fetchMock, 'sysml.workspace.add_refine_link');
      expect(adds.length).toBe(1);
      expect(adds[0].params).toEqual({ requirement_id: 'e-1', refined_id: 'e-2' });
      const file = useWorkspaceStore.getState().loadedFiles.get(TRIP_URI)!;
      expect(file.source).toContain('@Refinement;');
      expect(file.source).toContain('private import ModelingMetadata::*;');
    });
    // Candidate query filtered to requirement kinds AND user-authored.
    const q = commandCalls(fetchMock, 'sysml.query')[0];
    const filters = q.params.spec.filter.filters as Array<Record<string, unknown>>;
    expect(filters).toContainEqual({ type: 'kind', kinds: ['RequirementDefinition', 'RequirementUsage'] });
    expect(filters).toContainEqual({ type: 'user_authored' });
  });
});
