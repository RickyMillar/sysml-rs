/**
 * App — top-level React entry with workflow-router.
 *
 * Structure (ninebar Phase 1):
 *   Providers (react-query)
 *     RouterProvider
 *       LayoutGate
 *         SessionStreamProvider   (the ONE live-stream owner, F15)
 *         AppShell | AppLayout    (ninebar flag picks the shell)
 *           RunWorkflow / VerifyWorkflow / ... (per-route component)
 *
 * Shared services (AutoLoadWorkspace, test hooks, route sync) live in
 * `app/appServices.tsx` and are mounted by whichever shell renders, so
 * they mount once regardless of active workflow or shell.
 */

import {
  createBrowserRouter,
  Navigate,
  Outlet,
  RouterProvider,
  useLocation,
} from 'react-router-dom';
import { Providers } from '@/app/providers';
import { AppShell } from '@/app/AppShell';
import {
  AutoLoadWorkspace,
  useActiveToolRouteSync,
  useTestHooks,
  WorkspaceBar,
} from '@/app/appServices';
import { isFlagEnabled } from '@/featureFlags';
import { WorkspaceLoadErrorBanner } from '@/features/workspace/WorkspaceLoadErrorBanner';
import { SelectedViewRenderer } from '@/features/views/SelectedViewRenderer';
import { SessionStreamProvider } from '@/features/sessions/SessionStreamProvider';
import { OverridePopoverHost } from '@/features/sessions/OverridePopover';
import { CommandPalette, isDevCmdKEnabled } from '@/features/command-palette';
import { WorkflowSwitcher } from '@/workflows/ui/WorkflowSwitcher';
import { UtilityDrawer } from '@/features/utilities/UtilityDrawer';
import { useInstallSessionEventBus } from '@/engine/useInstallSessionEventBus';
import {
  BrowseWorkflow,
  RequirementsWorkflow,
  RunWorkflow,
  VerifyWorkflow,
  AnalyzeWorkflow,
  AnalyzeIndexRedirect,
  SweepWorkflow,
  MonteCarloWorkflow,
  TradeStudyWorkflow,
  SensitivityWorkflow,
  CompareWorkflow,
} from '@/workflows';

// ── View Switching ───────────────────────────────────────────────────
//
// Bucket 5: the legacy `__sysmlSwitchView` global hook went with
// `switchView` itself. Diagrams are picked by ElementId via ViewsPanel
// now; tests can drive that surface directly via `setSelectedViewId`.
function useViewSwitching() {}

// ── Legacy layout ────────────────────────────────────────────────────

function AppLayout() {
  useViewSwitching();
  useTestHooks();
  useActiveToolRouteSync();
  // BP5: installs the R1.5 `SessionEventBus` (breakpoint-hit / verdict-flip
  // / etc.) against the live react-query session cache. Was never wired
  // in before — see the doc comment on the hook for the discovered gap.
  useInstallSessionEventBus();

  return (
    <>
      <AutoLoadWorkspace />
      <SelectedViewRenderer />
      <div
        data-testid="app-shell"
        className="flex flex-col h-screen w-screen overflow-hidden"
      >
        <WorkflowSwitcher />
        <WorkspaceBar />
        <WorkspaceLoadErrorBanner />
        <UtilityDrawer />
        <div className="flex-1 overflow-hidden">
          <Outlet />
        </div>
      </div>
      {/* Dev-only: Cmd-K / Ctrl-K opens a full catalogue of backend
          commands. Enable with `VITE_DEV_CMDK=1 npm run dev`. Does
          nothing when the flag is unset, so production builds are
          unaffected. (First-class in the ninebar shell.) */}
      {isDevCmdKEnabled() && <CommandPalette />}
    </>
  );
}

// ── Layout gate ──────────────────────────────────────────────────────

/**
 * Picks the shell: `AppShell` (ninebar, `?flag=ninebar`) or the legacy
 * `AppLayout`. The gate also mounts the app-level session stream so
 * live state survives route changes under BOTH shells — exactly one
 * `SessionStreamProvider` exists in the tree (dev-asserted inside
 * `useSessionStream`).
 */
function LayoutGate() {
  return (
    <>
      <SessionStreamProvider />
      {/* One app-level override surface for every call site (variables
          pane, tree, plots) — ninebar Phase 3, audits F9/F15. */}
      <OverridePopoverHost />
      {isFlagEnabled('ninebar') ? <AppShell /> : <AppLayout />}
    </>
  );
}

// ── Router ───────────────────────────────────────────────────────────

/**
 * Preserves the current search string when redirecting so deep-links
 * like `/?workspace=/path` survive the hop to `/run`.
 */
function RedirectToRun() {
  const { search } = useLocation();
  return <Navigate to={{ pathname: '/run', search }} replace />;
}

/**
 * Legacy `/compare` deep-links land on the Phase 6 home under the
 * Simulate door. Search preserved (e.g. `?workspace=…`).
 */
function RedirectToCompare() {
  const { search } = useLocation();
  return <Navigate to={{ pathname: '/run/compare', search }} replace />;
}

const router = createBrowserRouter([
  {
    path: '/',
    element: <LayoutGate />,
    children: [
      { index: true, element: <RedirectToRun /> },
      // ninebar Phase 1.5 — Browse floor. Route-config is shared by
      // both shells (`LayoutGate` only picks which shell wraps the
      // Outlet); under the legacy `AppLayout` this still renders — the
      // primary reading surface + trace matrix work, but the left-rail
      // tree portal (`src/app/slots.tsx`) finds no target there and
      // silently renders nothing, since only `AppShell` mounts
      // `<LeftRailSlot/>`.
      { path: 'browse', element: <BrowseWorkflow /> },
      { path: 'run', element: <RunWorkflow /> },
      { path: 'verify', element: <VerifyWorkflow /> },
      // ninebar Phase 7.5 — Requirements workbench (same shell caveat
      // as /browse: the rail/strip portals only exist under AppShell).
      { path: 'requirements', element: <RequirementsWorkflow /> },
      {
        path: 'analyze',
        element: <AnalyzeWorkflow />,
        children: [
          { index: true, element: <AnalyzeIndexRedirect /> },
          { path: 'sweep', element: <SweepWorkflow /> },
          { path: 'montecarlo', element: <MonteCarloWorkflow /> },
          { path: 'trade-study', element: <TradeStudyWorkflow /> },
          { path: 'sensitivity', element: <SensitivityWorkflow /> },
        ],
      },
      // ninebar Phase 6 — Compare demoted to a Simulate mode
      // (workbench-suite ruling, plan §1.5): canonical route is
      // /run/compare; the old top-level /compare deep-link redirects
      // with its search string preserved.
      { path: 'run/compare', element: <CompareWorkflow /> },
      { path: 'compare', element: <RedirectToCompare /> },
      // Anything unknown lands on /run so reload-to-garbage-URL is safe.
      { path: '*', element: <RedirectToRun /> },
    ],
  },
]);

// ── App ──────────────────────────────────────────────────────────────

export function App() {
  return (
    <Providers>
      <RouterProvider router={router} />
    </Providers>
  );
}
