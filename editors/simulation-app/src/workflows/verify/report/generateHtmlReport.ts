/**
 * Pure-function HTML report generator for the Verify workflow.
 *
 * Given `Verdict[]` + run metadata, emits a self-contained HTML
 * document (single `<style>` block, small inline `<script>` for
 * sortable columns and expand toggles, no external dependencies).
 *
 * The function is deliberately side-effect-free and has no React /
 * DOM imports, so callers can run it from Node tooling, server
 * contexts, or CLIs without a browser polyfill.
 *
 * Markdown parity: `generateMarkdownReport.ts` emits the same data as
 * a GFM string; both files consume the same `ReportInput` contract.
 */

import type { Verdict, VerdictKind } from '@/engine/types';
import { generateMarkdownReport } from './generateMarkdownReport';
import {
  escapeHtml,
  formatDurationMs,
  formatFilenameStamp,
  formatTimestamp,
  formatValue,
  renderBadge,
  REPORT_SCRIPT,
  REPORT_STYLESHEET,
  slugifyWorkspace,
  VERDICT_TOKENS,
} from './reportTemplate';
import type {
  ReportCase,
  ReportInput,
  ReportOutput,
  VerdictRollup,
} from './types';

// ── Helpers ──────────────────────────────────────────────────────────

/**
 * Group verdicts by case id. Verdicts whose `id` matches a value in a
 * case's `requirementIds` go into that bucket; anything unmatched
 * falls under the synthetic `__ungrouped` case so it still appears in
 * the report.
 *
 * Each case's `summary` (pre-computed by the backend) is carried
 * through unchanged. The synthetic `__ungrouped` case is invented by
 * the generator, so its rollup is computed inline here — this is a
 * generator-local projection, not a fallback for missing backend data.
 */
interface GroupedCase {
  caseInfo: ReportCase;
  verdicts: Verdict[];
  summary: VerdictRollup;
}

function groupByCase(input: ReportInput): GroupedCase[] {
  const byReq = new Map<string, number>(); // reqId → case index
  input.cases.forEach((c, idx) => {
    c.requirementIds.forEach((r) => byReq.set(r, idx));
  });

  const buckets: Verdict[][] = input.cases.map(() => []);
  const ungrouped: Verdict[] = [];
  for (const v of input.verdicts) {
    const idx = v.id !== undefined ? byReq.get(v.id) : undefined;
    if (idx !== undefined) {
      buckets[idx].push(v);
    } else {
      ungrouped.push(v);
    }
  }

  const grouped: GroupedCase[] = input.cases.map((c, idx) => ({
    caseInfo: c,
    verdicts: buckets[idx],
    summary: c.summary,
  }));
  if (ungrouped.length > 0) {
    grouped.push({
      caseInfo: {
        id: '__ungrouped',
        label: 'Ungrouped verdicts',
        requirementIds: [],
        summary: rollupFromVerdicts(ungrouped),
      },
      verdicts: ungrouped,
      summary: rollupFromVerdicts(ungrouped),
    });
  }
  return grouped;
}

/**
 * Local rollup for the synthetic Ungrouped bucket only. Mirrors the
 * backend `VerdictRollup::overall` worst-wins priority
 * (Error > Fail > Inconclusive > Pass).
 */
function rollupFromVerdicts(verdicts: Verdict[]): VerdictRollup {
  let pass = 0, fail = 0, inconclusive = 0, error = 0;
  for (const v of verdicts) {
    if (v.verdict === 'pass') pass += 1;
    else if (v.verdict === 'fail') fail += 1;
    else if (v.verdict === 'inconclusive') inconclusive += 1;
    else error += 1;
  }
  const overall: VerdictKind =
    error > 0 ? 'error'
      : fail > 0 ? 'fail'
        : inconclusive > 0 ? 'inconclusive'
          : 'pass';
  return { pass, fail, inconclusive, error, overall };
}

/** Render the `evidence` cell — either a link or a greyed stub. */
function renderEvidence(v: Verdict): string {
  if (!v.evidence) {
    return `<span class="rpt-evidence rpt-evidence--missing">no evidence attached</span>`;
  }
  const { session_id: sessionId, tick, element_id: elementId } = v.evidence;
  const params = new URLSearchParams();
  params.set('session', sessionId);
  params.set('tick', String(tick));
  if (elementId) params.set('element', elementId);
  return (
    `<a class="rpt-evidence" href="/run?${escapeHtml(params.toString())}">Open in Run</a>`
  );
}

