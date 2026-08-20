/**
 * Report template — CSS tokens, badge rendering, and small HTML
 * helpers used by `generateHtmlReport.ts`.
 *
 * Everything here is pure string templating. No React, no DOM APIs.
 * Safe to import from Node tooling.
 *
 * Style notes:
 *   - Engineering-Atelier palette mirrors `src/styles/tokens.css`,
 *     but light theme is the default because these files are commonly
 *     printed / archived in PDFs. A `[data-theme='dark']` toggle on
 *     `<html>` switches to dark. The print-friendly button also sets
 *     `.report--print` which clamps to a tighter layout.
 *   - Colours are emitted in OKLCH with a hex fallback so old print
 *     renderers (wkhtmltopdf, some Safari prints) still get the right
 *     verdict colour.
 *   - No external fonts. System stack only.
 */

import type { VerdictKind } from '@/engine/types';

// ── Verdict token parity with VerdictBadge.tsx ─────────────────────────
//
// Kept in lockstep with `components/VerdictBadge.tsx` so a printed report
// is visually identical to the web view. Colours are the same hex values
// VerdictBadge emits; OKLCH equivalents are provided for modern
// browsers (and fall back to hex for print).

export interface VerdictTokens {
  hex: string;
  oklch: string;
  glyph: string;
  shape: string;
  label: string;
}

export const VERDICT_TOKENS: Record<VerdictKind, VerdictTokens> = {
  pass: {
    hex: '#10b981',
    oklch: 'oklch(73% 0.15 163)',
    glyph: '\u2713', // ✓
    shape: 'check',
    label: 'Pass',
  },
  fail: {
    hex: '#ef4444',
    oklch: 'oklch(64% 0.23 25)',
    glyph: '\u2717', // ✗
    shape: 'cross',
    label: 'Fail',
  },
  inconclusive: {
    hex: '#f59e0b',
    oklch: 'oklch(76% 0.16 75)',
    glyph: '?',
    shape: 'question',
    label: 'Inconclusive',
  },
  error: {
    hex: '#d946ef',
    oklch: 'oklch(66% 0.27 320)',
    glyph: '\u26A0', // ⚠ (triangle)
    shape: 'triangle',
    label: 'Error',
  },
};

/**
 * Escape text for safe HTML interpolation. Intentionally tiny — only
 * the five characters that matter inside element bodies and attributes.
 */
