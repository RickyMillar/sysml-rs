/**
 * breakpointsPanel — PanelDescriptor for the BreakpointsPanel (R2.3).
 *
 * Surfaced via `defaultPosition: 'utility'`. ResultsWorkbench only
 * iterates `defaultPosition === 'workbench'` panels, so this descriptor
 * is picked up by the shell utility drawer without the workbench
 * rendering it. Registering it gives every consumer a single way to
 * mount the panel — a one-line `findPanel('breakpoints')` lookup.
 *
 * The `render` path intentionally doesn't thread `PanelProps` through:
 * the BreakpointsPanel is self-contained (pulls from its own store,
 * session store, engine hooks) and doesn't need the PanelProps bundle
 * that result-workbench panels consume. Hosts render the component
 * directly via `<BreakpointsPanel />` — the descriptor exposes the
 * metadata (title / icon / accentColor) that a sidebar host uses for
 * its chrome.
 */

import { createElement } from 'react';
import { BreakpointsPanel } from '../../features/breakpoints/BreakpointsPanel';
import type { PanelDescriptor } from './types';

export const breakpointsPanel: PanelDescriptor = {
  id: 'breakpoints',
  title: 'Breakpoints',
  icon: 'radio_button_checked',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  // Always applicable — the panel's empty state handles the no-session
  // case with its "Press ⌘⇧B to add" hint, so we don't need to gate on
  // capabilities. Gating here would hide the panel from users who want
  // to set breakpoints before starting a session.
  applicableWhen: () => true,
  inactiveHint:
    'Set state-entry, transition-fire, action-invoke, constraint-violation, or threshold-crossing breakpoints to pause execution.',
  // Render ignores PanelProps — BreakpointsPanel manages its own data.
  render: () => createElement(BreakpointsPanel),
  expandedRender: () => createElement(BreakpointsPanel),
};
