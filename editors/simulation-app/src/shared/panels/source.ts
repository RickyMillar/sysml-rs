/**
 * Source panel descriptor — UtilityDrawer entry for the Monaco-mounted
 * SysML source viewer (S4.T4).
 *
 * Always applicable: the panel renders its own empty / no-selection /
 * no-span states. Lives at position 'utility' so the rendering path
 * goes through UtilityDrawer (where the panelRegistry's `render` is
 * called with the `emptyPanelProps` placeholder — Source ignores those).
 */
import { createElement } from 'react';
import { SourcePanel } from '@/features/editor/SourcePanel';
import type { PanelDescriptor } from './types';

export const sourcePanel: PanelDescriptor = {
  id: 'source',
  title: 'Source',
  icon: 'code',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  applicableWhen: () => true,
  render: () => createElement(SourcePanel),
};
