/**
 * DetailPanel Dispatch — verifies each ModelTreeNode kind resolves
 * to the right per-kind subcomponent, and that the empty state
 * renders when nothing is focused.
 *
 * Testing Dispatch directly (not the store-wired DetailPanel) keeps
 * these tests hook-free. The store + focusPath integration is
 * covered by the end-of-round browser smoke.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';

// ConstraintDetail pulls from react-query via useExpressionAst;
// Dispatch routing doesn't care about fetched data, so stub the
// hook at the module boundary to keep tests QueryClient-free.
vi.mock('@/features/results/useExpressionAst', () => ({
  useExpressionAst: () => ({ data: [], isLoading: false }),
}));

// SmDetail's accepted-events rail uses useInjectEvent (react-query
// mutation) and reads activeSessionId from the session store; stub
// both here so routing stays hook-free.
vi.mock('@/features/sessions/mutations', () => ({
  useInjectEvent: () => ({ mutate: () => {}, isPending: false }),
}));
vi.mock('../../store', () => ({
  useSessionStore: () => null,
}));

const mocks = vi.hoisted(() => ({
  setActiveSessionTarget: vi.fn(),
  navigate: vi.fn(),
}));

vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (selector: (state: { setActiveSessionTarget: typeof mocks.setActiveSessionTarget }) => unknown) =>
    selector({ setActiveSessionTarget: mocks.setActiveSessionTarget }),
}));

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return {
    ...actual,
    useNavigate: () => mocks.navigate,
  };
});

import { Dispatch } from '../detail/DetailPanel';
import type { ModelTreeNode } from '../types';

afterEach(() => {
  cleanup();
  mocks.setActiveSessionTarget.mockClear();
  mocks.navigate.mockClear();
});

function node<T extends ModelTreeNode['kind']>(
  kind: T,
  extra: Partial<ModelTreeNode> = {},
): ModelTreeNode {
  return {
    id: `id-${kind}`,
    elementId: `element-${kind}`,
    uri: 'file:///w.sysml',
    name: `name-${kind}`,
    rawKind: kind === 'part' ? 'PartUsage' : 'Unknown',
    kind,
    depth: 0,
    ownerPath: '',
    children: [],
    ...extra,
  } as ModelTreeNode;
}

describe('DetailPanel — Dispatch', () => {
  it('empty state when node is null', () => {
    render(<Dispatch node={null} testIdPrefix="d" />);
    expect(screen.getByTestId('d-empty')).toBeInTheDocument();
  });

  it('attribute → AttributeDetail', () => {
    render(<Dispatch node={node('attribute')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-attribute')).toBeInTheDocument();
  });

  it('constraint → ConstraintDetail', () => {
    render(<Dispatch node={node('constraint')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-constraint')).toBeInTheDocument();
  });

  it('sm → SmDetail', () => {
    render(<Dispatch node={node('sm')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-sm')).toBeInTheDocument();
  });

  it('ode → OdeDetail', () => {
    render(<Dispatch node={node('ode')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-ode')).toBeInTheDocument();
  });

  it('calc → CalcDetail', () => {
    render(<Dispatch node={node('calc')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-calc')).toBeInTheDocument();
  });

  it('part → PartDetail (health + signals rendered)', () => {
    render(<Dispatch node={node('part')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-part')).toBeInTheDocument();
    expect(screen.getByTestId('d-part-health')).toBeInTheDocument();
  });

  it('section → SectionDetail (hint copy rendered)', () => {
    render(
      <Dispatch
        node={node('section', {
          sectionKind: 'outputs',
          count: 3,
        } as Partial<ModelTreeNode>)}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-section')).toBeInTheDocument();
    expect(screen.getByTestId('d-section-hint')).toBeInTheDocument();
  });

  it('action → ActionDetail with launch for AnalysisCase', () => {
    render(
      <MemoryRouter>
        <Dispatch
          node={node('action', { rawKind: 'AnalysisCaseUsage', elementId: 'case-1' })}
          testIdPrefix="d"
        />
      </MemoryRouter>,
    );
    expect(screen.getByTestId('d-action')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('d-action-launch'));
    expect(mocks.setActiveSessionTarget).toHaveBeenCalledWith('case-1');
    expect(mocks.navigate).toHaveBeenCalledWith('/analyze/sweep');
  });

  it('action → ActionDetail with launch for VerificationCase', () => {
    render(
      <MemoryRouter>
        <Dispatch
          node={node('action', { rawKind: 'VerificationCaseDefinition', elementId: 'case-v' })}
          testIdPrefix="d"
        />
      </MemoryRouter>,
    );
    fireEvent.click(screen.getByTestId('d-action-launch'));
    expect(mocks.setActiveSessionTarget).toHaveBeenCalledWith('case-v');
    expect(mocks.navigate).toHaveBeenCalledWith('/verify');
  });

  it('other → OtherDetail', () => {
    render(<Dispatch node={node('other')} testIdPrefix="d" />);
    expect(screen.getByTestId('d-other')).toBeInTheDocument();
  });
});
