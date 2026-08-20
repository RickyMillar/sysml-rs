/**
 * Client-side feature flags for the simulation app.
 *
 * Flags are off by default. Enable for local dev via either:
 *   - `localStorage.setItem('sysml.flag.expressionView', '1')` (persists)
 *   - `window.__sysmlFlags.expressionView = true` (runtime override, non-persistent)
 *   - `?flag=expressionView` query string (one-shot; also persists to localStorage)
 *
 * Keep this file tiny and dependency-free — it's imported by cards and tests.
 */

const STORAGE_PREFIX = 'sysml.flag.';

declare global {
  interface Window {
    __sysmlFlags?: Record<string, boolean>;
  }
}

function readRuntimeFlag(name: string): boolean | null {
  if (typeof window === 'undefined') return null;
  const runtime = window.__sysmlFlags;
  if (runtime && typeof runtime[name] === 'boolean') return runtime[name]!;
  return null;
}

/**
 * Flags that are ON by default (the ninebar FLIP, Phase 3 gate + Phase
 * 1.5 gate both passed 2026-07-14). Opt OUT with an explicit stored
 * `'0'`/`'false'` (`localStorage['sysml.flag.ninebar']='0'` — the
 * legacy Playwright projects pin this via `storageState`) or a runtime
 * `window.__sysmlFlags.ninebar = false`. The flag itself is deleted in
 * Phase 8 along with the legacy shell.
 */
const DEFAULT_ON = new Set(['ninebar']);

function readStoredFlag(name: string): boolean {
  const fallback = DEFAULT_ON.has(name);
  if (typeof localStorage === 'undefined') return fallback;
  try {
    const raw = localStorage.getItem(`${STORAGE_PREFIX}${name}`);
    if (raw === '1' || raw === 'true') return true;
    if (raw === '0' || raw === 'false') return false;
    return fallback;
  } catch {
    return fallback;
  }
}

/** Promote `?flag=foo[,bar]` from the URL into localStorage so reloads keep
 *  the flag on. `?flag=!foo` (or `foo=0`) stores an explicit opt-OUT. */
function promoteQueryFlags(): void {
  if (typeof window === 'undefined') return;
  try {
    const params = new URLSearchParams(window.location.search);
    const flagList = params.get('flag') ?? params.get('flags');
    if (!flagList) return;
    for (const f of flagList.split(',').map((s) => s.trim()).filter(Boolean)) {
      if (f.startsWith('!')) {
        localStorage.setItem(`${STORAGE_PREFIX}${f.slice(1)}`, '0');
      } else if (f.endsWith('=0')) {
        localStorage.setItem(`${STORAGE_PREFIX}${f.slice(0, -2)}`, '0');
      } else {
        localStorage.setItem(`${STORAGE_PREFIX}${f}`, '1');
      }
    }
  } catch {
    /* noop */
  }
}

promoteQueryFlags();

export function isFlagEnabled(name: string): boolean {
  const runtime = readRuntimeFlag(name);
  if (runtime !== null) return runtime;
  return readStoredFlag(name);
}

/**
 * EXPRESSION_VIEW_ENABLED — gates KaTeX rendering of constraint/equation
 * expressions via `@sysml-rs/expression-view`. ON by default as of Day 6
 * (results workbench is live). Disable with
 * `localStorage.setItem('sysml.flag.expressionView', '0')` or
 * `window.__sysmlFlags.expressionView = false`.
 */
export const EXPRESSION_VIEW_ENABLED = (): boolean => {
  // Check for explicit opt-out first
  if (typeof window !== 'undefined') {
    const runtime = window.__sysmlFlags;
    if (runtime && typeof runtime.expressionView === 'boolean') return runtime.expressionView;
  }
  if (typeof localStorage !== 'undefined') {
    try {
      const raw = localStorage.getItem('sysml.flag.expressionView');
      if (raw === '0' || raw === 'false') return false;
    } catch { /* noop */ }
  }
  return true; // default ON
};
