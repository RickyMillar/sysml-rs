/**
 * AttributeRow — structural + interaction tests.
 *
 * Phase A3 scaffolding: the Phase B session tree detail region renders
 * one of these rows per AttributeUsage under the focused part. The
 * tests lock down the minimum contract the tree will rely on:
 *  - value formatting including units
 *  - verdict pill when a constraint verdict is present
 *  - sparkline visibility threshold
 *  - pin / edit callbacks stopPropagation (so the row click doesn't
 *    fire a drill when the user intends to pin or edit)
 *  - flash animation triggers when the value changes
 */

import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { AttributeRow } from '../AttributeRow';

afterEach(() => {
  cleanup();
});

describe('AttributeRow — basic render', () => {
  it('renders the name and formatted value with unit', () => {
    render(
      <AttributeRow id="bimetalTemp" name="bimetalTemp" value={298.15} unit="K" />,
    );
    const row = screen.getByTestId('attribute-row-bimetalTemp');
    expect(row).toHaveTextContent('bimetalTemp');
    expect(row).toHaveTextContent('K');
    // Formatter uses toPrecision(5) with trailing-zero trim — "298.15".
    expect(row).toHaveTextContent('298.15');
  });

  it('formats null as em-dash', () => {
    render(<AttributeRow id="v" name="v" value={null} />);
    expect(screen.getByTestId('attribute-row-v')).toHaveTextContent('—');
  });

  it('renders the verdict pill when a verdict is provided', () => {
    render(
      <AttributeRow
        id="current"
        name="loadCurrent"
        value={12.5}
        unit="A"
        verdict="fail"
      />,
    );
    // Pill testid namespaces under the row's testIdPrefix.
    expect(
      screen.getByTestId('attribute-row-current-pill'),
    ).toBeInTheDocument();
  });

  it('reflects selected state with data-selected="true"', () => {
    render(<AttributeRow id="v" name="v" value={1} selected />);
    expect(screen.getByTestId('attribute-row-v')).toHaveAttribute(
      'data-selected',
      'true',
    );
  });
});

describe('AttributeRow — sparkline gating', () => {
  it('hides the sparkline when fewer than 3 samples are available', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        sparklineSamples={[1, 2]}
      />,
    );
    expect(screen.queryByLabelText('v sparkline')).toBeNull();
  });

  it('renders the sparkline once 3+ samples exist', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={3}
        sparklineSamples={[1, 2, 3]}
      />,
    );
    expect(screen.getByLabelText('v sparkline')).toBeInTheDocument();
  });

  it('showSparkline=false suppresses the sparkline even with samples', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={3}
        sparklineSamples={[1, 2, 3, 4]}
        showSparkline={false}
      />,
    );
    expect(screen.queryByLabelText('v sparkline')).toBeNull();
  });
});

describe('AttributeRow — interactions', () => {
  it('fires onClick when the row body is clicked', () => {
    const onClick = vi.fn();
    render(
      <AttributeRow id="v" name="v" value={1} onClick={onClick} />,
    );
    fireEvent.click(screen.getByTestId('attribute-row-v'));
    expect(onClick).toHaveBeenCalled();
  });

  it('pin button does not bubble into row onClick', () => {
    const onClick = vi.fn();
    const onTogglePin = vi.fn();
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        onClick={onClick}
        onTogglePin={onTogglePin}
      />,
    );
    fireEvent.click(screen.getByTestId('attribute-row-v-pin'));
    expect(onTogglePin).toHaveBeenCalledOnce();
    expect(onClick).not.toHaveBeenCalled();
  });

  it('pin button reflects pinned state via aria-pressed', () => {
    const { rerender } = render(
      <AttributeRow id="v" name="v" value={1} onTogglePin={vi.fn()} />,
    );
    expect(screen.getByTestId('attribute-row-v-pin')).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    rerender(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        onTogglePin={vi.fn()}
        pinned
      />,
    );
    expect(screen.getByTestId('attribute-row-v-pin')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('edit pencil only renders when editable + onEdit are both provided', () => {
    const onEdit = vi.fn();
    const { rerender } = render(
      <AttributeRow id="v" name="v" value={1} onEdit={onEdit} />,
    );
    // onEdit present but editable omitted → no pencil.
    expect(screen.queryByTestId('attribute-row-v-edit')).toBeNull();
    rerender(<AttributeRow id="v" name="v" value={1} editable />);
    // editable without onEdit → still no pencil (no click target).
    expect(screen.queryByTestId('attribute-row-v-edit')).toBeNull();
    rerender(
      <AttributeRow id="v" name="v" value={1} editable onEdit={onEdit} />,
    );
    const pencil = screen.getByTestId('attribute-row-v-edit');
    fireEvent.click(pencil);
    expect(onEdit).toHaveBeenCalledOnce();
  });

  it('edit pencil click does not bubble to row onClick', () => {
    const onClick = vi.fn();
    const onEdit = vi.fn();
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        onClick={onClick}
        editable
        onEdit={onEdit}
      />,
    );
    fireEvent.click(screen.getByTestId('attribute-row-v-edit'));
    expect(onEdit).toHaveBeenCalledOnce();
    expect(onClick).not.toHaveBeenCalled();
  });

  it('onContextMenu fires with the mouse position', () => {
    const onContextMenu = vi.fn();
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        onContextMenu={onContextMenu}
      />,
    );
    fireEvent.contextMenu(screen.getByTestId('attribute-row-v'), {
      clientX: 123,
      clientY: 456,
    });
    expect(onContextMenu).toHaveBeenCalledWith({ x: 123, y: 456 });
  });
});

