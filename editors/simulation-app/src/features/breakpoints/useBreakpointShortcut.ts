/**
 * ⌘⇧B / Ctrl⇧B shortcut hook — opens the Add Breakpoint dialog.
 *
 * Mirrors the structure of `features/command-palette/useCmdKShortcut` so
 * the two shortcuts behave identically (platform-aware modifier, no-op
 * when disabled, Esc closes).
 */

import { useEffect } from 'react';

function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  const platform = navigator.platform ?? '';
  return /Mac|iPhone|iPad|iPod/i.test(platform);
}

export interface UseBreakpointShortcutOptions {
  enabled: boolean;
  isOpen: boolean;
  onOpen: () => void;
  onClose: () => void;
}

/**
 * Register a global keydown listener for ⌘⇧B (Mac) / Ctrl⇧B (other).
 * Esc closes the dialog regardless of platform.
 *
 * The handler ignores presses while the user is typing in a text field
 * (input / textarea / contenteditable) so a `B` inside a variable name
 * search doesn't accidentally trigger the shortcut.
 */
export function useBreakpointShortcut({
  enabled,
  isOpen,
  onOpen,
  onClose,
}: UseBreakpointShortcutOptions): void {
  useEffect(() => {
    if (!enabled) return;
    const mac = isMacPlatform();

    const handler = (event: KeyboardEvent): void => {
      // Skip when focus is inside an editable field — the user is typing,
      // not issuing a shortcut.
      const target = event.target as HTMLElement | null;
      if (target) {
        const tag = target.tagName;
        const typing =
          tag === 'INPUT' ||
          tag === 'TEXTAREA' ||
          tag === 'SELECT' ||
          target.isContentEditable;
        // Allow Esc inside the Add dialog's fields, but nothing else.
        if (typing && !(isOpen && event.key === 'Escape')) {
          // We still want to allow the Cmd/Ctrl-Shift-B combo even in
          // fields so the user can pull the dialog up from anywhere.
          const modifier = mac ? event.metaKey : event.ctrlKey;
          const comboKey = event.key.toLowerCase() === 'b';
          if (!(modifier && event.shiftKey && comboKey)) return;
        }
      }

      const key = event.key.toLowerCase();
      const modifier = mac ? event.metaKey : event.ctrlKey;

      if (modifier && event.shiftKey && key === 'b' && !event.altKey) {
        event.preventDefault();
        if (isOpen) {
          onClose();
        } else {
          onOpen();
        }
        return;
      }

      if (isOpen && event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };

    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [enabled, isOpen, onOpen, onClose]);
}

// Exported for tests — the platform helper is pure and trivially mockable.
export const __testables = { isMacPlatform };
