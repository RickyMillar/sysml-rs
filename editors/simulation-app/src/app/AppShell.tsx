/**
 * AppShell — the ninebar five-slot layout (Phase 1).
 *
 * Flag-gated sibling of `AppLayout` (see `LayoutGate` in App.tsx);
 * enable with `?flag=ninebar`. `AppLayout` stays the default until the
 * Phase 1.5 AND Phase 3 gates both pass (plan v1.3, audit F17).
 *
 * The spatial budget is a hard contract (plan §0): these five always-on
 * slots and NOTHING else. Everything further is an overlay — right-rail
 * context, modal, popover, or palette. No sixth resident panel, ever.
 *
 *   frame          40px top strip: mark · workflow nav · session chips ·
 *                  run controls (Phase 1 tasks fill this in)
 *   left-rail      collapsible navigator (300px / 0)
 *   primary-outlet the ONE primary surface (route outlet). Legacy
 *                  workflow bodies render here as-is until Phase 3
 *                  re-composes them ("interim primary-only mode", F17).
 *   right-rail     slide-in context host (`src/app/rail/RightRail.tsx`)
 *                  — off by default, pinnable, max two stacked contexts
 *                  (audit F3); see `src/app/rail/railStore.ts`
 *   bottom-strip   minimal strip (status/results home per workflow)
 *
 * Testid contract (Playwright): `app-shell` on the root plus one testid
 * per slot — `frame` / `left-rail` / `primary-outlet` / `right-rail` /
 * `bottom-strip`. Slots are always MOUNTED (collapsed ones carry
 * `data-state="closed"` and zero width/height) so specs can assert the
 * contract without racing visibility.
 *
 * Shared app services (window.__* test hooks, session event bus,
 * activeTool route sync, workspace autoload) mount here exactly as they
 * do in `AppLayout` — 61 data-hook assertions and the physics-mirror
 * suite depend on them being present regardless of which shell renders.
 * The live session stream is NOT mounted here — `SessionStreamProvider`
 * owns it at the gate, above both shells.
 */

import { useState } from 'react';
import { Outlet } from 'react-router-dom';
import { useTheme } from '@/app/useTheme';
import { WorkflowSwitcher } from '@/workflows/ui/WorkflowSwitcher';
import { WorkspaceLoadErrorBanner } from '@/features/workspace/WorkspaceLoadErrorBanner';
import { SelectedViewRenderer } from '@/features/views/SelectedViewRenderer';
import { CommandPalette } from '@/features/command-palette';
import { useInstallSessionEventBus } from '@/engine/useInstallSessionEventBus';
import { RightRail } from '@/app/rail/RightRail';
import { ModalHost } from '@/shared/overlays/ModalHost';
import { SessionSwitcherChip } from '@/app/frame/SessionSwitcherChip';
import { FrameStatus } from '@/app/frame/FrameStatus';
import { PhasePill } from '@/app/frame/PhasePill';
import { RunControls } from '@/app/frame/RunControls';
import { TrailCrumb } from '@/app/frame/TrailCrumb';
import { QuotaChip } from '@/app/frame/QuotaChip';
import { ReadinessChip } from '@/app/frame/ReadinessChip';
import { BottomStripSlot, LeftRailSlot, useSlotPresenceStore } from '@/app/slots';
import {
  AutoLoadWorkspace,
  useActiveToolRouteSync,
  useTestHooks,
  WorkspaceBar,
} from '@/app/appServices';

