/**
 * TransportBar — bottom-strip play/pause/resume/step cluster (ninebar
 * screenshot-comparison ruling A, 2026-07-14: "Transport moves to the
 * strip"). Companion to `RunControls.test.tsx`, which now only covers
 * the frame's own bulk-step/Stop controls.
 *
 * Verifies: phase-aware Run/Pause/Resume/Step enablement, the
 * disabled-with-reason pattern when NO MODEL IS LOADED (the only state in
 * which running is impossible — "no element picked" means run the whole
 * workspace, see `TransportBar`'s `canStartRun`), and
 * that each button calls through to the controller published on
 * `useSessionControllerBridge` — NOT a second `useSessionController()`
 * mount (this component must never call that hook; see
 * `sessionControllerBridge.ts`).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { SessionPhase } from '@/features/sessions/types';
import { useSessionControllerBridge } from '../sessionControllerBridge';

afterEach(() => {
  cleanup();
  useSessionControllerBridge.setState({ controller: null });
  mockLoadedFileCount = 0;
});

let mockPhase: SessionPhase = 'idle';
let mockActiveSessionId: string | null = null;
let mockActiveSessionTarget: string | null = null;
let mockLoadedFileCount = 0;

vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (
    selector: (s: { phase: SessionPhase; activeSessionId: string | null }) => unknown,
  ) => selector({ phase: mockPhase, activeSessionId: mockActiveSessionId }),
}));

vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (selector: (s: { activeSessionTarget: string | null }) => unknown) =>
    selector({ activeSessionTarget: mockActiveSessionTarget }),
}));

// The injector dock (mounted at the transport's trailing edge) needs a
// QueryClient + live-store plumbing out of scope here — own boundary.
vi.mock('@/store/workspace', () => ({
  useWorkspaceStore: (selector: (s: { loadedFiles: Map<string, unknown> }) => unknown) =>
    selector({
      loadedFiles: new Map(
        Array.from({ length: mockLoadedFileCount }, (_, i) => [`f${i}.sysml`, {}]),
      ),
    }),
}));

vi.mock('../InjectorDock', () => ({ InjectorDock: () => null }));

import { TransportBar } from '../TransportBar';

const NO_MODEL_TITLE = 'No model loaded — open a workspace before running';
const WHOLE_WORKSPACE_TITLE =
  'Run the whole workspace — every subsystem advances in lockstep. Pick a single element under Configure to narrow it';

function publishMockController() {
  const controllerMocks = {
    play: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    stop: vi.fn(),
    stepOnce: vi.fn(),
    fastForward: vi.fn(),
    runToBreakpoint: vi.fn(),
    startSession: vi.fn(),
  };
  useSessionControllerBridge.setState({ controller: controllerMocks });
  return controllerMocks;
}

describe('TransportBar', () => {
  it('renders disabled while the bridge has no controller published yet', () => {
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = 'Foo::bar';
    render(<TransportBar />);

    expect(screen.getByTestId('transport-run')).toBeDisabled();
  });

  it('disables Run with an explanatory title when no model is loaded', () => {
    publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockLoadedFileCount = 0;
    render(<TransportBar />);

    const run = screen.getByTestId('transport-run');
    expect(run).toBeDisabled();
    expect(run).toHaveAttribute('title', NO_MODEL_TITLE);
  });

  // The regression this component's gate used to be: with a model loaded but
  // no element picked, Run was disabled and told the user to go click a ▶ in
  // the tree — which a large model does not surface (punch-list finding 15),
  // leaving the Cmd-K developer console as the only way to start the very
  // session the product exists to produce (finding 31). No target is a REAL
  // run: `sessions.create` with no `target` runs the whole workspace.
  it('enables Run on a loaded model with no target, and says it runs the whole workspace', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockLoadedFileCount = 3;
    render(<TransportBar />);

    const run = screen.getByTestId('transport-run');
    expect(run).not.toBeDisabled();
    expect(run).toHaveAttribute('title', WHOLE_WORKSPACE_TITLE);
    fireEvent.click(run);
    expect(controllerMocks.play).toHaveBeenCalledTimes(1);
  });

  it('enables Run once a run target is selected, and clicking it calls play()', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = 'Foo::bar';
    render(<TransportBar />);

    const run = screen.getByTestId('transport-run');
    expect(run).not.toBeDisabled();
    fireEvent.click(run);
    expect(controllerMocks.play).toHaveBeenCalledTimes(1);
  });

  it('shows Pause (not Run) while running', () => {
    publishMockController();
    mockPhase = 'running';
    mockActiveSessionId = 'sess-1';
    mockActiveSessionTarget = null;
    render(<TransportBar />);

    expect(screen.queryByTestId('transport-run')).toBeNull();
    expect(screen.getByTestId('transport-pause')).not.toBeDisabled();
  });

  it('shows Resume while paused, and clicking it calls resume()', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'paused';
    mockActiveSessionId = 'sess-1';
    mockActiveSessionTarget = null;
    render(<TransportBar />);

    const resume = screen.getByTestId('transport-resume');
    fireEvent.click(resume);
    expect(controllerMocks.resume).toHaveBeenCalledTimes(1);
  });

  it('Step calls stepOnce() and is disabled once the session has completed', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'paused';
    mockActiveSessionId = 'sess-1';
    mockActiveSessionTarget = null;
    const { rerender } = render(<TransportBar />);

    const step = screen.getByTestId('transport-step');
    expect(step).not.toBeDisabled();
    fireEvent.click(step);
    expect(controllerMocks.stepOnce).toHaveBeenCalledTimes(1);

    mockPhase = 'completed';
    rerender(<TransportBar />);
    expect(screen.getByTestId('transport-step')).toBeDisabled();
  });

  // ── Moved down from the frame (2026-07-14 follow-up to ruling A) ──

  it('Stop is disabled while idle, enabled while running/paused, and calls stop()', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    const { rerender } = render(<TransportBar />);
    expect(screen.getByTestId('transport-stop')).toBeDisabled();

    mockPhase = 'running';
    mockActiveSessionId = 'sess-1';
    rerender(<TransportBar />);
    const stop = screen.getByTestId('transport-stop');
    expect(stop).not.toBeDisabled();
    fireEvent.click(stop);
    expect(controllerMocks.stop).toHaveBeenCalledTimes(1);

    mockPhase = 'paused';
    rerender(<TransportBar />);
    expect(screen.getByTestId('transport-stop')).not.toBeDisabled();
  });

  it('"Run N ticks" is a generic bulk-step — no domain-named run-to-X button exists', () => {
    const controllerMocks = publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = 'Foo::bar';
    render(<TransportBar />);

    expect(screen.queryByText(/run to trip/i)).toBeNull();
    expect(screen.queryByText(/run to breakpoint/i)).toBeNull();

    const input = screen.getByTestId('transport-run-n-ticks-input') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '250' } });
    const go = screen.getByTestId('transport-run-n-ticks-go');
    expect(go.getAttribute('title')).toContain('stops early if a breakpoint fires');
    fireEvent.click(go);
    expect(controllerMocks.fastForward).toHaveBeenCalledWith(250);
  });

  it('disables the bulk-step control with the no-model reason when nothing is loaded', () => {
    publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockLoadedFileCount = 0;
    render(<TransportBar />);

    const go = screen.getByTestId('transport-run-n-ticks-go');
    expect(go).toBeDisabled();
    expect(go).toHaveAttribute('title', NO_MODEL_TITLE);
    expect(screen.getByTestId('transport-run-n-ticks-input')).toBeDisabled();
  });

  it('enables the bulk-step control on a loaded model with no target', () => {
    publishMockController();
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockLoadedFileCount = 3;
    render(<TransportBar />);

    expect(screen.getByTestId('transport-run-n-ticks-go')).not.toBeDisabled();
    expect(screen.getByTestId('transport-run-n-ticks-input')).not.toBeDisabled();
  });
});
