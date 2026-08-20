/**
 * features/diagnostics/types — panel-local types for the Diagnostics panel (R6.1).
 *
 * Wire mirrors (Diagnostic / DiagnosticSeverity / DiagnosticSpan) live in
 * `engine/types.ts`; re-exported here so component files only import
 * `./types` and never touch the engine surface directly.
 *
 * Local types cover the UI filter state and the canonical severity lists
 * consumed by the filter controls — all UI-only, kept off the engine
 * barrel.
 */

import type {
  Diagnostic,
  DiagnosticSeverity,
  DiagnosticSpan,
} from '@/engine/types';

export type { Diagnostic, DiagnosticSeverity, DiagnosticSpan };

/** Scope toggle for the Diagnostics panel filter controls. */
export type DiagnosticsScope = 'current-file' | 'workspace';

/**
 * Per-severity on/off state for the Diagnostics panel filter controls.
 * Mirrors the four canonical `DiagnosticSeverity` values so future backend
 * additions (e.g., `"hint"`) slot in without a separate map.
 */
export type SeverityFilter = Record<DiagnosticSeverity, boolean>;

/**
 * Full filter state held by `<DiagnosticsPanel />` and consumed by the
 * pure `filterDiagnostics` helper. Each field is required in the UI so
 * the component never has to carry `undefined`s.
 */
export interface DiagnosticsFilter {
  /** Per-severity on/off mask. */
  severity: SeverityFilter;
  /** Free-text substring search (case-insensitive, matched against message). */
  search: string;
  /** Whether to restrict diagnostics to the active URI or show the whole workspace. */
  scope: DiagnosticsScope;
}

/**
 * Default filter used on panel mount — every severity on, empty search,
 * whole-workspace scope so a fresh panel shows every diagnostic loaded.
 */
export const DEFAULT_DIAGNOSTICS_FILTER: DiagnosticsFilter = {
  severity: { error: true, warning: true, info: true, hint: true },
  search: '',
  scope: 'workspace',
};

/**
 * Canonical ordered list of severities for the filter control. Kept in
 * one place so the Panel and tests stay in lock-step as new severities
 * land. Ordered highest-to-lowest severity.
 */
export const DIAGNOSTIC_SEVERITY_OPTIONS: readonly DiagnosticSeverity[] = [
  'error',
  'warning',
  'info',
  'hint',
] as const;

/** Human labels for the severity checkboxes. */
export const DIAGNOSTIC_SEVERITY_LABELS: Record<DiagnosticSeverity, string> = {
  error: 'Errors',
  warning: 'Warnings',
  info: 'Info',
  hint: 'Hints',
};

/**
 * Accent color per severity — kept separate from the panel's own accent
 * so the severity chip renders consistently regardless of the sidebar
 * host theme. 1:1 mapping onto the ninebar diagnostic-severity ladder
 * (`--severity-error` / `--severity-warning` / `--severity-info` /
 * `--severity-hint`) so the chip always matches the row it labels.
 */
export const DIAGNOSTIC_SEVERITY_COLORS: Record<DiagnosticSeverity, string> = {
  error: 'var(--severity-error)',
  warning: 'var(--severity-warning)',
  info: 'var(--severity-info)',
  hint: 'var(--severity-hint)',
};

/**
 * A rendered diagnostic row carries the wire diagnostic plus the parent
 * URI the backend returned it under — diagnostics have `span.file` but
 * span is optional, so the parent URI is the stable grouping key.
 */
export interface DiagnosticEntry {
  /** URI the diagnostic was fetched under (parent file). */
  uri: string;
  /** The wire diagnostic payload. */
  diagnostic: Diagnostic;
}
