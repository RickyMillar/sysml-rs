/**
 * ViewlessState (W5 / F14) — the first-class "no view selected" surface:
 * lists declared views (click renders via setSelectedViewId), or guides
 * scratch-view creation when the workspace declares none.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, fireEvent } from '@testing-library/react';
import type { ViewSummary } from '@/features/views/queries';

afterEach(cleanup);

const setSelectedViewId = vi.fn();
let workspaceRoot: string | null = '/ws';
let focusedUri: string | null = '/ws/Model.sysml';
let selectedElementId: string | null = null;
let viewsData: ViewSummary[] | undefined = [];
let viewsLoading = false;

vi.mock('@/store/workspace', () => ({
  useWorkspaceStore: (sel: (s: Record<string, unknown>) => unknown) =>
    sel({ workspaceRoot, focusedUri, setSelectedViewId }),
}));
vi.mock('@/features/selection/store', () => ({
  useSelectionStore: (sel: (s: Record<string, unknown>) => unknown) => sel({ selectedElementId }),
}));
vi.mock('@/features/views/queries', () => ({
  useViewsList: () => ({ data: viewsData, isLoading: viewsLoading }),
}));
const mutate = vi.fn();
vi.mock('@/features/views/ViewsPanel', () => ({
  viewKindLabel: (k: string) => (k.endsWith('Definition') ? 'view def' : 'view'),
  summariseExposed: () => 'Vehicle',
}));
// CreateViewPrompt pulls useCreateScratchView from the (mocked) queries module,
// so give the browse-context path a real mutation stub via its own mock.
vi.mock('../CreateViewPrompt', () => ({
  CreateViewPrompt: ({ targetId, context }: { targetId: string; context?: string }) => (
    <div data-testid="create-view-prompt" data-target={targetId} data-context={context} />
  ),
}));

import { ViewlessState } from '../ViewlessState';

const view = (id: string, name: string, kind = 'ViewUsage'): ViewSummary => ({
  id,
  name,
  kind,
  exposed: [],
  renderings: [],
  filters: [],
  source_span: null,
});

beforeEach(() => {
  setSelectedViewId.mockClear();
  mutate.mockClear();
  workspaceRoot = '/ws';
  focusedUri = '/ws/Model.sysml';
  selectedElementId = null;
  viewsData = [];
  viewsLoading = false;
});

describe('ViewlessState', () => {
  it('asks for a workspace when nothing is loaded', () => {
    workspaceRoot = null;
    focusedUri = null;
    render(<ViewlessState />);
    expect(screen.getByText(/Load a workspace/)).toBeTruthy();
  });

  it('lists declared views and renders one on click', () => {
    viewsData = [view('v1', 'OverviewView'), view('v2', 'PowertrainView', 'ViewDefinition')];
    render(<ViewlessState />);
    expect(screen.getByText(/declares 2/)).toBeTruthy();
    expect(screen.getByText('OverviewView')).toBeTruthy();
    fireEvent.click(screen.getByTestId('viewless-view-row-v2'));
    expect(setSelectedViewId).toHaveBeenCalledWith('v2');
  });

  it('with no views and no selection, explains views-first and hints at the tree', () => {
    render(<ViewlessState />);
    expect(screen.getByText('No declared views in this workspace')).toBeTruthy();
    expect(screen.getByTestId('viewless-select-hint')).toBeTruthy();
    expect(screen.queryByTestId('create-view-prompt')).toBeNull();
  });

  it('with no views and a tree selection, offers guided create in browse context', () => {
    selectedElementId = 'el-9';
    render(<ViewlessState />);
    const prompt = screen.getByTestId('create-view-prompt');
    expect(prompt.getAttribute('data-target')).toBe('el-9');
    expect(prompt.getAttribute('data-context')).toBe('browse');
  });
});
