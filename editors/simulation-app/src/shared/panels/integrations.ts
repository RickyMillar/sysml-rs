/**
 * integrationsPanel — Phase 7 PanelDescriptor for the Integrations
 * utility drawer (MCP / REST / LSP connection details).
 *
 * Always registered + always surfaced (unlike `debugPanel`, which is
 * gated behind `VITE_DEBUG_DRAWER`). This is a product feature, not
 * a dev affordance.
 */

import { createElement } from 'react';
import { IntegrationsPanel } from '../../features/utilities/IntegrationsPanel';
import type { PanelDescriptor } from './types';

export const integrationsPanel: PanelDescriptor = {
  id: 'integrations',
  title: 'Integrations',
  icon: 'hub',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'utility',
  applicableWhen: () => true,
  render: () => createElement(IntegrationsPanel),
};
