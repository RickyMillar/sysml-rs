/**
 * CreateViewPrompt v2 — projection-first flow (create-view-v2.md):
 * type card → specialized hierarchical scope picker (multi-expose) →
 * name → buffer append (dirty; editor owns save, §6).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, fireEvent, waitFor } from '@testing-library/react';
import type { ModelTreeNode } from '@/features/sessions/tree/types';

afterEach(cleanup);

const mutate = vi.fn((_refs: string[], opts?: { onSuccess?: (s: string) => void }) => {
  opts?.onSuccess?.('view scratch : GeneralView {\n    expose P::MotorStates;\n}');
});
vi.mock('@/features/views/queries', () => ({
  useCreateScratchView: () => ({ mutate, isPending: false, isError: false, error: null }),
}));

const node = (
  kind: ModelTreeNode['kind'],
  name: string,
  children: ModelTreeNode[] = [],
  rawKind?: string,
): ModelTreeNode =>
  ({
    id: name,
    elementId: `id-${name}`,
    uri: 'file:///models/m.sysml',
    name,
    rawKind: rawKind ?? kind,
    kind,
    depth: 0,
    ownerPath: '',
    children,
  }) as ModelTreeNode;

let mockTree: ModelTreeNode[] = [];
vi.mock('@/features/sessions/tree/useSessionModelTree', () => ({
  useSessionModelTree: () => ({ tree: mockTree, isLoading: false }),
}));

const updateSource = vi.fn();
const focusFile = vi.fn(async () => {});
let mockLoadedFiles = new Map<string, { uri: string; source: string; dirty: boolean; tree: [] }>();
vi.mock('@/store/workspace', () => {
  const useWorkspaceStore = (selector: (s: Record<string, unknown>) => unknown) =>
    selector({ updateSource, focusFile, loadedFiles: mockLoadedFiles });
  useWorkspaceStore.getState = () => ({ updateSource, focusFile, loadedFiles: mockLoadedFiles });
  return { useWorkspaceStore };
});

import { CreateViewPrompt } from '../CreateViewPrompt';

describe('CreateViewPrompt (v2 — projection first)', () => {
  beforeEach(() => {
    mutate.mockClear();
    updateSource.mockClear();
    mockTree = [
      node('part', 'motor', [
        node('port', 'shaft'),
        node('sm', 'MotorStates', [node('other', 'armed', [], 'StateUsage')]),
        node('action', 'spinUp'),
      ]),
    ];
    mockLoadedFiles = new Map([
      ['file:///models/m.sysml', { uri: 'file:///models/m.sysml', source: 'package P { }', dirty: false, tree: [] }],
    ]);
  });

  it('shows availability counts on the projection cards', () => {
    render(<CreateViewPrompt targetId="" context="modal" />);
    expect(screen.getByTestId('create-view-count-StateTransition').textContent).toBe('1');
    expect(screen.getByTestId('create-view-count-Interconnection').textContent).toBe('1');
    expect(screen.getByTestId('create-view-count-Geometry').textContent).toBe('1');
  });

  it('picking StateTransition narrows the scope to machines (hierarchy + inline states)', () => {
    render(<CreateViewPrompt targetId="" context="modal" />);
    fireEvent.click(screen.getByTestId('create-view-type-StateTransition'));
    // motor renders as a muted group header; MotorStates is the checkbox row.
    const scope = screen.getByTestId('create-view-scope');
    expect(scope.textContent).toContain('motor');
    expect(screen.getByTestId('create-view-scope-id-MotorStates')).toBeTruthy();
    expect(scope.textContent).toContain('armed');
    // actions are not offered under StateTransition
    expect(screen.queryByTestId('create-view-scope-id-spinUp')).toBeNull();
  });

  it('multi-expose create: rewrites tokens and appends to the owning file buffer', async () => {
    render(<CreateViewPrompt targetId="" context="modal" />);
    fireEvent.click(screen.getByTestId('create-view-type-StateTransition'));
    fireEvent.click(screen.getByTestId('create-view-scope-id-MotorStates').querySelector('input')!);
    expect((screen.getByTestId('create-view-name') as HTMLInputElement).value).toBe('MotorStatesView');

    fireEvent.click(screen.getByTestId('create-view-button'));
    expect(mutate).toHaveBeenCalledWith(['id-MotorStates'], expect.anything());
    await waitFor(() => expect(updateSource).toHaveBeenCalledTimes(1));
    const [uri, source] = updateSource.mock.calls[0];
    expect(uri).toBe('file:///models/m.sysml');
    expect(source).toContain('view MotorStatesView : StateTransitionView {');
    expect(source).toContain('expose P::MotorStates;');
    await waitFor(() => expect(screen.getByText(/unsaved/i)).toBeTruthy());
  });

  it('prefills the scope from the incoming target when eligible under the default kind', () => {
    render(<CreateViewPrompt targetId="id-motor" targetName="motor" context="browse" />);
    const row = screen.getByTestId('create-view-scope-id-motor');
    expect((row.querySelector('input') as HTMLInputElement).checked).toBe(true);
  });

  it('Create disabled with nothing selected', () => {
    render(<CreateViewPrompt targetId="" context="modal" />);
    expect(screen.getByTestId('create-view-button')).toBeDisabled();
  });
});