export function escapeHtml(value: unknown): string {
  if (value === null || value === undefined) return '';
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Render a single verdict badge as inline HTML. The shape glyph is
 * duplicated into `data-verdict-shape` so the printed version remains
 * colour-blind accessible (the glyph is the primary signal).
 */
export function renderBadge(kind: VerdictKind, opts: { compact?: boolean } = {}): string {
  const t = VERDICT_TOKENS[kind];
  const cls = opts.compact ? 'rpt-badge rpt-badge--compact' : 'rpt-badge';
  return (
    `<span class="${cls}" data-verdict="${kind}" data-verdict-shape="${t.shape}" ` +
    `style="--badge-color: ${t.hex}; --badge-color-oklch: ${t.oklch};" ` +
    `title="${escapeHtml(t.label)}">` +
    `<span class="rpt-badge__glyph" aria-hidden="true">${t.glyph}</span>` +
    `<span class="rpt-badge__label">${escapeHtml(t.label)}</span>` +
    `</span>`
  );
}

/**
 * Format a timestamp as `YYYY-MM-DD HH:MM:SS UTC` — deterministic
 * output for snapshot tests. Uses UTC so reports are reproducible
 * regardless of the machine that generated them.
 */
export function formatTimestamp(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  const y = date.getUTCFullYear();
  const m = pad(date.getUTCMonth() + 1);
  const d = pad(date.getUTCDate());
  const H = pad(date.getUTCHours());
  const M = pad(date.getUTCMinutes());
  const S = pad(date.getUTCSeconds());
  return `${y}-${m}-${d} ${H}:${M}:${S} UTC`;
}

/**
 * Format the timestamp portion of the filename. Uses UTC so filenames
 * sort chronologically regardless of the generating machine's zone.
 */
export function formatFilenameStamp(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return (
    `${date.getUTCFullYear()}${pad(date.getUTCMonth() + 1)}${pad(date.getUTCDate())}-` +
    `${pad(date.getUTCHours())}${pad(date.getUTCMinutes())}${pad(date.getUTCSeconds())}`
  );
}

/**
 * Slugify a workspace name for safe use in a filename. Lowercased;
 * non-alphanumerics collapsed to `-`; trimmed of leading/trailing
 * dashes. Empty input yields `"workspace"`.
 */
export function slugifyWorkspace(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'workspace';
}

/**
 * Format a duration in milliseconds as a human-readable string.
 * 999ms → "0.999 s", 65000ms → "1m 5s", 3_600_000ms → "1h 0m".
 */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${ms} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(3)} s`;
  const minutes = Math.floor(seconds / 60);
  const remSec = Math.floor(seconds - minutes * 60);
  if (minutes < 60) return `${minutes}m ${remSec}s`;
  const hours = Math.floor(minutes / 60);
  const remMin = minutes - hours * 60;
  return `${hours}h ${remMin}m`;
}

/**
 * Stringify a `Value` (see engine/types.ts) for display in the table.
 * Mirrors what the Variables pane does — numbers get tabular formatting
 * (trailing zeros trimmed), strings pass through, booleans become
 * literal `true`/`false`, nulls become `—`, objects/arrays fall back to
 * compact JSON.
 */
export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return '—';
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) return String(value); // Infinity / NaN
    // Use up to 6 significant digits, trim trailing zeros.
    const s = value.toPrecision(6);
    // toPrecision may yield scientific notation; prefer simpler form if
    // the number is in a reasonable range.
    if (Math.abs(value) >= 1e-4 && Math.abs(value) < 1e15) {
      const plain = Number(s).toString();
      return plain;
    }
    return s;
  }
  if (typeof value === 'boolean') return String(value);
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/**
 * The single embedded stylesheet. Kept in one string so HTML output
 * is truly standalone (no external refs). All palette values are token
 * mirrors of `src/styles/tokens.css` — when the app tokens change, this
 * should be updated in lockstep.
 */
export const REPORT_STYLESHEET: string = `
:root {
  /* Light surfaces — default because reports are printed. */
  --rpt-bg:              #f8fafc;
  --rpt-surface:         #ffffff;
  --rpt-surface-alt:     #f1f5f9;
  --rpt-border:          #e2e8f0;
  --rpt-text:            #0f172a;
  --rpt-text-muted:      #475569;
  --rpt-text-subtle:     #94a3b8;
  --rpt-accent:          oklch(62% 0.16 265);
  --rpt-accent-hex:      #6366f1;
  --rpt-shadow:          0 1px 2px rgba(15,23,42,0.06), 0 1px 3px rgba(15,23,42,0.08);
  --rpt-mono:            ui-monospace, 'JetBrains Mono', 'Fira Code', 'SF Mono', Menlo, monospace;
  --rpt-sans:            system-ui, -apple-system, 'Segoe UI', Roboto, 'Inter', sans-serif;
}
[data-theme='dark'] {
  --rpt-bg:              #0d131f;
  --rpt-surface:         #19202b;
  --rpt-surface-alt:     #151c27;
  --rpt-border:          #2f3541;
  --rpt-text:            #dde2f2;
  --rpt-text-muted:      #c6c5d5;
  --rpt-text-subtle:     #908f9e;
  --rpt-accent:          oklch(78% 0.10 285);
  --rpt-accent-hex:      #bdc2ff;
  --rpt-shadow:          0 1px 2px rgba(0,0,0,0.4), 0 1px 3px rgba(0,0,0,0.5);
}
* { box-sizing: border-box; }
html, body {
  margin: 0;
  padding: 0;
  background: var(--rpt-bg);
  color: var(--rpt-text);
  font-family: var(--rpt-sans);
  font-size: 14px;
  line-height: 1.45;
  font-feature-settings: 'tnum' 1, 'ss01' 1;
}
main.report {
  max-width: 1200px;
  margin: 0 auto;
  padding: 24px;
}
.rpt-header {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 16px;
  padding: 20px 24px;
  border-radius: 10px;
  background: var(--rpt-surface);
  box-shadow: var(--rpt-shadow);
  border: 1px solid var(--rpt-border);
  margin-bottom: 20px;
}
.rpt-header h1 {
  margin: 0 0 4px 0;
  font-size: 20px;
  font-weight: 600;
  letter-spacing: -0.01em;
}
.rpt-header__meta {
  display: flex;
  gap: 16px;
  color: var(--rpt-text-muted);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}
