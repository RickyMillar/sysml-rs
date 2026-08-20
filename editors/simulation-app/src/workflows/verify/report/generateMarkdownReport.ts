/**
 * Pure-function GFM report generator — the Markdown sibling of
 * `generateHtmlReport.ts`.
 *
 * Used in two places:
 *   1. The HTML "Copy as Markdown" footer action embeds the output
 *      verbatim so users can paste a compact summary into PRs, issues,
 *      or chat.
 *   2. Node-side tooling / CI can call it directly to archive a
 *      human-readable verdict log next to the raw JSON.
 *
 * The Markdown output intentionally omits the action buttons and the
 * inline JSON payload — it's a summary, not a self-contained archive.
 */

import type { Verdict, VerdictKind } from '@/engine/types';
import type { ReportCase, ReportInput, VerdictRollup } from './types';
import {
  formatDurationMs,
  formatTimestamp,
  formatValue,
  VERDICT_TOKENS,
} from './reportTemplate';

function mdBadge(kind: VerdictKind): string {
  const t = VERDICT_TOKENS[kind];
  return `${t.glyph} ${t.label}`;
}

function mdEscape(value: string): string {
  // Minimal escape for table cells: pipes break GFM table rows.
  return value.replace(/\|/g, '\\|').replace(/\n/g, ' ');
}

function mdEvidence(v: Verdict): string {
  if (!v.evidence) return '_no evidence_';
  const { session_id: sessionId, tick, element_id: elementId } = v.evidence;
  const params = new URLSearchParams();
  params.set('session', sessionId);
  params.set('tick', String(tick));
  if (elementId) params.set('element', elementId);
  return `[Open in Run](/run?${params.toString()})`;
}

interface GroupedCase {
  caseInfo: ReportCase;
  verdicts: Verdict[];
  summary: VerdictRollup;
}

function groupByCase(input: ReportInput): GroupedCase[] {
  const byReq = new Map<string, number>();
  input.cases.forEach((c, idx) => c.requirementIds.forEach((r) => byReq.set(r, idx)));
  const buckets: Verdict[][] = input.cases.map(() => []);
  const ungrouped: Verdict[] = [];
  for (const v of input.verdicts) {
    const idx = v.id !== undefined ? byReq.get(v.id) : undefined;
    if (idx !== undefined) buckets[idx].push(v);
    else ungrouped.push(v);
  }
  const out: GroupedCase[] = input.cases.map((c, idx) => ({
    caseInfo: c,
    verdicts: buckets[idx],
    summary: c.summary,
  }));
  if (ungrouped.length) {
    const ungroupedSummary = rollupFromVerdicts(ungrouped);
    out.push({
      caseInfo: {
        id: '__ungrouped',
        label: 'Ungrouped verdicts',
        requirementIds: [],
        summary: ungroupedSummary,
      },
      verdicts: ungrouped,
      summary: ungroupedSummary,
    });
  }
  return out;
}

/**
 * Local rollup for the synthetic Ungrouped bucket only — the
 * generator invents this bucket so it owns its own summary.
 * Mirrors the backend `VerdictRollup::overall` worst-wins priority.
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

/**
 * Generate a GitHub-Flavored Markdown report. Stable output — deterministic
 * formatting so snapshot tests don't flake across platforms.
 */
