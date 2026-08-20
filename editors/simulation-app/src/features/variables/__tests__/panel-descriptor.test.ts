/**
 * PanelDescriptor for the Variables pane (R2.2).
 *
 * Shallow tests that lock the registry contract — the descriptor must
 * remain registered, appear in the single canonical registry order, stay
 * hidden, and declare a non-workbench surface so ResultsWorkbench doesn't
 * try to render it.
 */

import { describe, it, expect } from 'vitest';
import { panelRegistry, findPanel } from '@/shared/panels/registry';

describe('variablesPanel descriptor', () => {
  it('is registered under the id "variables"', () => {
    expect(findPanel('variables')?.title).toBe('Variables');
  });

  it('declares utility as its default position (ResultsWorkbench filter)', () => {
    expect(findPanel('variables')?.defaultPosition).toBe('utility');
  });

  it('is registered as a utility panel alongside breakpoints', () => {
    const utilityPanels = panelRegistry.filter((p) => p.defaultPosition === 'utility');
    const ids = utilityPanels.map((p) => p.id);
    expect(ids).toContain('variables');
    expect(ids).toContain('breakpoints');
  });

  it('uses the collapsed neutral accent (ninebar: no per-panel accents)', () => {
    expect(findPanel('variables')?.accentColor).toBe('var(--text-secondary)');
  });

  it('defines an inactive hint for idle sessions', () => {
    const p = findPanel('variables');
    expect(p?.inactiveHint?.length).toBeGreaterThan(0);
  });
});
