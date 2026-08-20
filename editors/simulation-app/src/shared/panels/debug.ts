/**
 * debugPanel — Phase 8 PanelDescriptor for the dev-only DebugDrawer.
 *
 * Always registered in `panelRegistry`, but only surfaced as a toolbar
 * affordance in `UtilityDrawer` when `import.meta.env.VITE_DEBUG_DRAWER`
 * is `'1'`. The descriptor itself is harmless when the flag is off —
 * nothing references it.
 */

import { createElement } from 'react';
import { DebugDrawer } from '../../features/utilities/DebugDrawer';
import type { PanelDescriptor } from './types';

export const debugPanel: PanelDescriptor = {
  id: 'debug',
  title: 'Debug',
  icon: 'bug_report',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  applicableWhen: () => true,
  render: () => createElement(DebugDrawer),
};

/**
 * Whether the dev-only Debug drawer is enabled in this build/run.
 *
 * Reads `import.meta.env.VITE_DEBUG_DRAWER` at call time so vitest's
 * `vi.stubEnv` can toggle it per test. Vite inlines the value at build
 * time in production, so flipping the flag off truly removes the
 * affordance — there is no runtime cost.
 */
export function isDebugDrawerEnabled(): boolean {
  return (
    typeof import.meta !== 'undefined'
    && (import.meta as { env?: Record<string, string | undefined> }).env
      ?.VITE_DEBUG_DRAWER === '1'
  );
}
