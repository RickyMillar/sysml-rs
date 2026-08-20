/**
 * ConstraintDetail — render tests. Mocks useExpressionAst +
 * ExpressionViewReact so tests don't depend on the backend or the
 * KaTeX DOM plumbing.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

const mockUseExpressionAst = vi.fn();

vi.mock('@/features/results/useExpressionAst', () => ({
  useExpressionAst: (uri: string | null) => mockUseExpressionAst(uri),
}));

vi.mock('@/components/cards/ExpressionViewReact', () => ({
  ExpressionViewReact: ({ source }: { source: unknown }) => (
    <span data-testid="katex-mock" data-has-ast={source ? 'true' : 'false'} />
  ),
}));

import { ConstraintDetail } from '../detail/ConstraintDetail';
import type { ConstraintTreeNode } from '../types';

function node(overrides: Partial<ConstraintTreeNode> = {}): ConstraintTreeNode {
  return {
    id: 'c1',
    elementId: 'c1',
    uri: 'file:///w.sysml',
    name: 'tempBand',
    rawKind: 'AssertConstraintUsage',
    kind: 'constraint',
    depth: 2,
    ownerPath: 'ProductionCell.GroupHead',
    children: [],
    ...overrides,
  } as ConstraintTreeNode;
}

afterEach(() => {
  cleanup();
  mockUseExpressionAst.mockReset();
});

describe('ConstraintDetail — render', () => {
  it('loading state while ASTs are in flight', () => {
    mockUseExpressionAst.mockReturnValue({ data: undefined, isLoading: true });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-constraint-expression-loading'),
    ).toBeInTheDocument();
  });

  it('renders KaTeX view when a matching AST is returned', () => {
    mockUseExpressionAst.mockReturnValue({
      data: [
        {
          element_id: 'c1',
          element_name: 'tempBand',
          element_kind: 'ConstraintUsage',
          source: 'current < rated',
          ast: { kind: 'op', op: '<' }, // any non-null ast triggers the render
        },
      ],
      isLoading: false,
    });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-constraint-expression-katex'),
    ).toBeInTheDocument();
    expect(screen.getByTestId('katex-mock')).toHaveAttribute(
      'data-has-ast',
      'true',
    );
  });

  it('falls back to source-code text when no AST is parsed but source is present', () => {
    mockUseExpressionAst.mockReturnValue({
      data: [
        {
          element_id: 'c1',
          element_name: 'tempBand',
          element_kind: 'ConstraintUsage',
          source: 'current < rated',
          ast: null,
        },
      ],
      isLoading: false,
    });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    const src = screen.getByTestId('d-constraint-expression-source');
    expect(src).toHaveTextContent('current < rated');
    // No KaTeX block when there's no AST.
    expect(screen.queryByTestId('d-constraint-expression-katex')).toBeNull();
  });

  it('prefers node.expression over AST source when AST absent', () => {
    // ConstraintView.expression (merged by mergeLiveState) is the
    // runtime-serialised form; preferred over the original parsed
    // source if both are available.
    mockUseExpressionAst.mockReturnValue({
      data: [],
      isLoading: false,
    });
    render(
      <ConstraintDetail
        node={node({ expression: 'temperature > 0' })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-constraint-expression-source')).toHaveTextContent(
      'temperature > 0',
    );
  });

  it('shows absent-state when neither AST nor source is available', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-constraint-expression-absent'),
    ).toBeInTheDocument();
  });

  it('renders the verdict badge when a verdict is present', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(
      <ConstraintDetail
        node={node({ verdict: 'fail' })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-constraint-verdict')).toBeInTheDocument();
  });

  it('omits the verdict badge when verdict is undefined', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    expect(screen.queryByTestId('d-constraint-verdict')).toBeNull();
  });

  it('shows the constraint name prominently', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(
      <ConstraintDetail
        node={node({ name: 'thermalBound' })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-constraint-name')).toHaveTextContent(
      'thermalBound',
    );
  });
});

describe('ConstraintDetail — live-operand overlay (GAP-CONSTR-002)', () => {
  it('shows the pending placeholder when operands is undefined', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(<ConstraintDetail node={node()} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-constraint-operands-pending'),
    ).toBeInTheDocument();
  });

  it('shows the empty placeholder when operands is present but has no entries', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(
      <ConstraintDetail
        node={node({ operands: {} })}
        testIdPrefix="d"
      />,
    );
    expect(
      screen.getByTestId('d-constraint-operands-empty'),
    ).toBeInTheDocument();
  });

  it('lists each operand with its current scalar value, sorted alphabetically', () => {
    mockUseExpressionAst.mockReturnValue({ data: [], isLoading: false });
    render(
      <ConstraintDetail
        node={node({
          operands: { temperature: 321.5, cap: 400 },
          verdict: 'pass',
        })}
        testIdPrefix="d"
      />,
    );
    const panel = screen.getByTestId('d-constraint-operands');
    expect(panel).toBeInTheDocument();
    expect(
      screen.getByTestId('d-constraint-operand-temperature'),
    ).toBeInTheDocument();
    expect(screen.getByTestId('d-constraint-operand-cap')).toBeInTheDocument();
    // Names sort cap < temperature → cap renders first in the grid.
    const names = panel.textContent!;
    expect(names.indexOf('cap')).toBeLessThan(names.indexOf('temperature'));
    // Values rendered.
    expect(
      screen.getByTestId('d-constraint-operand-temperature-value')
        .textContent,
    ).toContain('321.5');
    expect(
      screen.getByTestId('d-constraint-operand-cap-value').textContent,
    ).toContain('400');
  });
});
