/**
 * DiagramHost dispatch gate — focuses on the 3.14 view-less-run branch:
 * a live session whose run target has no declared view renders CreateViewPrompt;
 * otherwise the normal renderers (SvgCanvas / non-graph) win.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

// Controllable store state.
const ws = {
  tableModel: null as unknown,
  geometryModel: null as unknown,
  treeModel: null as unknown,
  selectedViewId: null as string | null,
  workspaceRoot: '/ws' as string | null,
  focusedUri: 'file:///x.sysml' as string | null,
};
const ui = { activeSessionTarget: null as string | null };
const sess = { activeSessionId: null as string | null };
let viewsData: Array<{ exposed: { exposed_element_id: string }[] }> = [];

vi.mock('@/store/workspace', () => ({
  useWorkspaceStore: (sel: (s: typeof ws) => unknown) => sel(ws),
}));
vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (sel: (s: typeof ui) => unknown) => sel(ui),
}));
vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (sel: (s: typeof sess) => unknown) => sel(sess),
}));
vi.mock('@/features/views/queries', () => ({
  useViewsList: () => ({ data: viewsData }),
}));
// Stub the heavy renderers to markers so we assert which branch wins.
vi.mock('../TableView', () => ({ TableView: () => <div data-testid="m-table" /> }));
vi.mock('../GeometryView', () => ({ GeometryView: () => <div data-testid="m-geometry" /> }));
vi.mock('../BrowserView', () => ({ BrowserView: () => <div data-testid="m-browser" /> }));
vi.mock('../CreateViewPrompt', () => ({ CreateViewPrompt: () => <div data-testid="m-prompt" /> }));
vi.mock('../ViewlessState', () => ({ ViewlessState: () => <div data-testid="m-viewless" /> }));
vi.mock('@/diagram-svg/SvgCanvas', () => ({ SvgCanvas: () => <div data-testid="m-svg" /> }));

import { DiagramHost } from '../DiagramHost';

function reset() {
  ws.tableModel = ws.geometryModel = ws.treeModel = null;
  ws.selectedViewId = null;
  ui.activeSessionTarget = null;
  sess.activeSessionId = null;
  viewsData = [];
}
afterEach(() => {
  cleanup();
  reset();
});

describe('DiagramHost view-less-run gate (3.14)', () => {
  it('renders CreateViewPrompt for a live session whose target has no declared view', () => {
    sess.activeSessionId = 'sess-1';
    ui.activeSessionTarget = 'target-1';
    viewsData = []; // no view exposes target-1
    render(<DiagramHost />);
    expect(screen.getByTestId('m-prompt')).toBeTruthy();
    expect(screen.queryByTestId('m-svg')).toBeNull();
  });

  it('renders the view-less landing (not the prompt) when a declared view exposes the target but none is selected', () => {
    // W5: with no view SELECTED the landing surface lists the declared views —
    // it never silently falls through to an empty canvas.
    sess.activeSessionId = 'sess-1';
    ui.activeSessionTarget = 'target-1';
    viewsData = [{ exposed: [{ exposed_element_id: 'target-1' }] }];
    render(<DiagramHost />);
    expect(screen.getByTestId('m-viewless')).toBeTruthy();
    expect(screen.queryByTestId('m-prompt')).toBeNull();
    expect(screen.queryByTestId('m-svg')).toBeNull();
  });

  it('renders the view-less landing (not the prompt) when there is no active session', () => {
    sess.activeSessionId = null;
    ui.activeSessionTarget = 'target-1';
    render(<DiagramHost />);
    expect(screen.getByTestId('m-viewless')).toBeTruthy();
    expect(screen.queryByTestId('m-prompt')).toBeNull();
    expect(screen.queryByTestId('m-svg')).toBeNull();
  });

  it('renders SvgCanvas (not the prompt) when a view is already selected', () => {
    sess.activeSessionId = 'sess-1';
    ui.activeSessionTarget = 'target-1';
    ws.selectedViewId = 'view-9';
    render(<DiagramHost />);
    expect(screen.getByTestId('m-svg')).toBeTruthy();
    expect(screen.queryByTestId('m-prompt')).toBeNull();
  });

  it('non-graph payload still wins over the prompt', () => {
    sess.activeSessionId = 'sess-1';
    ui.activeSessionTarget = 'target-1';
    ws.treeModel = { roots: [] };
    render(<DiagramHost />);
    expect(screen.getByTestId('m-browser')).toBeTruthy();
    expect(screen.queryByTestId('m-prompt')).toBeNull();
  });
});
