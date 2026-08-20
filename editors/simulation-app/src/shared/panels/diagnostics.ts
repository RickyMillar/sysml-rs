/**
 * diagnosticsPanel — PanelDescriptor for the Diagnostics sidebar (R6.1).
 *
 * Surfaced via `defaultPosition: 'utility'` alongside Breakpoints and
 * Archive in the shell drawer. Unlike workbench panels (Plots,
 * Constraints, …), the diagnostics surface isn't session-scoped — it
 * always reflects whatever the workspace currently has loaded — so
 * `applicableWhen` returns `true` and the panel's own empty-states
 * handle the "clean workspace" and "no matches" cases.
 *
 * Accent: was amber, historically distinguished from Variables' violet,
 * Breakpoints' red, and Archive's rose to keep the sidebar icons visually
 * separable at a glance. ninebar sweep: amber sits in the reserved accent
 * wedge (selection/active/primacy only), so accentColor now resolves to
 * --text-secondary like the other utility panels; the hex values this
 * comment once cited are historical and no longer live.
 */

import { createElement } from 'react';
import { DiagnosticsPanel } from '../../features/diagnostics/DiagnosticsPanel';
import type { PanelDescriptor } from './types';

export const diagnosticsPanel: PanelDescriptor = {
  id: 'diagnostics',
  title: 'Diagnostics',
  icon: 'bug_report',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  // Always applicable — the panel's empty states handle "no
  // diagnostics" and "no matches" without gating on capabilities.
  applicableWhen: () => true,
  inactiveHint:
    'Parse and semantic diagnostics appear here once a workspace is loaded.',
  // Self-contained — the DiagnosticsPanel pulls its data via react-query
  // from sysml.diagnostics and navigates via react-router.
  render: () => createElement(DiagnosticsPanel),
  expandedRender: () => createElement(DiagnosticsPanel),
};