describe('AttributeRow — sparkline click', () => {
  it('renders the sparkline as a plain svg when no handler is passed', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        sparklineSamples={[1, 2, 3]}
      />,
    );
    expect(screen.queryByTestId('attribute-row-v-spark-btn')).toBeNull();
  });

  it('wraps the sparkline in a button when onSparklineClick is provided', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        sparklineSamples={[1, 2, 3]}
        onSparklineClick={() => {}}
      />,
    );
    expect(
      screen.getByTestId('attribute-row-v-spark-btn'),
    ).toBeInTheDocument();
  });

  it('fires onSparklineClick but NOT the row onClick', () => {
    const onSparklineClick = vi.fn();
    const onClick = vi.fn();
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        sparklineSamples={[1, 2, 3]}
        onClick={onClick}
        onSparklineClick={onSparklineClick}
      />,
    );
    fireEvent.click(screen.getByTestId('attribute-row-v-spark-btn'));
    expect(onSparklineClick).toHaveBeenCalledOnce();
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe('AttributeRow — flash on value change', () => {
  it('remounts the flash span (new key) when the value changes', () => {
    function Harness() {
      const [v, setV] = useState(1);
      return (
        <div>
          <button
            data-testid="bump"
            type="button"
            onClick={() => setV((x) => x + 1)}
          >
            bump
          </button>
          <AttributeRow id="v" name="v" value={v} />
        </div>
      );
    }
    render(<Harness />);
    const row1 = screen.getByTestId('attribute-row-v');
    const flash1 = row1.querySelector('.sysml-variable-flash');
    expect(flash1).not.toBeNull();
    fireEvent.click(screen.getByTestId('bump'));
    const flash2 = screen.getByTestId('attribute-row-v').querySelector(
      '.sysml-variable-flash',
    );
    expect(flash2).not.toBeNull();
    // Different DOM node (React key changed) — the flash span remounted.
    expect(flash2).not.toBe(flash1);
  });
});

describe('AttributeRow — title tooltip', () => {
  it('includes name and last-changed tick in the title attribute', () => {
    render(
      <AttributeRow
        id="v"
        name="bimetalTemp"
        value={1}
        lastChangedTick={471}
      />,
    );
    const row = screen.getByTestId('attribute-row-v');
    const title = row.getAttribute('title')!;
    expect(title).toContain('bimetalTemp');
    expect(title).toContain('tick 471');
  });
});

describe('AttributeRow — testIdPrefix', () => {
  it('namespaces all testids under the supplied prefix', () => {
    render(
      <AttributeRow
        id="v"
        name="v"
        value={1}
        testIdPrefix="tree-attr"
        onTogglePin={vi.fn()}
        editable
        onEdit={vi.fn()}
        verdict="pass"
      />,
    );
    expect(screen.getByTestId('tree-attr-v')).toBeInTheDocument();
    expect(screen.getByTestId('tree-attr-v-label')).toBeInTheDocument();
    expect(screen.getByTestId('tree-attr-v-pin')).toBeInTheDocument();
    expect(screen.getByTestId('tree-attr-v-edit')).toBeInTheDocument();
    expect(screen.getByTestId('tree-attr-v-pill')).toBeInTheDocument();
    // Default prefix is NOT used when the consumer overrides.
    expect(screen.queryByTestId('attribute-row-v')).toBeNull();
  });
});
