import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { WorkspaceAttributePicker } from '../WorkspaceAttributePicker';
import type { MetricDescriptor } from '../../metrics/types';

afterEach(() => {
  cleanup();
});

const CANDIDATES: MetricDescriptor[] = [
  {
    id: 'voltage',
    name: 'voltage',
    source: 'variable',
    expression: 'voltage',
    unit: 'V',
    domain: 'electrical',
  },
  {
    id: 'loadCurrent',
    name: 'loadCurrent',
    source: 'variable',
    expression: 'loadCurrent',
    unit: 'A',
    domain: 'electrical',
  },
  {
    id: 'bimetalTemp',
    name: 'bimetalTemp',
    source: 'variable',
    expression: 'bimetalTemp',
    unit: 'K',
    domain: 'thermal',
  },
  {
    id: 'trip_time',
    name: 'trip_time',
    source: 'expression',
    expression: 'first_crossing(bimetalTemp > 350)',
  },
];

describe('WorkspaceAttributePicker — rendering', () => {
  it('renders each candidate with its name, unit, and domain subtitle', () => {
    const onToggle = vi.fn();
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={[]}
        onToggle={onToggle}
      />,
    );
    const root = screen.getByTestId('workspace-attribute-picker');
    expect(root).toHaveAttribute('data-candidate-count', '4');
    expect(root).toHaveAttribute('data-selected-count', '0');
    for (const c of CANDIDATES) {
      const row = screen.getByTestId(`workspace-attribute-picker-row-${c.id}`);
      expect(row).toHaveTextContent(c.name);
      expect(row).toHaveAttribute('data-checked', 'false');
      expect(row).toHaveAttribute('data-source', c.source);
    }
    // Units are rendered for variables that carry them.
    expect(screen.getByTestId('workspace-attribute-picker-unit-voltage')).toHaveTextContent(
      'V',
    );
    // Expression source is tagged in the subtitle — distinguishes derived
    // metrics from raw variables in the list.
    const tripSub = screen.getByTestId(
      'workspace-attribute-picker-subtitle-trip_time',
    );
    expect(tripSub.textContent).toContain('expression');
  });

  it('marks checked rows with data-checked="true" and reflects selection count', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={['voltage', 'bimetalTemp']}
        onToggle={vi.fn()}
      />,
    );
    expect(
      screen.getByTestId('workspace-attribute-picker-row-voltage'),
    ).toHaveAttribute('data-checked', 'true');
    expect(
      screen.getByTestId('workspace-attribute-picker-row-bimetalTemp'),
    ).toHaveAttribute('data-checked', 'true');
    expect(
      screen.getByTestId('workspace-attribute-picker-row-loadCurrent'),
    ).toHaveAttribute('data-checked', 'false');
    expect(screen.getByTestId('workspace-attribute-picker')).toHaveAttribute(
      'data-selected-count',
      '2',
    );
  });
});

describe('WorkspaceAttributePicker — toggling', () => {
  it('fires onToggle with the candidate id when a checkbox changes', () => {
    const onToggle = vi.fn();
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={['voltage']}
        onToggle={onToggle}
      />,
    );
    const checkVoltage = screen.getByTestId(
      'workspace-attribute-picker-checkbox-voltage',
    );
    fireEvent.click(checkVoltage);
    expect(onToggle).toHaveBeenCalledWith('voltage');

    const checkCurrent = screen.getByTestId(
      'workspace-attribute-picker-checkbox-loadCurrent',
    );
    fireEvent.click(checkCurrent);
    expect(onToggle).toHaveBeenLastCalledWith('loadCurrent');
  });

  it('round-trips toggle state when the consumer owns selection state', () => {
    // Sanity check that the "dumb component" contract works: consumer
    // flips state, component re-renders with updated data-checked.
    function Harness() {
      const [selected, setSelected] = useState<string[]>([]);
      return (
        <WorkspaceAttributePicker
          candidates={CANDIDATES}
          selected={selected}
          onToggle={(id) =>
            setSelected((prev) =>
              prev.includes(id) ? prev.filter((p) => p !== id) : [...prev, id],
            )
          }
        />
      );
    }
    render(<Harness />);
    const row = () =>
      screen.getByTestId('workspace-attribute-picker-row-bimetalTemp');
    expect(row()).toHaveAttribute('data-checked', 'false');
    fireEvent.click(
      screen.getByTestId('workspace-attribute-picker-checkbox-bimetalTemp'),
    );
    expect(row()).toHaveAttribute('data-checked', 'true');
    fireEvent.click(
      screen.getByTestId('workspace-attribute-picker-checkbox-bimetalTemp'),
    );
    expect(row()).toHaveAttribute('data-checked', 'false');
  });
});

