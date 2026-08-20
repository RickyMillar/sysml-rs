/**
 * sessionControllerBridge — publishes the frame's single
 * `useSessionController()` instance for the bottom-strip transport
 * cluster to consume (ninebar screenshot-comparison ruling A,
 * 2026-07-14: "Transport moves to the strip").
 *
 * WHY THIS EXISTS: `useSessionController` owns the setTimeout-based
 * step loop via component-local refs (`timerRef`/`abortRef` — see its
 * doc comment). Mounting it twice would race two independent loops
 * against the same session — the "one step-loop owner" guardrail
 * (plan §4 audit F15-adjacent). Ruling A moves the play/pause/resume/
 * step buttons out of the frame into `WaveformCard`'s header row, but
 * that row only exists while the Run workflow has portaled content
 * into the bottom-strip slot (`src/app/slots.tsx`) — it unmounts on
 * route change. The frame's `RunControls`, by contrast, is mounted
 * unconditionally in `AppShell` and never unmounts for the lifetime of
 * the ninebar shell.
 *
 * WIRING: `RunControls` keeps the sole `useSessionController()` mount
 * and publishes the returned action callbacks here via a `useEffect`
 * (see its doc comment). `TransportBar` (rendered inside
 * `WaveformCard`'s header) reads the callbacks from this store instead
 * of calling the hook itself — phase-derived enablement (`canPlay` /
 * `canPause` / …) is computed independently in both components from
 * the shared `useSessionStore` / `useWorkspaceUIStore` state, so this
 * bridge only needs to carry the four action functions, not the whole
 * derived UI state.
 *
 * Starts `null` (no controller published yet) for the brief window
 * before `RunControls`'s mount effect fires; consumers must treat a
 * `null` bridge as "not ready" (e.g. render disabled buttons) rather
 * than throwing.
 */
import { create } from 'zustand';
import type { SessionController } from '@/features/sessions/useSessionController';

interface SessionControllerBridgeState {
  /** The frame-owned controller, or `null` before `RunControls` mounts. */
  controller: SessionController | null;
  setController: (controller: SessionController) => void;
}

export const useSessionControllerBridge = create<SessionControllerBridgeState>((set) => ({
  controller: null,
  setController: (controller) => set({ controller }),
}));
