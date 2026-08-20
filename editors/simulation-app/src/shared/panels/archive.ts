/**
 * archivePanel — PanelDescriptor for the session archive sidebar (R4.1).
 *
 * Surfaced via `defaultPosition: 'utility'` in the shell drawer.
 * Unlike workbench panels (Plots, Constraints, …), the archive isn't
 * session-scoped — it always shows every run the
 * workspace has ever produced, so `applicableWhen` returns `true` and
 * the panel's own empty-state handles the "nothing archived yet" case.
 *
 * Accent: rose (distinct from Variables' violet and Breakpoints' red),
 * picked to keep the sidebar icons visually separable at a glance.
 * ninebar sweep: accentColor now resolves to --text-secondary (utility
 * panels no longer carry a per-panel hue); the rose/violet/red hex this
 * comment once cited are historical and no longer live values.
 */

import { createElement } from 'react';
import { ArchivePanel } from '../../features/archive/ArchivePanel';
import type { PanelDescriptor } from './types';

export const archivePanel: PanelDescriptor = {
  id: 'archive',
  title: 'Archive',
  icon: 'archive',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  // Always applicable — the panel's empty-state handles "no archived
  // sessions" and the filter empty-state handles "filters match nothing".
  applicableWhen: () => true,
  inactiveHint:
    'Archived sessions appear here once a run, verify, or compare session completes.',
  // Self-contained — the ArchivePanel pulls its data via react-query
  // from the backend archive commands and navigates via react-router.
  render: () => createElement(ArchivePanel),
  expandedRender: () => createElement(ArchivePanel),
};