/**
 * Compute the total runtime for a case by summing the `runtimeMs`
 * fields of its verdicts. Verdicts without a runtimeMs contribute 0.
 */
function caseRuntimeMs(verdicts: Verdict[]): number {
  let total = 0;
  for (const v of verdicts) {
    if (typeof v.runtimeMs === 'number' && Number.isFinite(v.runtimeMs)) {
      total += v.runtimeMs;
    }
  }
  return total;
}

function pickRepresentativeActual(verdicts: Verdict[]): unknown {
  // Prefer a failing verdict's actual (that's what the user wants to
  // see at a glance). Falls through to the first verdict's actual.
  const fail = verdicts.find((v) => v.verdict === 'fail' || v.verdict === 'error');
  if (fail && fail.actual !== undefined) return fail.actual;
  if (verdicts.length && verdicts[0].actual !== undefined) return verdicts[0].actual;
  return null;
}

function pickRepresentativeExpected(verdicts: Verdict[]): unknown {
  const fail = verdicts.find((v) => v.verdict === 'fail' || v.verdict === 'error');
  if (fail && fail.expected !== undefined) return fail.expected;
  if (verdicts.length && verdicts[0].expected !== undefined) return verdicts[0].expected;
  return null;
}

function pickRepresentativeMargin(verdicts: Verdict[]): number | null {
  const fail = verdicts.find((v) => v.verdict === 'fail' || v.verdict === 'error');
  if (fail && typeof fail.margin === 'number') return fail.margin;
  const first = verdicts.find((v) => typeof v.margin === 'number');
  return first ? (first.margin as number) : null;
}

/**
 * Surface the first non-empty `error` string in the group so the case
 * row's Actual cell can render `error: <msg>` instead of `—`. Falls
 * back to null when nothing in the group reported an evaluation error.
 */
function pickRepresentativeError(verdicts: Verdict[]): string | null {
  for (const v of verdicts) {
    if (typeof v.error === 'string' && v.error.length > 0) return v.error;
  }
  return null;
}

// ── Section renderers ────────────────────────────────────────────────

function renderHeader(input: ReportInput): string {
  const { summary } = input;
  const t = VERDICT_TOKENS[summary.overall];
  return `
  <header class="rpt-header">
    <div>
      <h1>${escapeHtml(input.workspaceName)} — Verification Report</h1>
      <div class="rpt-header__meta">
        <span><strong>Run:</strong> ${escapeHtml(formatTimestamp(input.runTimestamp))}</span>
        <span><strong>Duration:</strong> ${escapeHtml(formatDurationMs(input.durationMs))}</span>
        <span><strong>Cases:</strong> ${input.cases.length}</span>
      </div>
    </div>
    <div class="rpt-overall" data-overall="${summary.overall}">
      <span class="rpt-overall__badge" style="--badge-color: ${t.hex}; --badge-color-oklch: ${t.oklch};" title="${escapeHtml(t.label)}" aria-label="Overall verdict: ${escapeHtml(t.label)}">${t.glyph}</span>
      <div class="rpt-overall__counts">
        <span><strong>${summary.pass}</strong> pass</span>
        <span><strong>${summary.fail}</strong> fail</span>
        <span><strong>${summary.inconclusive}</strong> inconclusive</span>
        <span><strong>${summary.error}</strong> error</span>
      </div>
    </div>
  </header>`;
}

function renderEnvironment(input: ReportInput): string {
  const env = input.environment;
  const items: Array<[string, string]> = [];
  items.push(['Backend', env.backendVersion]);
  if (env.suiteName) items.push(['Suite', env.suiteName]);
  if (env.simDt !== undefined) items.push(['Sim dt', `${env.simDt} s`]);
  if (env.seeds && env.seeds.length > 0) {
    items.push(['Seeds', env.seeds.join(', ')]);
  }
  const rows = items
    .map(
      ([label, value]) => `
      <div class="rpt-env__item">
        <span class="rpt-env__label">${escapeHtml(label)}</span>
        <span class="rpt-env__value">${escapeHtml(value)}</span>
      </div>`,
    )
    .join('');
  return `
  <section class="rpt-section">
    <h2>Environment</h2>
    <div class="rpt-env">${rows}</div>
  </section>`;
}

