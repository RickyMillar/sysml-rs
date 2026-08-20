/**
 * BreakpointsPanel — toast message + panel descriptor contract tests.
 *
 * The panel's interactive behaviour (add dialog flow, flash, toast
 * dismiss) is covered by the Playwright suite. Here we pin the pure
 * pieces: toast-message derivation from a `breakpoint-hit` context and
 * the `breakpointsPanel` descriptor shape registered in the panel
 * registry.
 */

import { describe, expect, it } from 'vitest';
import { toastMessage, buildToastText } from '../BreakpointsPanel';
import type { BreakpointLocal } from '../useBreakpointStore';
import { breakpointsPanel } from '@/shared/panels/breakpoints';
import { panelRegistry, findPanel } from '@/shared/panels/registry';

describe('toastMessage', () => {
  it('falls back to a generic message when context is empty', () => {
    expect(toastMessage(undefined)).toBe('Paused at a breakpoint');
  });

  it('reads target when present', () => {
    expect(toastMessage({ target: 'Engine.On' })).toBe('Paused at Engine.On');
  });

  it('prefers state > transition > breakpointId when no explicit target is set', () => {
    expect(toastMessage({ state: 'Heating' })).toBe('Paused at Heating');
    expect(toastMessage({ transition: 'T1' })).toBe('Paused at T1');
    expect(toastMessage({ breakpointId: 'bk-42' })).toBe('Paused at bp-bk-42');
  });

  it('handles non-string fields gracefully', () => {
    expect(toastMessage({ target: 42 })).toBe('Paused at a breakpoint');
  });
});

// BP5: `buildToastText` prefers the fired breakpoint's local entry
// (looked up by id) over the raw event context, so the toast shows a
// human label ("when `i_drive` > 5") instead of a bare BreakpointId.
describe('buildToastText', () => {
  const conditionalEntry: BreakpointLocal = {
    id: 'bp-1',
    breakpoint: {
      kind: 'conditional',
      target: 'circuit1',
      variable: 'i_drive',
      op: 'gt',
      value: 5,
    },
  };

  it('renders the matched local entry label', () => {
    expect(
      buildToastText({ id: 'bp-1', hitAtMs: 0 }, [conditionalEntry]),
    ).toBe('Paused at when `circuit1.i_drive` > 5');
  });

  it('falls back to toastMessage when no local entry matches', () => {
    expect(
      buildToastText(
        { id: 'bp-unknown', hitAtMs: 0, context: { target: 'Engine.On' } },
        [conditionalEntry],
      ),
    ).toBe('Paused at Engine.On');
  });
});

describe('breakpointsPanel descriptor', () => {
  it('is registered in the panel registry', () => {
    expect(panelRegistry).toContain(breakpointsPanel);
    expect(findPanel('breakpoints')).toBe(breakpointsPanel);
  });

  it('carries the expected metadata', () => {
    expect(breakpointsPanel.id).toBe('breakpoints');
    expect(breakpointsPanel.title).toBe('Breakpoints');
    expect(breakpointsPanel.icon).toBe('radio_button_checked');
    expect(breakpointsPanel.defaultPosition).toBe('utility');
  });

  it('is always applicable (empty state covers no-session case)', () => {
    const caps = {} as Parameters<typeof breakpointsPanel.applicableWhen>[0];
    const session = {} as Parameters<typeof breakpointsPanel.applicableWhen>[1];
    expect(breakpointsPanel.applicableWhen(caps, session)).toBe(true);
  });

  it('has a render function (returns a React element)', () => {
    expect(typeof breakpointsPanel.render).toBe('function');
    const rendered = breakpointsPanel.render({} as never);
    expect(rendered).toBeDefined();
  });
});
