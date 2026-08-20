/**
 * Tests for the hotkey resolver + platform hints used by
 * WorkflowSwitcher. The DOM rendering is exercised end-to-end by
 * Playwright (smoke + integration); these unit tests lock in the
 * pure-logic bits so we can iterate on them safely.
 */

import { describe, it, expect } from 'vitest';
import {
  modifierLabel,
  resolveHotkey,
} from '@/workflows/ui/WorkflowSwitcher';

function keyEvent(
  key: string,
  opts: Partial<{ metaKey: boolean; altKey: boolean; ctrlKey: boolean }> = {},
) {
  return {
    metaKey: false,
    altKey: false,
    ctrlKey: false,
    key,
    ...opts,
  };
}

describe('modifierLabel', () => {
  it('is ⌘ on mac, Alt elsewhere', () => {
    expect(modifierLabel(true)).toBe('⌘');
    expect(modifierLabel(false)).toBe('Alt');
  });
});

describe('resolveHotkey — mac (⌘+digit)', () => {
  it('resolves ⌘+1 to the Run workflow', () => {
    const intent = resolveHotkey(keyEvent('1', { metaKey: true }), true);
    expect(intent?.path).toBe('/run');
    expect(intent?.workflowId).toBe('session');
  });

  it('resolves ⌘+2 → /verify', () => {
    const intent = resolveHotkey(keyEvent('2', { metaKey: true }), true);
    expect(intent?.path).toBe('/verify');
  });

  it('resolves ⌘+3 → /analyze', () => {
    const intent = resolveHotkey(keyEvent('3', { metaKey: true }), true);
    expect(intent?.path).toBe('/analyze');
    expect(intent?.workflowId).toBe('analyze');
  });

  it('⌘+4 is unbound — Compare gave up its slot on demotion (Phase 6)', () => {
    // The freed front-door binding belongs to the Phase 8 switcher
    // redesign; until then digit 4 behaves like the unmapped 5–9.
    const intent = resolveHotkey(keyEvent('4', { metaKey: true }), true);
    expect(intent?.path).toBeNull();
    expect(intent?.workflowId).toBeNull();
  });

  it('ignores plain digit press (no modifier)', () => {
    expect(resolveHotkey(keyEvent('1'), true)).toBeNull();
  });

  it('ignores Alt+1 on mac (wrong modifier)', () => {
    expect(resolveHotkey(keyEvent('1', { altKey: true }), true)).toBeNull();
  });

  it('ignores non-digit keys', () => {
    expect(resolveHotkey(keyEvent('a', { metaKey: true }), true)).toBeNull();
  });

  it('returns null-path for digits without a mapping (5–9)', () => {
    const intent = resolveHotkey(keyEvent('9', { metaKey: true }), true);
    expect(intent?.path).toBeNull();
    expect(intent?.workflowId).toBeNull();
  });
});

describe('resolveHotkey — non-mac (Alt+digit)', () => {
  it('resolves Alt+1 to /run', () => {
    const intent = resolveHotkey(keyEvent('1', { altKey: true }), false);
    expect(intent?.path).toBe('/run');
  });

  it('ignores Meta+1 on non-mac', () => {
    expect(resolveHotkey(keyEvent('1', { metaKey: true }), false)).toBeNull();
  });

  it('declines when Ctrl is also held (leaves Ctrl+Alt+<digit> to the OS)', () => {
    const intent = resolveHotkey(
      keyEvent('1', { altKey: true, ctrlKey: true }),
      false,
    );
    expect(intent).toBeNull();
  });
});
