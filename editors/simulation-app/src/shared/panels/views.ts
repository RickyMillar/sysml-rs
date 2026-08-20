/**
 * viewsPanel — PanelDescriptor for the authored-views sidebar
 * (Phase 5).
 *
 * Surfaces the user-authored ViewUsage / ViewDefinition list discovered
 * by the backend's `sysml.query` command. Click a row → backend
 * renders that view via `sysml.views.render` and the diagram pane
 * follows.
 *
 * `defaultPosition: 'utility'` puts it alongside Diagnostics, Archive,
 * and Breakpoints — the `UtilityDrawer` top bar (above the diagram canvas)
 * pins all utility panels into a togglable side-drawer.
 *
 * Accent: was cyan, historically distinct from the existing utility
 * panels' palette. ninebar sweep: accentColor now resolves to
 * --text-secondary like the other utility panels; the cyan hex this
 * comment once cited is historical and no longer a live value.
 */

import { createElement } from 'react';
import { ViewsPanel } from '../../features/views/ViewsPanel';
import type { PanelDescriptor } from './types';

export const viewsPanel: PanelDescriptor = {
  id: 'views',
  title: 'Views',
  icon: 'visibility',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  // Always applicable — empty workspaces just show the empty state.
  applicableWhen: () => true,
  inactiveHint:
    'User-authored ViewUsage / ViewDefinition declarations appear here once a workspace is loaded.',
  render: () => createElement(ViewsPanel),
  expandedRender: () => createElement(ViewsPanel),
};
