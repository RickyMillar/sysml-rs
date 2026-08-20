/**
 * promoteToPlots — idempotent helper that pushes a variable into
 * the Plots tab's selection set for the active session.
 */
import { beforeEach, describe, expect, it } from 'vitest';
import { promoteToPlots } from '../detail/promoteToPlots';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';

describe('promoteToPlots', () => {
  beforeEach(() => {
    usePlotSelectionStore.setState({ selectionsBySession: {} });
  });

  it('adds a variable to the session selection', () => {
    promoteToPlots('circuit1.current', 'sess-a');
    expect(
      usePlotSelectionStore.getState().getSelected('sess-a'),
    ).toEqual(['circuit1.current']);
  });

  it('is idempotent — adding the same name twice keeps one copy', () => {
    promoteToPlots('v_bus', 'sess-a');
    promoteToPlots('v_bus', 'sess-a');
    expect(
      usePlotSelectionStore.getState().getSelected('sess-a'),
    ).toEqual(['v_bus']);
  });

  it('preserves previously selected variables', () => {
    usePlotSelectionStore
      .getState()
      .setSelected('sess-a', ['pre-existing']);
    promoteToPlots('new', 'sess-a');
    expect(
      usePlotSelectionStore.getState().getSelected('sess-a'),
    ).toEqual(['pre-existing', 'new']);
  });

  it('no-ops quietly when sessionId is null', () => {
    promoteToPlots('x', null);
    expect(usePlotSelectionStore.getState().selectionsBySession).toEqual(
      {},
    );
  });

  it('no-ops quietly when name is empty', () => {
    promoteToPlots('', 'sess-a');
    expect(
      usePlotSelectionStore.getState().getSelected('sess-a'),
    ).toEqual([]);
  });

  it('keeps sessions isolated', () => {
    promoteToPlots('a', 'sess-a');
    promoteToPlots('b', 'sess-b');
    const s = usePlotSelectionStore.getState();
    expect(s.getSelected('sess-a')).toEqual(['a']);
    expect(s.getSelected('sess-b')).toEqual(['b']);
  });
});
