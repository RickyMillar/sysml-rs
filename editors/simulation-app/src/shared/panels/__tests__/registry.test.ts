/**
 * Tests for the panel registry filtering behaviour (R1.6).
 *
 * Focus: the registry encodes the same visibility rules that the
 * ResultsWorkbench uses for task tabs. These tests pin the current
 * behaviour so any future refactor can verify nothing regressed.
 */
import { describe, it, expect } from 'vitest';
import { panelRegistry, findPanel } from '../registry';
import type { PanelSessionState } from '../types';
import type { ModelCapabilities } from '../../../hooks/useModelCapabilities';

function makeCaps(overrides: Partial<ModelCapabilities> = {}): ModelCapabilities {
  return {
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
    sessionType: 'sm',
    isMultiFile: false,
    ...overrides,
  };
}

function makeSession(overrides: Partial<PanelSessionState> = {}): PanelSessionState {
  return {
    phase: 'idle',
    activeSessionId: null,
    hasStreamingData: false,
    ...overrides,
  };
}

function applicableIds(caps: ModelCapabilities, session: PanelSessionState): string[] {
  // Workbench-focused helper — sidebar / detail-positioned panels (e.g.
  // Breakpoints) are excluded so the tests assert only on the Run
  // ResultsWorkbench surface.
  return panelRegistry
    .filter((p) => p.defaultPosition === 'workbench')
    .filter((p) => p.applicableWhen(caps, session))
    .map((p) => p.id);
}

describe('panelRegistry', () => {
  it('registers all canonical panels in workbench order then utility panels', () => {
    // Workbench panels first, then utility/detail panels mounted by their
    // host surfaces.
    expect(panelRegistry.map((p) => p.id)).toEqual([
      'plots',
      'stateTimeline',
      'constraints',
      'equations',
      'streams',
      'kpi',
      // Utility-position panels appended at the tail; they do not
      // participate in the ResultsWorkbench.
      'breakpoints',
      'variables',
      'archive',
      'diagnostics',
      // R6.2 — Traceability matrix panel (detail position).
      'traceabilityMatrix',
      // R7.1 — Causal trace panel (detail position).
      'causalTrace',
      // S4.T4 — Source panel (utility position).
      'source',
      // Phase 5 — Authored-views panel (utility position).
      'views',
      // Phase 7 — Integrations panel (MCP / REST / LSP wiring).
      'integrations',
      // Phase 8 — Debug drawer (utility position; surfaced behind
      // VITE_DEBUG_DRAWER=1 in the shell).
      'debug',
    ]);
  });

  it('every descriptor has the invariants consumers rely on', () => {
    for (const panel of panelRegistry) {
      expect(panel.id).toMatch(/^[a-zA-Z]+$/);
      expect(panel.title.length).toBeGreaterThan(0);
      expect(panel.icon.length).toBeGreaterThan(0);
      expect(panel.accentColor.length).toBeGreaterThan(0);
      // Panels are either workbench-rendered or hosted by detail/utility
      // surfaces. All are valid positions.
      expect(['workbench', 'detail', 'utility']).toContain(panel.defaultPosition);
      expect(typeof panel.applicableWhen).toBe('function');
      expect(typeof panel.render).toBe('function');
    }
  });

  it('findPanel resolves ids case-sensitively and returns undefined otherwise', () => {
    expect(findPanel('plots')?.title).toBe('Plots');
    expect(findPanel('unknown')).toBeUndefined();
    // Case-sensitive: tab/panel wiring relies on stable ids.
    expect(findPanel('Plots')).toBeUndefined();
  });

  describe('applicableWhen — capability gating', () => {
    it('empty model: only always-on panels are active (equations, kpi)', () => {
      expect(applicableIds(makeCaps(), makeSession())).toEqual([
        'equations',
        'kpi',
      ]);
    });

    it('hasODEs unlocks the plots panel', () => {
      expect(applicableIds(makeCaps({ hasODEs: true }), makeSession())).toContain(
        'plots',
      );
    });

    it('hasStateMachines unlocks the state timeline panel', () => {
      expect(
        applicableIds(makeCaps({ hasStateMachines: true }), makeSession()),
      ).toContain('stateTimeline');
    });

    it('hasConstraints unlocks the constraints panel', () => {
      expect(
        applicableIds(makeCaps({ hasConstraints: true }), makeSession()),
      ).toContain('constraints');
    });

    it('verification, analysis cases, and flows no longer create placeholder workbench panels', () => {
      expect(
        applicableIds(
          makeCaps({ hasVerification: true, hasAnalysisCases: true, hasFlows: true }),
          makeSession(),
        ),
      ).toEqual(['equations', 'kpi']);
    });
  });

  describe('applicableWhen — session-state gating', () => {
    it('streams stays inactive until the backend provides streaming data', () => {
      // Bare capability flags are not enough.
      expect(
        applicableIds(
          makeCaps({ hasActionFlows: true }),
          makeSession(),
        ),
      ).not.toContain('streams');
    });

    it('streams activates when hasStreamingData is true', () => {
      expect(
        applicableIds(
          makeCaps(),
          makeSession({ hasStreamingData: true }),
        ),
      ).toContain('streams');
    });
  });

  it('fully-featured model: every workbench panel is applicable when a session is active', () => {
    const caps = makeCaps({
      hasStateMachines: true,
      hasODEs: true,
      hasConstraints: true,
      hasVerification: true,
      hasAnalysisCases: true,
      hasFlows: true,
    });
    const session = makeSession({ hasStreamingData: true, phase: 'running' });
    const ids = applicableIds(caps, session);
    expect(ids).toEqual(
      panelRegistry.filter((p) => p.defaultPosition === 'workbench').map((p) => p.id),
    );
  });

  it('Phase B7: variables panel is always hidden (its jobs moved into SessionTreeV2)', () => {
    // Prior to Phase B the pane rendered as a sidebar fixture whenever a
    // session wasn't idle. After the session-tree rearchitecture, its
    // jobs (list / filter / pin / live values) live per-node in the
    // tree — the descriptor is kept for consumers that import it but it
    // is not surfaced by the utility drawer.
    const caps = makeCaps();
    const variables = panelRegistry.find((p) => p.id === 'variables');
    expect(variables).toBeDefined();
    expect(variables!.applicableWhen(caps, makeSession({ phase: 'idle' }))).toBe(false);
    expect(variables!.applicableWhen(caps, makeSession({ phase: 'running' }))).toBe(false);
    expect(variables!.applicableWhen(caps, makeSession({ phase: 'paused' }))).toBe(false);
    expect(variables!.applicableWhen(caps, makeSession({ phase: 'completed' }))).toBe(false);
  });

  it('iteration order is stable — active panels keep their relative order', () => {
    // Partial enablement should never reorder panels.
    const caps = makeCaps({ hasStateMachines: true, hasFlows: true });
    expect(applicableIds(caps, makeSession())).toEqual([
      'stateTimeline',
      'equations',
      'kpi',
    ]);
  });
});
