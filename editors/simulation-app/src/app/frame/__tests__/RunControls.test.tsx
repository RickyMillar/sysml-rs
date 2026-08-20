/**
 * RunControls — after the 2026-07-14 "frame carries zero playback
 * controls" follow-up to ruling A, this component's jobs are: (1) own
 * the single useSessionController mount + publish it to the bridge,
 * (2) render the Configure gear. Playback (transport, bulk-step, stop,
 * injector) is TransportBar's territory — asserted absent here, present
 * there (TransportBar.test.tsx).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import type { SessionPhase } from '@/features/sessions/types';

afterEach(cleanup);

let mockPhase: SessionPhase = 'idle';
let mockActiveSessionId: string | null = null;
let mockActiveSessionTarget: string | null = null;

const controllerMocks = {
  play: vi.fn(),
  pause: vi.fn(),
  resume: vi.fn(),
  stop: vi.fn(),
  stepOnce: vi.fn(),
  fastForward: vi.fn(),
  runToBreakpoint: vi.fn(),
};

vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (
    selector: (s: { phase: SessionPhase; activeSessionId: string | null }) => unknown,
  ) => selector({ phase: mockPhase, activeSessionId: mockActiveSessionId }),
}));

vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (selector: (s: { activeSessionTarget: string | null }) => unknown) =>
    selector({ activeSessionTarget: mockActiveSessionTarget }),
}));

vi.mock('@/features/sessions/useSessionController', () => ({
  useSessionController: () => controllerMocks,
}));

import { RunControls } from '../RunControls';
import { useSessionControllerBridge } from '../sessionControllerBridge';

describe('RunControls', () => {
  it('renders ONLY the Configure gear — the frame carries zero playback controls', () => {
    mockPhase = 'running';
    mockActiveSessionId = 'sess-1';
    mockActiveSessionTarget = 'Foo::bar';
    render(<RunControls />);

    expect(screen.getByTestId('frame-control-configure')).toBeInTheDocument();
    // Everything playback-shaped moved to TransportBar (bottom strip).
    expect(screen.queryByTestId('frame-control-stop')).toBeNull();
    expect(screen.queryByTestId('frame-control-run-n-ticks-input')).toBeNull();
    expect(screen.queryByTestId('frame-control-run-n-ticks-go')).toBeNull();
    expect(screen.queryByTestId('frame-control-run')).toBeNull();
    expect(screen.queryByTestId('frame-injector-open')).toBeNull();
    // Guardrail (plan §4): no domain-named run button can ever appear here.
    expect(screen.queryByText(/run to trip/i)).toBeNull();
    expect(screen.queryByText(/run to breakpoint/i)).toBeNull();
  });

  it('publishes the controller to the bridge for TransportBar', () => {
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    render(<RunControls />);

    expect(useSessionControllerBridge.getState().controller).not.toBeNull();
    expect(useSessionControllerBridge.getState().controller?.stop).toBe(controllerMocks.stop);
  });
});