.rpt-header__meta span strong {
  color: var(--rpt-text);
  font-weight: 500;
}
.rpt-overall {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  border-radius: 8px;
  background: var(--rpt-surface-alt);
  border: 1px solid var(--rpt-border);
}
.rpt-overall__badge {
  font-size: 24px;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: 8px;
  color: var(--badge-color-oklch, var(--badge-color, #0f172a));
  background: color-mix(in srgb, var(--badge-color, #0f172a) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--badge-color, #0f172a) 30%, transparent);
  font-weight: 600;
}
.rpt-overall__counts {
  display: flex;
  gap: 10px;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.rpt-overall__counts span { color: var(--rpt-text-muted); }
.rpt-overall__counts strong { color: var(--rpt-text); font-weight: 600; }

.rpt-section {
  margin-bottom: 20px;
  padding: 16px 20px;
  background: var(--rpt-surface);
  border-radius: 10px;
  border: 1px solid var(--rpt-border);
  box-shadow: var(--rpt-shadow);
}
.rpt-section h2 {
  margin: 0 0 12px 0;
  font-size: 13px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--rpt-text-muted);
}
.rpt-env {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 12px;
}
.rpt-env__item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 13px;
}
.rpt-env__label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--rpt-text-subtle);
}
.rpt-env__value {
  font-family: var(--rpt-mono);
  font-size: 12px;
  color: var(--rpt-text);
  word-break: break-word;
}

.rpt-table {
  width: 100%;
  border-collapse: collapse;
  font-variant-numeric: tabular-nums;
  font-size: 13px;
}
.rpt-table th,
.rpt-table td {
  text-align: left;
  padding: 8px 10px;
  border-bottom: 1px solid var(--rpt-border);
  vertical-align: top;
}
.rpt-table th {
  font-weight: 600;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--rpt-text-muted);
  cursor: pointer;
  user-select: none;
  background: var(--rpt-surface-alt);
}
.rpt-table th[data-sort-active='asc']::after { content: ' \\25B2'; }
.rpt-table th[data-sort-active='desc']::after { content: ' \\25BC'; }
.rpt-table tr.rpt-row--fail { background: color-mix(in srgb, #ef4444 5%, transparent); }
.rpt-table tr.rpt-row--error { background: color-mix(in srgb, #d946ef 6%, transparent); }

.rpt-row__toggle {
  cursor: pointer;
  background: transparent;
  border: 0;
  color: var(--rpt-accent-hex);
  font-family: inherit;
  font-size: 12px;
  padding: 0;
}
.rpt-row__toggle::before { content: '\\25B8 '; }
.rpt-row__toggle[aria-expanded='true']::before { content: '\\25BE '; }

.rpt-sub {
  background: var(--rpt-surface-alt);
}
.rpt-sub td { padding: 8px 16px; }
.rpt-sub-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
.rpt-sub-table th,
.rpt-sub-table td {
  padding: 6px 8px;
  border-bottom: 1px solid var(--rpt-border);
  text-align: left;
}
.rpt-sub-table th {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--rpt-text-subtle);
}

.rpt-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 500;
  line-height: 1.4;
  color: var(--badge-color);
  color: var(--badge-color-oklch);
  background: color-mix(in srgb, var(--badge-color) 14%, transparent);
  border: 1px solid color-mix(in srgb, var(--badge-color) 36%, transparent);
  white-space: nowrap;
}
.rpt-badge--compact { font-size: 10px; padding: 1px 6px; }
.rpt-badge__glyph { font-weight: 700; }

.rpt-evidence {
  font-family: var(--rpt-mono);
  font-size: 12px;
  color: var(--rpt-accent-hex);
  text-decoration: none;
  border-bottom: 1px dashed currentColor;
}
.rpt-evidence:hover { opacity: 0.8; }
.rpt-evidence--missing {
  color: var(--rpt-text-subtle);
  font-style: italic;
  cursor: default;
  border-bottom: 0;
}

.rpt-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  padding-top: 16px;
  margin-top: 20px;
  border-top: 1px solid var(--rpt-border);
}
.rpt-actions__button {
  font: inherit;
  padding: 6px 14px;
  border-radius: 6px;
  border: 1px solid var(--rpt-border);
  background: var(--rpt-surface);
  color: var(--rpt-text);
  cursor: pointer;
}
.rpt-actions__button:hover { background: var(--rpt-surface-alt); }
.rpt-actions__button:focus-visible {
  outline: 2px solid var(--rpt-accent-hex);
  outline-offset: 2px;
}

.report--print .rpt-actions,
.report--print .rpt-row__toggle { display: none; }
.report--print .rpt-sub { display: table-row !important; }
.report--print .rpt-header__meta { font-size: 11px; }
.report--print main.report { padding: 8px; }

@media print {
  .rpt-actions, .rpt-row__toggle { display: none; }
  .rpt-section { break-inside: avoid; box-shadow: none; }
  body { background: #ffffff; }
  .rpt-sub { display: table-row !important; }
}
`.trim();

/**
 * The inline JS that powers (a) sort-by-column on the case table and
 * (b) per-case expand toggles. Kept tiny and dependency-free.
 */
export const REPORT_SCRIPT: string = `
(function() {
  // Expand / collapse per-row detail sections.
  document.querySelectorAll('[data-row-toggle]').forEach(function(btn) {
    btn.addEventListener('click', function() {
      var row = document.getElementById(btn.getAttribute('data-row-toggle'));
      if (!row) return;
      var open = btn.getAttribute('aria-expanded') === 'true';
      btn.setAttribute('aria-expanded', open ? 'false' : 'true');
      row.style.display = open ? 'none' : 'table-row';
    });
  });

  // Sort the main case table by clicking a header cell.
  document.querySelectorAll('table.rpt-table').forEach(function(table) {
    var ths = table.querySelectorAll('thead th[data-col]');
    ths.forEach(function(th, colIdx) {
      th.addEventListener('click', function() {
        var current = th.getAttribute('data-sort-active');
        var dir = current === 'asc' ? 'desc' : 'asc';
        ths.forEach(function(x) { x.removeAttribute('data-sort-active'); });
        th.setAttribute('data-sort-active', dir);
        var tbody = table.querySelector('tbody');
        if (!tbody) return;
        // Sort only top-level rows (not the expand-detail rows).
        var rows = Array.prototype.filter.call(
          tbody.querySelectorAll('tr[data-row-kind="case"]'),
          function() { return true; }
        );
        rows.sort(function(a, b) {
          var av = a.children[colIdx] ? a.children[colIdx].getAttribute('data-sort-value') || a.children[colIdx].textContent : '';
          var bv = b.children[colIdx] ? b.children[colIdx].getAttribute('data-sort-value') || b.children[colIdx].textContent : '';
          var na = parseFloat(av), nb = parseFloat(bv);
          if (!isNaN(na) && !isNaN(nb)) return dir === 'asc' ? na - nb : nb - na;
          return dir === 'asc' ? av.localeCompare(bv) : bv.localeCompare(av);
        });
        rows.forEach(function(r) {
          tbody.appendChild(r);
          var detailId = r.getAttribute('data-detail-id');
          if (detailId) {
            var detail = document.getElementById(detailId);
            if (detail) tbody.appendChild(detail);
          }
        });
      });
    });
  });

  // Print-friendly toggle.
  var printBtn = document.getElementById('rpt-action-print');
  if (printBtn) {
    printBtn.addEventListener('click', function() {
      document.body.classList.toggle('report--print');
    });
  }

  // Copy as Markdown.
  var copyBtn = document.getElementById('rpt-action-copy-md');
  if (copyBtn) {
    copyBtn.addEventListener('click', function() {
      var md = document.getElementById('rpt-payload-md');
      if (!md || !navigator.clipboard) return;
      navigator.clipboard.writeText(md.textContent || '').then(function() {
        var prev = copyBtn.textContent;
        copyBtn.textContent = 'Copied!';
        setTimeout(function() { copyBtn.textContent = prev; }, 1500);
      });
    });
  }

  // Download JSON.
  var jsonBtn = document.getElementById('rpt-action-download-json');
  if (jsonBtn) {
    jsonBtn.addEventListener('click', function() {
      var payload = document.getElementById('rpt-payload-json');
      if (!payload) return;
      var blob = new Blob([payload.textContent || ''], { type: 'application/json' });
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = (document.title || 'verdicts') + '.json';
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      setTimeout(function() { URL.revokeObjectURL(url); }, 1000);
    });
  }
})();
`.trim();