function renderProvenance(input: ReportInput): string {
  const p = input.provenance;
  if (!p) return '';
  const dash = (v: string | number | undefined | null): string =>
    v === undefined || v === null || v === '' ? '—' : String(v);
  const items: Array<[string, string]> = [
    // Evaluation mode is BINDING (§2.1a(d)) — a downloaded report must say
    // whether verdicts were static desk checks, trajectory runs, or
    // ingested external evidence. Leads the block: it qualifies every
    // verdict below it.
    ['Evaluation', p.evaluationModes && p.evaluationModes.length > 0 ? p.evaluationModes.join(', ') : '—'],
    ['Model revision', dash(p.model?.manifestHash)],
    ['Workspace root', dash(p.model?.workspaceRoot)],
    ['Session', dash(p.sessionId)],
    ['Archive', dash(p.archiveId)],
    ['Origin', dash(p.origin)],
    ['Sim dt', p.runConfig?.dt !== undefined ? `${p.runConfig.dt} s` : '—'],
    ['Scenario', dash(p.runConfig?.scenario)],
    ['View', dash(p.runConfig?.view)],
    ['Started', dash(p.startedAt)],
    ['Stopped', dash(p.stoppedAt)],
  ];
  if (p.selectedCases && p.selectedCases.length > 0) {
    items.push(['Cases', p.selectedCases.join(', ')]);
  }
  if (p.overrides && p.overrides.length > 0) {
    items.push(['Overrides', p.overrides.map(([k, v]) => `${k}=${v}`).join(', ')]);
  }
  const rows = items
    .map(
      ([label, value]) => `
      <div class="rpt-env__item">
        <span class="rpt-env__label">${escapeHtml(label)}</span>
        <span class="rpt-env__value">${escapeHtml(value)}</span>
      </div>`,
    )
    .join('');
  const fileRows =
    p.model?.files && p.model.files.length > 0
      ? `<table class="rpt-sub-table"><thead><tr><th>File</th><th>Hash</th></tr></thead><tbody>${p.model.files
          .map((f) => `<tr><td>${escapeHtml(f.uri)}</td><td>${escapeHtml(f.hash ?? '—')}</td></tr>`)
          .join('')}</tbody></table>`
      : '';
  return `
  <section class="rpt-section">
    <h2>Provenance</h2>
    <div class="rpt-env">${rows}</div>
    ${fileRows}
  </section>`;
}

function renderCaseRow(group: GroupedCase, index: number): string {
  const overall = group.summary.overall;
  const badge = renderBadge(overall);
  const detailId = `rpt-case-detail-${index}`;
  const errorStr = pickRepresentativeError(group.verdicts);
  // Fold the evaluation error into the Actual cell with a distinct
  // "error: <msg>" prefix so readers don't conflate it with inconclusive.
  const actual = errorStr ? `error: ${errorStr}` : formatValue(pickRepresentativeActual(group.verdicts));
  const expected = formatValue(pickRepresentativeExpected(group.verdicts));
  const marginValue = pickRepresentativeMargin(group.verdicts);
  const margin = marginValue !== null ? formatValue(marginValue) : '—';
  const runtime = formatDurationMs(caseRuntimeMs(group.verdicts));
  const rowClass =
    overall === 'fail'
      ? 'rpt-row--fail'
      : overall === 'error'
        ? 'rpt-row--error'
        : '';
  return `
    <tr class="${rowClass}" data-row-kind="case" data-detail-id="${detailId}">
      <td>
        <button type="button" class="rpt-row__toggle" aria-expanded="false" data-row-toggle="${detailId}">
          ${escapeHtml(group.caseInfo.label)}
        </button>
      </td>
      <td data-sort-value="${overall}">${badge}</td>
      <td data-sort-value="${escapeHtml(actual)}">${escapeHtml(actual)}</td>
      <td data-sort-value="${escapeHtml(expected)}">${escapeHtml(expected)}</td>
      <td data-sort-value="${marginValue ?? ''}">${escapeHtml(margin)}</td>
      <td data-sort-value="${caseRuntimeMs(group.verdicts)}">${escapeHtml(runtime)}</td>
    </tr>
    <tr id="${detailId}" class="rpt-sub" style="display: none;" data-row-kind="detail">
      <td colspan="6">
        ${renderSubTable(group.verdicts)}
      </td>
    </tr>`;
}