export function AppShell() {
  useTestHooks();
  useActiveToolRouteSync();
  useInstallSessionEventBus();

  const [leftOpen, setLeftOpen] = useState(true);
  const { theme, toggle: toggleTheme } = useTheme();

  // ninebar Phase 1.5 (`src/app/slots.tsx`): a workflow that portals
  // left-rail / bottom-strip content announces its presence here so
  // the shell can pick the left-rail fallback and the strip's
  // open/closed state without knowing what was portaled.
  const leftRailContentActive = useSlotPresenceStore((s) => s.leftRailActive);
  const bottomStripActive = useSlotPresenceStore((s) => s.bottomStripActive);
  const bottomStripCollapsed = useSlotPresenceStore((s) => s.bottomStripCollapsed);

  return (
    <>
      <AutoLoadWorkspace />
      <SelectedViewRenderer />
      <div
        data-testid="app-shell"
        className="flex flex-col h-screen w-screen overflow-hidden"
        style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
      >
        {/* ── Slot 1: frame ──
            Calm chrome: `--surface-chrome` is the dedicated chrome tier
            (distinct from `--surface-panel`, used by rails) + a single
            hairline border. No amber here — the frame is never a
            selection surface. */}
        <header
          data-testid="frame"
          className="flex items-center gap-3 shrink-0 px-3"
          style={{
            height: 'var(--frame-top-height)',
            background: 'var(--surface-chrome)',
            borderBottom: '1px solid var(--border-default)',
          }}
        >
          <button
            type="button"
            data-testid="left-rail-toggle"
            onClick={() => setLeftOpen((v) => !v)}
            aria-label={leftOpen ? 'Collapse navigator' : 'Expand navigator'}
            className="material-symbols-outlined"
            style={{
              fontSize: 18,
              color: 'var(--text-secondary)',
              background: 'none',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              cursor: 'pointer',
              padding: 4,
            }}
          >
            {leftOpen ? 'left_panel_close' : 'left_panel_open'}
          </button>
          {/* The wordmark — text only. The <Ninebar/> meter glyph is
              reserved for live/pending measures (see its doc comment); it
              is never a decorative logo, so it does not appear here.
              Clicking the wordmark toggles light / dark (useTheme) — the
              theme is a single data-theme attribute the whole token system
              follows. */}
          <button
            type="button"
            data-testid="theme-toggle"
            onClick={toggleTheme}
            title={`Switch to ${theme === 'dark' ? 'light' : 'dark'} theme`}
            aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} theme`}
            style={{
              fontSize: 'var(--text-sm)',
              fontWeight: 600,
              color: 'var(--text-primary)',
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
              background: 'none',
              border: 'none',
              padding: 0,
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            ninebar
          </button>
          <div
            aria-hidden
            style={{ width: 1, alignSelf: 'stretch', margin: '10px 0', background: 'var(--border-default)' }}
          />
          {/* Workflow nav — ninebar variant is text-only + inline (no
              nested sub-bar), per ruling C. */}
          <WorkflowSwitcher variant="ninebar" />

          {/* Session identity + status cluster (frame chips, audit
              F2/F7/F10): active-session switcher + the live 6-phase
              pill. See src/app/frame/*. */}
          <SessionSwitcherChip />
          <PhasePill />

          {/* Run-control cluster (Phase 1 task list) — mounts the step
              loop here so it survives route changes; see RunControls.tsx
              doc comment for the frame-vs-legacy mount split. */}
          <RunControls />

          <div className="flex-1" />

          {/* Investigation trail + quota sit on the quiet trailing edge —
              informational, not primary navigation. The quiet mono model
              status (ruling C) anchors the far edge, as in the demo. */}
          <TrailCrumb />
          <ReadinessChip />
          <QuotaChip />
          <FrameStatus />
        </header>

        <WorkspaceLoadErrorBanner />

        {/* ── Middle band: left rail · primary · right rail ── */}
        <div className="flex flex-1 overflow-hidden">
          {/* ── Slot 2: left rail ──
              `--surface-panel` reads as a distinct rail section against the
              `--surface-canvas` primary surface; the "Workspace" row is a
              section header only — `WorkspaceBar` internals are shared with
              the legacy shell and are not restyled here. */}
          <aside
            data-testid="left-rail"
            data-state={leftOpen ? 'open' : 'closed'}
            className="shrink-0 flex flex-col overflow-hidden"
            style={{
              width: leftOpen ? 'var(--rail-left-width)' : 0,
              background: 'var(--surface-panel)',
              borderRight: leftOpen ? '1px solid var(--border-default)' : 'none',
              transition: 'width var(--motion-panel) ease',
            }}
          >
            {/* Interim: the workspace loader lives here (until it moves
                to a modal, plan §1) ONLY when no workflow has portaled
                its own left-rail content (`src/app/slots.tsx`) — e.g.
                still true on `/run` today. Browse (Phase 1.5) is the
                first workflow to replace this with its package+element
                tree. */}
            {!leftRailContentActive && (
              <>
                <div
                  className="flex items-center shrink-0 px-3"
                  style={{
                    height: 'var(--row-default)',
                    fontSize: 'var(--text-sm)',
                    fontWeight: 500,
                    color: 'var(--text-secondary)',
                    letterSpacing: '0.03em',
                    textTransform: 'uppercase',
                    borderBottom: '1px solid var(--border-default)',
                  }}
                >
                  Workspace
                </div>
                <WorkspaceBar />
              </>
            )}
            <LeftRailSlot />
          </aside>

          {/* ── Slot 3: primary surface ── */}
          <main
            data-testid="primary-outlet"
            className="flex-1 overflow-hidden flex flex-col"
          >
            <Outlet />
          </main>

          {/* ── Slot 4: right rail — slide-in context host, off by
                 default, pinnable, max two stacked contexts (audit F3).
                 See src/app/rail/RightRail.tsx. ── */}
          <RightRail />
        </div>

        {/* ── Slot 5: bottom strip ── */}
        <footer
          data-testid="bottom-strip"
          data-state={bottomStripActive ? 'open' : 'closed'}
          data-collapsed={bottomStripActive && bottomStripCollapsed ? 'true' : undefined}
          className="shrink-0 overflow-hidden flex flex-col"
          style={
            bottomStripActive
              ? {
                  // Ghost mode (plan §0): no min-height floor while the
                  // workflow flags its strip content collapsed — the
                  // transport row keeps its intrinsic height, the chart
                  // area reserves nothing.
                  minHeight: bottomStripCollapsed ? 0 : 'var(--strip-min-height)',
                  maxHeight: 'var(--strip-max-height)',
                  height: 'auto',
                  borderTop: '1px solid var(--border-default)',
                  background: 'var(--surface-panel)',
                }
              : { height: 0 }
          }
        >
          <BottomStripSlot />
        </footer>
      </div>

      {/* Cmd-K is first-class in the ninebar shell — no dev-flag gate.
          It is also the stress-test instrument: every service command
          exercisable without bespoke UI (plan §2 triage posture). */}
      <CommandPalette />

      {/* Modal host — renders whichever modal useModalStore has active
          (see src/shared/overlays/modalStore.ts). No modal is registered
          yet in Phase 1; this just mounts the host so later phases can
          register + openModal() without touching AppShell again. */}
      <ModalHost />
    </>
  );
}
