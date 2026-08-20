/**
 * Workflow route descriptors — pure data, no React imports.
 *
 * Kept separate from `index.ts` (the component barrel) so that logic
 * tests (vitest, node env) can exercise the descriptor table without
 * transitively importing the React component tree (RunWorkflow → DiagramView).
 */

export interface WorkflowDescriptor {
  /** Stable id (also used as the nav testid `tool-tab-<id>`). */
  id: string;
  /** URL path (absolute, always starts with `/`). */
  path: string;
  /** Human-readable label for the nav. */
  label: string;
  /** Material Symbols icon. */
  icon: string;
  /**
   * Optional keyboard hint label (rendered as a subtle shortcut chip in
   * the switcher). Modifier is Alt on non-mac, ⌘ on mac — resolved at
   * render time.
   */
  hotkey?: string;
  /**
   * Group — drives visual separators in the nav.
   *   'primary'  — Run / Verify / Compare
   *   'analyze'  — Analyze shell and its modes
   */
  group?: 'primary' | 'analyze';
  /** True if this entry is a Round-2+ workflow (not a legacy tool). */
  isWorkflow?: boolean;
  /** Hidden routes stay addressable but no longer appear as top-level tabs. */
  visibleInNav?: boolean;
}

/**
 * The workflows shipped today. Order matters — it drives nav order AND
 * the index-based hotkey binding (Cmd/Alt+1 → first, etc.).
 *
 * NOTE: the `id` value `session` deliberately matches the legacy
 * `ActiveTool` value used by older workflow-control call-sites.
 */
export const WORKFLOWS: WorkflowDescriptor[] = [
  {
    // ninebar Phase 1.5 — the Browse floor. First in tab order (demo:
    // "Browse · Run · Analyze · Verify · Compare" — a systems engineer
    // reads/validates a model before running it). Left un-hotkeyed
    // deliberately: Run/Verify/Analyze/Compare's existing Cmd/Alt+1-4
    // bindings are pinned by `switcher.test.ts`; renumbering them to
    // make room is a bigger, unrelated nav change than this phase's
    // scope. `group: 'primary'` (not reordered relative to Verify /
    // Analyze — that swap is a separate, out-of-scope nav change; see
    // BrowseWorkflow.tsx's doc comment) puts it first within the
    // primary tab group since array order is preserved by the
    // switcher's `.filter()`.
    id: 'browse',
    path: '/browse',
    label: 'Browse',
    icon: 'menu_book',
    group: 'primary',
    isWorkflow: true,
  },
  {
    id: 'session',
    path: '/run',
    label: 'Run',
    icon: 'science',
    hotkey: '1',
    group: 'primary',
    isWorkflow: true,
  },
  {
    id: 'verify',
    path: '/verify',
    label: 'Verify',
    icon: 'verified',
    hotkey: '2',
    group: 'primary',
    isWorkflow: true,
  },
  {
    // ninebar Phase 7.5 — the Requirements workbench (workbench-suite
    // pivot, plan §1.5). Un-hotkeyed like Browse: 1–4 stay pinned to
    // Run/Verify/Analyze/Compare by `switcher.test.ts`. Placed after
    // Verify inside the primary group — the two are lenses over one
    // graph (requirements-workbench-design.md §3) and the demo's tab
    // row seats Requirements beside Verify.
    id: 'requirements',
    path: '/requirements',
    label: 'Requirements',
    icon: 'checklist',
    group: 'primary',
    isWorkflow: true,
  },
  {
    id: 'analyze',
    path: '/analyze',
    label: 'Analyze',
    icon: 'analytics',
    hotkey: '3',
    group: 'analyze',
    isWorkflow: true,
  },
  {
    id: 'sweep',
    path: '/analyze/sweep',
    label: 'Sweep',
    icon: 'tune',
    group: 'analyze',
    isWorkflow: true,
    visibleInNav: false,
  },
  {
    id: 'montecarlo',
    path: '/analyze/montecarlo',
    label: 'Monte Carlo',
    icon: 'casino',
    group: 'analyze',
    isWorkflow: true,
    visibleInNav: false,
  },
  {
    id: 'trade-study',
    path: '/analyze/trade-study',
    label: 'Trade Study',
    icon: 'balance',
    group: 'analyze',
    isWorkflow: true,
    visibleInNav: false,
  },
  {
    // R7.4 — sensitivity workflow (Morris / Sobol). Lives inside the
    // Analyze shell alongside Sweep / Monte Carlo / Trade Study.
    id: 'sensitivity',
    path: '/analyze/sensitivity',
    label: 'Sensitivity',
    icon: 'analytics',
    group: 'analyze',
    isWorkflow: true,
    visibleInNav: false,
  },
  {
    // ninebar Phase 6 — Compare is DEMOTED from a top-level tool to a
    // Simulate mode (workbench-suite ruling 2026-07-15, plan §1.5): the
    // route moves under the Simulate door (`/run/compare`), leaves the
    // nav, and gives up its ⌘/Alt+4 binding (the freed front-door slot
    // belongs to the Phase 8 switcher redesign, not this phase). It is
    // reached via the frame session switcher's "Compare sessions…"
    // action, Cmd-K (`open.compare`), and the promote-to-Compare
    // surfaces (Analyze strip, viewer buttons, fork-and-compare).
    id: 'compare',
    path: '/run/compare',
    label: 'Compare',
    icon: 'compare',
    group: 'primary',
    isWorkflow: true,
    visibleInNav: false,
  },
];

/**
 * Map a URL pathname to its workflow id, or `null` when the path does
 * not belong to the workflow router. Longest-prefix match so that
 * `/analyze/sweep` resolves to the sweep workflow rather than the
 * (non-existent) `/analyze` parent.
 */
export function workflowIdForPath(pathname: string): string | null {
  let best: WorkflowDescriptor | null = null;
  for (const wf of WORKFLOWS) {
    if (pathname === wf.path || pathname.startsWith(`${wf.path}/`)) {
      if (!best || wf.path.length > best.path.length) best = wf;
    }
  }
  return best?.id ?? null;
}

/**
 * Map a workflow id back to its route path.
 */
export function pathForWorkflowId(id: string): string | null {
  return WORKFLOWS.find((w) => w.id === id)?.path ?? null;
}

/**
 * The nav tab to light for a pathname — longest-prefix match over the
 * VISIBLE workflows only. Differs from `workflowIdForPath` for routes
 * that live under another door without a nav tab of their own:
 * `/run/compare` resolves to the `compare` workflow but lights the
 * Simulate (`session`) tab, because Compare is a Simulate mode
 * (Phase 6 demotion), not a front door.
 */
export function navActiveIdForPath(pathname: string): string | null {
  let best: WorkflowDescriptor | null = null;
  for (const wf of WORKFLOWS) {
    if (wf.visibleInNav === false) continue;
    if (pathname === wf.path || pathname.startsWith(`${wf.path}/`)) {
      if (!best || wf.path.length > best.path.length) best = wf;
    }
  }
  return best?.id ?? null;
}