function renderSubTable(verdicts: Verdict[]): string {
  if (verdicts.length === 0) {
    return `<em style="color: var(--rpt-text-subtle);">No requirement verdicts captured for this case.</em>`;
  }
  const rows = verdicts
    .map((v) => {
      const badge = renderBadge(v.verdict, { compact: true });
      const errStr = typeof v.error === 'string' && v.error.length > 0 ? v.error : null;
      const actual = errStr ? `error: ${errStr}` : formatValue(v.actual ?? null);
      const reason = v.reason ? escapeHtml(v.reason) : '';
      const label = escapeHtml(v.label ?? v.id ?? '(unnamed)');
      const evidence = renderEvidence(v);
      // When the row carries an evaluation error, mark the row so the
      // stylesheet can colour the Actual cell distinctly from a normal
      // inconclusive row (whose Actual is "—").
      const trClass = errStr ? ' class="rpt-sub-row--error"' : '';
      return `
        <tr${trClass}>
          <td>${label}</td>
          <td>${badge}</td>
          <td>${escapeHtml(actual)}</td>
          <td>${reason}</td>
          <td>${evidence}</td>
        </tr>`;
    })
    .join('');
  return `
    <table class="rpt-sub-table">
      <thead>
        <tr>
          <th>Requirement</th>
          <th>Verdict</th>
          <th>Actual</th>
          <th>Reason / note</th>
          <th>Evidence</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>`;
}

function renderCaseTable(groups: GroupedCase[]): string {
  const rows = groups.map(renderCaseRow).join('');
  return `
  <section class="rpt-section">
    <h2>Cases (${groups.length})</h2>
    <table class="rpt-table">
      <thead>
        <tr>
          <th data-col="name">Case</th>
          <th data-col="verdict">Verdict</th>
          <th data-col="actual">Actual</th>
          <th data-col="expected">Expected</th>
          <th data-col="margin">Margin</th>
          <th data-col="runtime">Runtime</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>
  </section>`;
}

function renderActions(input: ReportInput, markdown: string): string {
  const jsonPayload = JSON.stringify(input.verdicts, null, 2);
  return `
  <div class="rpt-actions">
    <button type="button" class="rpt-actions__button" id="rpt-action-copy-md">Copy as Markdown</button>
    <button type="button" class="rpt-actions__button" id="rpt-action-download-json">Download JSON</button>
    <button type="button" class="rpt-actions__button" id="rpt-action-print">Print-friendly view</button>
  </div>
  <script type="text/plain" id="rpt-payload-md">${escapeHtml(markdown)}</script>
  <script type="application/json" id="rpt-payload-json">${escapeHtml(jsonPayload)}</script>`;
}

// ── Public API ───────────────────────────────────────────────────────

/**
 * Generate a standalone HTML report. Returns the HTML string and the
 * conventional filename (`verify-{workspace}-{YYYYMMDD-HHMMSS}.html`).
 */
export function generateHtmlReport(input: ReportInput): ReportOutput {
  const groups = groupByCase(input);
  const markdown = generateMarkdownReport(input);

  const body = `
  ${renderHeader(input)}
  ${renderEnvironment(input)}
  ${renderProvenance(input)}
  ${renderCaseTable(groups)}
  ${renderActions(input, markdown)}
`;

  const title = `${input.workspaceName} — Verify (${formatTimestamp(input.runTimestamp)})`;
  const html = [
    '<!DOCTYPE html>',
    '<html lang="en" data-theme="light">',
    '<head>',
    '<meta charset="utf-8" />',
    '<meta name="viewport" content="width=device-width, initial-scale=1" />',
    `<title>${escapeHtml(title)}</title>`,
    `<style>${REPORT_STYLESHEET}</style>`,
    '</head>',
    '<body>',
    '<main class="report">',
    body,
    '</main>',
    `<script>${REPORT_SCRIPT}</script>`,
    '</body>',
    '</html>',
    '',
  ].join('\n');

  const filename = `verify-${slugifyWorkspace(input.workspaceName)}-${formatFilenameStamp(input.runTimestamp)}.html`;
  return { html, filename };
}
