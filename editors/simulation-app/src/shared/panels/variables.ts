/**
 * variablesPanel — PanelDescriptor for the R2.2 Variables pane.
 *
 * The legacy Variables pane used to sit on the right sidebar and show
 * every variable in the active session snapshot. Those jobs moved into
 * the model tree and Results Workbench; this descriptor remains hidden
 * for compatibility with direct imports.
 *
 * Registered in `./registry.ts` alongside the workbench panels. The
 * registry filters by `defaultPosition` so ResultsWorkbench skips it.
 */

import { createElement, type ComponentType } from 'react';
import { VariablesPane, type VariablesPaneProps } from '@/features/variables/VariablesPane';
import type { PanelDescriptor } from './types';

const BOOK_BASE = 'https://www.omg.org/spec/SysML/';

export const variablesPanel: PanelDescriptor = {
  id: 'variables',
  title: 'Variables',
  icon: 'data_object',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  inactiveHint: 'Variables appear once a session is running.',
  learnUrl: `${BOOK_BASE}/attributes`,
  // Phase B7: the right-hand Variables pane is superseded by the new
  // in-tree attribute rows in SessionTreeV2 (list / filter / pin /
  // live values all live per-node now). Keep the descriptor so consumers
  // that import it still compile, but hide it from the sidebar.
  applicableWhen: () => false,
  render: () => createElement(VariablesPane),
  expandedRender: () =>
    createElement(VariablesPane as ComponentType<VariablesPaneProps>, { expanded: true }),
};