describe('WorkspaceAttributePicker — filter', () => {
  it('filters by candidate name (case-insensitive)', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );
    const input = screen.getByTestId('workspace-attribute-picker-filter');
    fireEvent.change(input, { target: { value: 'TEMP' } });
    expect(
      screen.getByTestId('workspace-attribute-picker-row-bimetalTemp'),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId('workspace-attribute-picker-row-voltage'),
    ).toBeNull();
  });

  it('filters by domain so thermal-only quickly narrows the list', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );
    const input = screen.getByTestId('workspace-attribute-picker-filter');
    fireEvent.change(input, { target: { value: 'thermal' } });
    expect(
      screen.getByTestId('workspace-attribute-picker-row-bimetalTemp'),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId('workspace-attribute-picker-row-voltage'),
    ).toBeNull();
    expect(
      screen.queryByTestId('workspace-attribute-picker-row-loadCurrent'),
    ).toBeNull();
  });

  it('shows the filtered-empty state when the filter excludes everything', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );
    fireEvent.change(
      screen.getByTestId('workspace-attribute-picker-filter'),
      { target: { value: 'zzzzzz' } },
    );
    expect(
      screen.getByTestId('workspace-attribute-picker-filtered-empty'),
    ).toBeInTheDocument();
  });
});

describe('WorkspaceAttributePicker — empty states', () => {
  it('renders the no-workspace row when hasWorkspace=false', () => {
    render(
      <WorkspaceAttributePicker
        candidates={[]}
        selected={[]}
        onToggle={vi.fn()}
        hasWorkspace={false}
      />,
    );
    expect(
      screen.getByTestId('workspace-attribute-picker-no-workspace'),
    ).toBeInTheDocument();
    // Filter input is suppressed so the user isn't misled into typing.
    expect(
      screen.queryByTestId('workspace-attribute-picker-filter'),
    ).toBeNull();
  });

  it('renders the loading row when isLoading=true', () => {
    render(
      <WorkspaceAttributePicker
        candidates={[]}
        selected={[]}
        onToggle={vi.fn()}
        isLoading
      />,
    );
    expect(
      screen.getByTestId('workspace-attribute-picker-loading'),
    ).toBeInTheDocument();
  });

  it('renders the empty row when the workspace has no candidates', () => {
    render(
      <WorkspaceAttributePicker
        candidates={[]}
        selected={[]}
        onToggle={vi.fn()}
      />,
    );
    expect(
      screen.getByTestId('workspace-attribute-picker-empty'),
    ).toBeInTheDocument();
  });

  it('allows per-consumer message overrides', () => {
    render(
      <WorkspaceAttributePicker
        candidates={[]}
        selected={[]}
        onToggle={vi.fn()}
        hasWorkspace={false}
        messages={{
          noWorkspaceTitle: 'Open a project first',
          noWorkspaceHint: 'Sweep needs parameters.',
        }}
      />,
    );
    const empty = screen.getByTestId('workspace-attribute-picker-no-workspace');
    expect(empty).toHaveTextContent('Open a project first');
    expect(empty).toHaveTextContent('Sweep needs parameters.');
  });
});

describe('WorkspaceAttributePicker — renderExpanded slot', () => {
  it('renders the per-row editor only under checked rows', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={['voltage']}
        onToggle={vi.fn()}
        renderExpanded={(id) => (
          <span data-testid={`editor-${id}`}>editor for {id}</span>
        )}
      />,
    );
    // Checked row → expanded slot appears.
    expect(
      screen.getByTestId('workspace-attribute-picker-expanded-voltage'),
    ).toBeInTheDocument();
    expect(screen.getByTestId('editor-voltage')).toHaveTextContent(
      'editor for voltage',
    );
    // Unchecked row → no slot.
    expect(
      screen.queryByTestId('workspace-attribute-picker-expanded-loadCurrent'),
    ).toBeNull();
    expect(screen.queryByTestId('editor-loadCurrent')).toBeNull();
  });

  it('skips the expanded slot when renderExpanded returns null', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={['voltage']}
        onToggle={vi.fn()}
        renderExpanded={() => null}
      />,
    );
    expect(
      screen.queryByTestId('workspace-attribute-picker-expanded-voltage'),
    ).toBeNull();
  });

  it('lets each consumer pick its own testIdPrefix for scoping', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES.slice(0, 2)}
        selected={['voltage']}
        onToggle={vi.fn()}
        testIdPrefix="sweep-params"
        renderExpanded={(id) => <span data-testid={`range-${id}`}>range</span>}
      />,
    );
    expect(screen.getByTestId('sweep-params')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-params-row-voltage')).toBeInTheDocument();
    expect(
      screen.getByTestId('sweep-params-expanded-voltage'),
    ).toBeInTheDocument();
    // Default prefix is NOT used when consumer overrides.
    expect(screen.queryByTestId('workspace-attribute-picker')).toBeNull();
  });
});

describe('WorkspaceAttributePicker — ordering', () => {
  it('preserves candidate order — consumer pre-sorts, picker does not reorder', () => {
    render(
      <WorkspaceAttributePicker
        candidates={CANDIDATES}
        selected={['trip_time']}
        onToggle={vi.fn()}
      />,
    );
    const list = screen.getByTestId('workspace-attribute-picker-list');
    const rows = within(list).getAllByRole('checkbox');
    // Checkbox ids reflect the input array order, regardless of selection.
    const ids = rows.map((r) =>
      (r.getAttribute('data-testid') ?? '').replace(
        'workspace-attribute-picker-checkbox-',
        '',
      ),
    );
    expect(ids).toEqual(['voltage', 'loadCurrent', 'bimetalTemp', 'trip_time']);
  });
});
