/**
 * Registry contract test (R4.3).
 *
 * Asserts there are exactly three modes, with the stable ids the
 * Compare shell expects. If this breaks, Agent W's shell will silently
 * mount the wrong mode — so we test it hard.
 */
import { describe, it, expect } from 'vitest';
import { compareModeById, compareModes } from '../modeRegistry';

describe('compareModes registry', () => {
  it('has exactly 3 entries', () => {
    expect(compareModes).toHaveLength(3);
  });

  it('uses the stable ids ensemble | golden | two-design', () => {
    const ids = compareModes.map((m) => m.id).sort();
    expect(ids).toEqual(['ensemble', 'golden', 'two-design']);
  });

  it('every mode has a label + description + configRender', () => {
    for (const mode of compareModes) {
      expect(mode.label).toBeTruthy();
      expect(mode.description).toBeTruthy();
      expect(typeof mode.configRender).toBe('function');
    }
  });

  it('compareModeById resolves each id', () => {
    expect(compareModeById.ensemble.id).toBe('ensemble');
    expect(compareModeById.golden.id).toBe('golden');
    expect(compareModeById['two-design'].id).toBe('two-design');
  });

  it('keeps insertion order: ensemble, golden, two-design', () => {
    expect(compareModes.map((m) => m.id)).toEqual(['ensemble', 'golden', 'two-design']);
  });
});
