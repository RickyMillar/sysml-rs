/**
 * Global Cmd-K / Ctrl-K keyboard shortcut hook.
 *
 * Platform detection: Cmd on macOS, Ctrl elsewhere. Escape closes regardless
 * of platform. The hook is a no-op when `enabled` is false, which lets the
 * caller gate the palette behind a dev-mode env flag without paying the
 * cost of a global listener in production.
 */

import { useEffect } from 'react';

function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  // `navigator.platform` is deprecated but still the most reliable synchronous
  // signal. `userAgentData` would be async and is not available in jsdom.
  const platform = navigator.platform ?? '';
  return /Mac|iPhone|iPad|iPod/i.test(platform);
}

export interface UseCmdKShortcutOptions {
  enabled: boolean;
  isOpen: boolean;
  onOpen: () => void;
  onClose: () => void;
}

export function useCmdKShortcut({
  enabled,
  isOpen,
  onOpen,
  onClose,
}: UseCmdKShortcutOptions): void {
  useEffect(() => {
    if (!enabled) return;
    const mac = isMacPlatform();

    const handler = (event: KeyboardEvent): void => {
      const key = event.key.toLowerCase();
      const modifier = mac ? event.metaKey : event.ctrlKey;

      if (modifier && key === 'k' && !event.altKey && !event.shiftKey) {
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

// Exported for tests (pure function, platform-independent for assertion).
export const __testables = { isMacPlatform };
