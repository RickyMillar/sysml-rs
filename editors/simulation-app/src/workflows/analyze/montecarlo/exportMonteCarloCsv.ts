/**
 * CSV export for Monte Carlo iteration batches (R5.9).
 *
 * Pure function — no DOM, no side-effects. The matching React wrapper
 * lives in `DownloadCsvButton.tsx` and handles the blob/anchor plumbing.
 *
 * Format:
 *   - RFC 4180 CRLF line endings ("\r\n")
 *   - Double-quote wrapping only when a field contains `,`, `"`, `\r`
 *     or `\n`; embedded `"` is escaped to `""`
 *   - Header row: `iteration, session_id, status, <params…>, <metrics…>, verdict_overall`
 *   - Columns are the UNION of every child's params / metrics, sorted
 *     alphabetically for determinism (agents must agree even when
 *     children arrive out of order)
 *   - Rows are emitted in ascending `child.index` order (NOT arrival
 *     order — mirrors the dashboard's canonical iteration order)
 *   - Missing cells are blank (the empty string)
 *   - `verdict_overall` is `pass` when every child verdict is `pass`;
 *     `fail` when any verdict is `fail`; `error` when any verdict is
 *     `error` (and no `fail`); `inconclusive` when any verdict is
 *     `inconclusive` (and no fail/error); blank when no verdicts were
 *     emitted for the child (e.g. pending iteration)
 */

import type { Value, VerdictKind } from '../../../engine/types';
import type { ChildDescriptor } from './passRateHelpers';

/** RFC 4180 line terminator. */
const CRLF = '\r\n';

/**
 * Escape a single CSV field per RFC 4180. Returns the original string
 * when no quoting is required — keeps small cells (like the numeric
 * iteration index) readable.
 */
export function escapeCsvField(raw: string): string {
  if (raw.length === 0) return '';
  const needsQuote =
    raw.includes(',') ||
    raw.includes('"') ||
    raw.includes('\n') ||
    raw.includes('\r');
  if (!needsQuote) return raw;
  const escaped = raw.replace(/"/g, '""');
  return `"${escaped}"`;
}

/**
 * Render a `Value` as the CSV cell string. Objects and arrays are
 * serialised through `JSON.stringify` so the CSV stays grep-able; UI
 * consumers that want richer rendering can do so on top of the raw
 * child descriptor instead of the exported CSV.
 */
function valueToCell(v: Value | undefined | null): string {
  if (v === undefined || v === null) return '';
  switch (typeof v) {
    case 'string':
      return v;
    case 'number':
      // Non-finite values become blanks so spreadsheet parsers don't
      // choke on `Infinity` / `NaN`.
      return Number.isFinite(v) ? String(v) : '';
    case 'boolean':
      return v ? 'true' : 'false';
    default:
      try {
        return JSON.stringify(v);
      } catch {
        return '';
      }
  }
}

/**
 * Priority order when rolling up verdicts for the overall column. `fail`
 * dominates `error` dominates `inconclusive` dominates `pass` — matches
 * the SysML v2 VerificationCase folding semantics.
 */
const VERDICT_PRIORITY: Record<VerdictKind, number> = {
  pass: 0,
  inconclusive: 1,
  error: 2,
  fail: 3,
};

function overallVerdict(child: ChildDescriptor): VerdictKind | '' {
  if (!child.verdicts || child.verdicts.length === 0) return '';
  let best: VerdictKind = 'pass';
  for (const v of child.verdicts) {
    if (VERDICT_PRIORITY[v.verdict] > VERDICT_PRIORITY[best]) {
      best = v.verdict;
    }
  }
  return best;
}

/**
 * Collect the union of column names from a record-of-Values across
 * children. Returned sorted alphabetically so ordering is deterministic
 * regardless of child arrival order.
 */
function collectColumns(
  children: ChildDescriptor[],
  pick: (c: ChildDescriptor) => Record<string, Value> | undefined,
): string[] {
  const set = new Set<string>();
  for (const c of children) {
    const rec = pick(c);
    if (!rec) continue;
    for (const k of Object.keys(rec)) set.add(k);
  }
  return [...set].sort();
}

/**
 * Build the CSV document. Always returns at least a header row — empty
 * batches still produce a usable file (header-only) so downstream
 * tooling doesn't crash on zero rows.
 */
export function exportMonteCarloCsv(children: ChildDescriptor[]): string {
  // Sort ascending by iteration index for deterministic row order.
  const sorted = [...children].sort((a, b) => a.index - b.index);
  const paramCols = collectColumns(sorted, (c) => c.params);
  const metricCols = collectColumns(sorted, (c) => c.metrics);

  const header = [
    'iteration',
    'session_id',
    'status',
    ...paramCols,
    ...metricCols,
    'verdict_overall',
  ];

  const lines: string[] = [header.map(escapeCsvField).join(',')];
  for (const child of sorted) {
    const row: string[] = [
      String(child.index),
      child.session_id ?? '',
      child.status,
    ];
    for (const k of paramCols) {
      row.push(valueToCell(child.params?.[k]));
    }
    for (const k of metricCols) {
      row.push(valueToCell(child.metrics?.[k]));
    }
    row.push(overallVerdict(child));
    lines.push(row.map(escapeCsvField).join(','));
  }
  // RFC 4180 recommends CRLF, including a final terminator on the last
  // record. Splitting on "\r\n" round-trips cleanly through Excel and
  // Google Sheets.
  return lines.join(CRLF) + CRLF;
}

/**
 * Build the filename used by the download button. Pure so tests can
 * snapshot against a fixed clock.
 *
 * Shape: `monte-carlo-<batchId>-<YYYYMMDD-HHMMSS>.csv`
 */
export function monteCarloCsvFilename(batchId: string, now: Date = new Date()): string {
  const pad2 = (n: number) => String(n).padStart(2, '0');
  const y = now.getFullYear();
  const m = pad2(now.getMonth() + 1);
  const d = pad2(now.getDate());
  const hh = pad2(now.getHours());
  const mm = pad2(now.getMinutes());
  const ss = pad2(now.getSeconds());
  const sanitizedBatch = batchId.replace(/[^A-Za-z0-9._-]+/g, '-') || 'batch';
  return `monte-carlo-${sanitizedBatch}-${y}${m}${d}-${hh}${mm}${ss}.csv`;
}

export const __internals = { escapeCsvField, valueToCell, overallVerdict, collectColumns };