export function generateMarkdownReport(input: ReportInput): string {
  const { summary } = input;
  const groups = groupByCase(input);

  const lines: string[] = [];
  lines.push(`# ${input.workspaceName} — Verification Report`);
  lines.push('');
  lines.push(`**Overall:** ${mdBadge(summary.overall)}  `);
  lines.push(`**Run:** ${formatTimestamp(input.runTimestamp)}  `);
  lines.push(`**Duration:** ${formatDurationMs(input.durationMs)}  `);
  lines.push(
    `**Verdict counts:** ${summary.pass} pass · ${summary.fail} fail · ${summary.inconclusive} inconclusive · ${summary.error} error`,
  );
  lines.push('');

  lines.push('## Environment');
  lines.push('');
  lines.push(`- **Backend:** ${input.environment.backendVersion}`);
  if (input.environment.suiteName) lines.push(`- **Suite:** ${input.environment.suiteName}`);
  if (input.environment.simDt !== undefined) lines.push(`- **Sim dt:** ${input.environment.simDt} s`);
  if (input.environment.seeds && input.environment.seeds.length > 0) {
    lines.push(`- **Seeds:** ${input.environment.seeds.join(', ')}`);
  }
  lines.push('');

  const p = input.provenance;
  if (p) {
    const dash = (v: string | number | undefined | null): string =>
      v === undefined || v === null || v === '' ? '—' : String(v);
    lines.push('## Provenance');
    lines.push('');
    // Evaluation mode is BINDING (§2.1a(d)) — leads the block, qualifies
    // every verdict in the report.
    lines.push(
      `- **Evaluation:** ${
        p.evaluationModes && p.evaluationModes.length > 0 ? p.evaluationModes.join(', ') : '—'
      }`,
    );
    lines.push(`- **Model revision:** ${dash(p.model?.manifestHash)}`);
    lines.push(`- **Workspace root:** ${dash(p.model?.workspaceRoot)}`);
    // Per-file manifest (§6.2) — parity with the HTML report's file table.
    if (p.model?.files && p.model.files.length > 0) {
      lines.push(`- **Files (${p.model.files.length}):**`);
      for (const f of p.model.files) {
        lines.push(`  - \`${f.uri}\` — ${f.hash ? `\`${f.hash}\`` : '—'}`);
      }
    }
    lines.push(`- **Session:** ${dash(p.sessionId)}`);
    lines.push(`- **Archive:** ${dash(p.archiveId)}`);
    lines.push(`- **Origin:** ${dash(p.origin)}`);
    if (p.runConfig?.dt !== undefined) lines.push(`- **Sim dt:** ${p.runConfig.dt} s`);
    if (p.runConfig?.scenario) lines.push(`- **Scenario:** ${p.runConfig.scenario}`);
    if (p.runConfig?.view) lines.push(`- **View:** ${p.runConfig.view}`);
    if (p.startedAt) lines.push(`- **Started:** ${p.startedAt}`);
    if (p.stoppedAt) lines.push(`- **Stopped:** ${p.stoppedAt}`);
    if (p.selectedCases && p.selectedCases.length > 0) lines.push(`- **Cases:** ${p.selectedCases.join(', ')}`);
    if (p.overrides && p.overrides.length > 0) lines.push(`- **Overrides:** ${p.overrides.map(([k, v]) => `${k}=${v}`).join(', ')}`);
    lines.push('');
  }

  lines.push('## Cases');
  lines.push('');
  lines.push('| Case | Verdict | Actual | Expected | Margin | Runtime |');
  lines.push('|---|---|---|---|---|---|');
  for (const g of groups) {
    const failing = g.verdicts.find((v) => v.verdict === 'fail' || v.verdict === 'error');
    const repr = failing ?? g.verdicts[0];
    const actual = repr ? formatValue(repr.actual ?? null) : '—';
    const expected = repr ? formatValue(repr.expected ?? null) : '—';
    const margin =
      repr && typeof repr.margin === 'number' ? formatValue(repr.margin) : '—';
    let runtimeMs = 0;
    for (const v of g.verdicts) {
      if (typeof v.runtimeMs === 'number' && Number.isFinite(v.runtimeMs)) runtimeMs += v.runtimeMs;
    }
    lines.push(
      `| ${mdEscape(g.caseInfo.label)} | ${mdBadge(g.summary.overall)} | ${mdEscape(actual)} | ${mdEscape(expected)} | ${mdEscape(margin)} | ${formatDurationMs(runtimeMs)} |`,
    );
  }
  lines.push('');

  // Per-case requirement detail — only for cases that aren't all pass.
  const interestingGroups = groups.filter((g) => g.summary.overall !== 'pass');
  if (interestingGroups.length > 0) {
    lines.push('## Requirement detail');
    lines.push('');
    for (const g of interestingGroups) {
      lines.push(`### ${g.caseInfo.label} — ${mdBadge(g.summary.overall)}`);
      lines.push('');
      lines.push('| Requirement | Verdict | Actual | Reason | Evidence |');
      lines.push('|---|---|---|---|---|');
      for (const v of g.verdicts) {
        const label = v.label ?? v.id ?? '(unnamed)';
        const actual = formatValue(v.actual ?? null);
        const reason = v.reason ? mdEscape(v.reason) : '';
        lines.push(
          `| ${mdEscape(label)} | ${mdBadge(v.verdict)} | ${mdEscape(actual)} | ${reason} | ${mdEvidence(v)} |`,
        );
      }
      lines.push('');
    }
  }

  return lines.join('\n');
}
