/**
 * useTheme — the app's light/dark toggle.
 *
 * The design system is fully token-driven: `:root` defines the dark
 * ("warm ink") ramp and `:root[data-theme='light']` the light ("warm
 * paper") ramp (see styles/tokens.css). Flipping the theme is therefore a
 * single attribute on the document root — every surface, border, and text
 * tier follows because components only ever read the semantic tokens.
 *
 * The choice persists in localStorage and is applied at module load so a
 * saved light theme paints on the first frame (no dark flash on reload).
 */
import { useCallback, useEffect, useState } from 'react';

export type ThemeName = 'dark' | 'light';

const STORAGE_KEY = 'ninebar-theme';

function readStored(): ThemeName {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

function applyTheme(theme: ThemeName): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-theme', theme);
  }
}

// Apply once at module load so the stored theme is live before first paint.
applyTheme(readStored());

export function useTheme() {
  const [theme, setTheme] = useState<ThemeName>(readStored);

  useEffect(() => {
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      /* storage unavailable (private mode) — the in-session toggle still works */
    }
  }, [theme]);

  const toggle = useCallback(() => {
    setTheme((current) => (current === 'dark' ? 'light' : 'dark'));
  }, []);

  return { theme, toggle };
}
