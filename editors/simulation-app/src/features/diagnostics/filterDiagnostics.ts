/**
 * filterDiagnostics — pure client-side narrowing helper for the
 * Diagnostics panel (R6.1).
 *
 * The backend always returns every diagnostic for a requested URI; the
 * panel combines per-severity checkboxes, a case-insensitive substring
 * search, and a current-file vs. workspace scope toggle on top of that
 * list. Keeping the filter pure makes every combination trivially
 * unit-testable.
 *
 * The helper works on `DiagnosticEntry` (URI + diagnostic tuple) so the
 * panel can mix diagnostics from multiple files in one list and narrow
 * them with a single pass.
 */

import type { DiagnosticEntry, DiagnosticsFilter } from './types';

export interface FilterDiagnosticsOpts extends DiagnosticsFilter {
  /**
   * URI treated as the "current" file for the `current-file` scope.
   * When the scope is `current-file` and `activeUri` is `null`, no
   * entries pass the scope gate — the panel then shows the "no matches"
   * empty state rather than silently falling back to workspace-wide.
   */
  activeUri: string | null;
}

/**
 * Pure filter — takes the fetched diagnostic entries plus the panel
 * filter state and returns only the entries the UI should render.
 * Input array is never mutated.
 */
export function filterDiagnostics(
  entries: DiagnosticEntry[],
  opts: FilterDiagnosticsOpts,
): DiagnosticEntry[] {
  const needle = opts.search.trim().toLowerCase();
  const scopeUri = opts.scope === 'current-file' ? opts.activeUri : null;

  return entries.filter((entry) => {
    // Scope gate — current-file scope narrows to the active URI; the
    // diagnostic's `span.file` may differ from the fetched URI (cross-
    // file semantic errors), so we also accept matches where the span's
    // file equals the active URI.
    if (opts.scope === 'current-file') {
      if (scopeUri === null) return false;
      const spanFile = entry.diagnostic.span?.file;
      if (entry.uri !== scopeUri && spanFile !== scopeUri) return false;
    }

    // Severity gate — the filter mask has one flag per severity; a
    // severity that is off removes every diagnostic of that kind.
    if (!opts.severity[entry.diagnostic.severity]) return false;

    // Substring search — case-insensitive match against the message.
    // Also matches against the code when present so `"E001"` narrows to
    // that code family without forcing the user to remember the copy.
    if (needle.length > 0) {
      const hay =
        entry.diagnostic.message.toLowerCase() +
        ' ' +
        (entry.diagnostic.code ?? '').toLowerCase();
      if (!hay.includes(needle)) return false;
    }

    return true;
  });
}

/**
 * Group diagnostic entries by URI in insertion order. Used by the panel
 * to render collapsible per-file groups with count badges.
 *
 * Returns a `Map` so callers iterate deterministically without sorting
 * entries by URI (the fetch layer provides the canonical ordering).
 */
export function groupDiagnosticsByUri(
  entries: DiagnosticEntry[],
): Map<string, DiagnosticEntry[]> {
  const groups = new Map<string, DiagnosticEntry[]>();
  for (const entry of entries) {
    const existing = groups.get(entry.uri);
    if (existing) {
      existing.push(entry);
    } else {
      groups.set(entry.uri, [entry]);
    }
  }
  return groups;
}

/**
 * Compute a short human-readable location suffix (`"line:col"`) for a
 * diagnostic span. Falls back to byte-offset when line/col are absent
 * so parse errors without line numbers still surface a locator.
 */
export function formatSpanLocation(
  span: { line?: number; col?: number; start: number; end: number } | undefined,
): string | null {
  if (!span) return null;
  if (span.line != null) {
    return span.col != null ? `${span.line}:${span.col}` : `${span.line}`;
  }
  return `@${span.start}`;
}
