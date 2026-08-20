/**
 * modeRegistry — source-of-truth for the three R4.3 Compare modes.
 *
 * Agent W's CompareWorkflow shell iterates `compareModes` to paint the
 * mode switcher and to look up `configRender` / `mainRender`. Keeping
 * this list in one file (not scattered across the mode files) means
 * reconciling with W's shell is a one-import fix if the interface
 * shifts at merge time.
 *
 * Stable ids: `'ensemble' | 'golden' | 'two-design'`.
 */
import type { CompareMode } from '../compareMode';
import { ensembleMode } from './ensemble';
import { goldenMode } from './golden';
import { twoDesignMode } from './twoDesign';

/** The canonical mode id type (compile-time guard against typos). */
export type CompareModeId = 'ensemble' | 'golden' | 'two-design';

/**
 * All three modes, in the order they should appear in the UI's
 * switcher (ensemble first because it's the simplest — reproducibility
 * check — and the other two build on top of its stats layer).
 */
export const compareModes: CompareMode[] = [ensembleMode, goldenMode, twoDesignMode];

/** Lookup table — O(1) access by id. */
export const compareModeById: Record<CompareModeId, CompareMode> = {
  ensemble: ensembleMode,
  golden: goldenMode,
  'two-design': twoDesignMode,
};

// Re-export so consumers only need one import path.
export { ensembleMode, goldenMode, twoDesignMode };
export type { CompareMode, CompareContext } from '../compareMode';
