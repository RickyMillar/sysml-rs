/**
 * VerifyHistoryView — the History sub-view, redesigned (design turn 3;
 * test-management study P3, ratified 2026-07-19).
 *
 * History splits into the two questions the old global timeline
 * conflated:
 *
 *   · LATEST STATUS (default) — "where are we?": one row per case, its
 *     current standing PER MODE (static desk check · latest run ∿ ·
 *     latest external ↓), grouped by the model's own structure
 *     (`qualified_name` owner chain — the industry's "suite" is our
 *     containment). Failing rows sort first within their group; band
 *     headers and facet chips carry trouble upward. This is the screen
 *     that stays legible at 1000 runs a day.
 *   · EXECUTIONS — "what happened?": one row per recorded performance,
 *     newest first, provenance on the row, per-case results (with their
 *     pinned case digests and changed-since flags) on expand.
 *
 * The global timeline DOES NOT return here — the lane timeline survives
 * only case-scoped, inside the case view's process register.
 *
 * Binding constraints (brief §4): per-mode never flat (1); verdict
 * colours only on verdicts, 1d mode badges, Sans approval read, warning
 * family for Δ/⚑ (2); changed-since renders on the entry whenever the
 * server says true, null renders nothing (3); external is dashed
 * provenance (4); unapproved + passing reads qualified (5); honest
 * empties (6); no global timeline (7).
 */

import { useMemo, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { VerdictBadge, normalizeVerdict } from '@/components/VerdictBadge';
import { EvaluationModeBadge } from '@/components/EvaluationModeBadge';
import { ApprovalRead } from '@/features/workflow/ApprovalRead';
import { useApprovalStates } from '@/features/workflow/queries';
import {
  caseIdOf,
  normalizeCaseVerdict,
  useVerificationCases,
} from './useVerificationCases';
import {
  composeLatestRows,
  groupLatestRows,
  latestByCase,
  recentVerdicts,
  relativeAge,
  rowStanding,
  useExecutions,
  useLatestStatus,
  type ExecutionRowWire,
  type LatestStatusRow,
} from './useExecutionHistory';

type HistoryMode = 'latest' | 'executions';
type Facet = 'all' | 'failing' | 'stale' | 'changed' | 'unapproved';

export interface VerifyHistoryViewProps {
  /** Open a case as a document (row click) — same id family findCase resolves. */
  onOpenCase: (caseId: string) => void;
}

export function VerifyHistoryView({ onOpenCase }: VerifyHistoryViewProps) {
  const [mode, setMode] = useState<HistoryMode>('latest');
  const casesQuery = useVerificationCases();
  const latestQuery = useLatestStatus();
  const executionsQuery = useExecutions();

  const cases = useMemo(() => casesQuery.data ?? [], [casesQuery.data]);
  const executions = useMemo(() => executionsQuery.data ?? [], [executionsQuery.data]);
  const latest = useMemo(() => latestByCase(latestQuery.data ?? []), [latestQuery.data]);
  const rows = useMemo(() => composeLatestRows(cases, latest), [cases, latest]);

  const elementIds = useMemo(
    () => rows.map((r) => r.elementId).filter((id): id is string => !!id),
    [rows],
  );
  const { states: approvals } = useApprovalStates(elementIds);

  return (
    <div data-testid="verify-history-view" className="flex flex-col h-full min-h-0">
      {/* ── the two-question toggle ─────────────────────────────────── */}
      <div
        className="flex items-center gap-3 px-3 shrink-0"
        style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}
      >
        <div className="flex" style={{ border: '1px solid var(--border-strong)', borderRadius: 4, overflow: 'hidden', fontSize: 11 }}>
          <HistoryModeTab
            id="latest"
            label="Latest status"
            active={mode === 'latest'}
            onClick={() => setMode('latest')}
            title="Where are we? — one row per case, current standing per mode"
          />
          <HistoryModeTab
            id="executions"
            label="Executions"
            active={mode === 'executions'}
            onClick={() => setMode('executions')}
            title="What happened? — one row per execution, newest first"
          />
        </div>
        <div style={{ flex: 1 }} />
        <ExecutionsSummary executions={executions} />
      </div>

      <div className="flex-1 min-h-0 overflow-auto">
        {mode === 'latest' ? (
          <LatestStatusTable
            rows={rows}
            executions={executions}
            approvals={approvals}
            isLoading={casesQuery.isLoading || latestQuery.isLoading}
            onOpenCase={onOpenCase}
          />
        ) : (
          <ExecutionsList executions={executions} isLoading={executionsQuery.isLoading} />
        )}
      </div>
    </div>
  );
}

// ── Latest-status table ──────────────────────────────────────────────

// Calm pass: the mode cells shrank (geometry mark + bare glyph + quiet
// ref, the mode WORD moved to the header) so the columns narrow and the
// case column — the row's one bright identifier — gets the freed width.
const COL = {
  approval: 104,
  static: 96,
  trajectory: 210,
  external: 240,
  recent: 104,
  /** Trailing qualification slot — present on every row so the fixed
   *  columns stay vertically aligned whether or not a row is qualified. */
  qualified: 130,
} as const;

function LatestStatusTable({
  rows,
  executions,
  approvals,
  isLoading,
  onOpenCase,
}: {
  rows: LatestStatusRow[];
  executions: ExecutionRowWire[];
  approvals: Map<string, string>;
  isLoading: boolean;
  onOpenCase: (caseId: string) => void;
}) {
  const [facet, setFacet] = useState<Facet>('all');
  const [filter, setFilter] = useState('');

  const isUnapproved = (row: LatestStatusRow) =>
    !!row.elementId && approvals.get(row.elementId) !== undefined
      ? approvals.get(row.elementId) !== 'approved'
      : false;

  const facetCounts = useMemo(
    () => ({
      all: rows.length,
      failing: rows.filter((r) => r.failing).length,
      stale: rows.filter((r) => r.stale).length,
      changed: rows.filter((r) => r.changed).length,
      unapproved: rows.filter(isUnapproved).length,
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [rows, approvals],
  );

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return rows.filter((row) => {
      if (needle && !row.caseName.toLowerCase().includes(needle)) return false;
      switch (facet) {
        case 'failing':
          return row.failing;
        case 'stale':
          return row.stale;
        case 'changed':
          return row.changed;
        case 'unapproved':
          return isUnapproved(row);
        default:
          return true;
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, facet, filter, approvals]);

  const groups = useMemo(() => groupLatestRows(visible), [visible]);

  if (isLoading && rows.length === 0) {
    return <EmptyNote testId="verify-latest-loading">Loading verification cases…</EmptyNote>;
  }
  if (rows.length === 0) {
    return (
      <EmptyNote testId="verify-latest-empty">
        No verification cases in this workspace — cases are authored in the source and appear here
        with their standing per mode.
      </EmptyNote>
    );
  }

  return (
    <div data-testid="verify-latest-status">
      {/* toolbar: filter + facet chips */}
      <div
        className="flex items-center gap-2 px-3"
        style={{ height: 36, borderBottom: '1px solid var(--border-hairline)' }}
      >
        <input
          data-testid="verify-latest-filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="filter cases…"
          className="mono-text"
          style={{
            width: 200,
            height: 24,
            padding: '0 10px',
            fontSize: 11,
            background: 'var(--surface-panel)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-hairline)',
            borderRadius: 4,
          }}
        />
        <FacetChip id="all" label={`all ${facetCounts.all}`} active={facet === 'all'} onClick={() => setFacet('all')} />
        <FacetChip id="failing" label={`failing ${facetCounts.failing}`} active={facet === 'failing'} onClick={() => setFacet('failing')} />
        <FacetChip id="stale" label={`stale ⚑ ${facetCounts.stale}`} active={facet === 'stale'} onClick={() => setFacet('stale')} />
        <FacetChip id="changed" label={`changed Δ ${facetCounts.changed}`} active={facet === 'changed'} onClick={() => setFacet('changed')} />
        <FacetChip id="unapproved" label={`unapproved ${facetCounts.unapproved}`} active={facet === 'unapproved'} onClick={() => setFacet('unapproved')} />
      </div>

      {/* Column headers — the mode vocabulary lives HERE, once, in full
          (calm-pass P3): the glyph + word name the column so the cells
          beneath carry only what varies. Tooltips teach on hover (P4). */}
      <div
        className="mono-text flex items-center"
        style={{ height: 26, borderBottom: '1px solid var(--border-hairline)', fontSize: 10, color: 'var(--text-muted)' }}
      >
        <span style={{ width: COL.approval, flex: 'none', padding: '0 12px' }}>approval</span>
        <span style={{ flex: 1, minWidth: 180 }}>case</span>
        <span
          style={{ width: COL.static, flex: 'none' }}
          title="Static desk check — checked against the model's current values, without a run. ƒ = computed."
        >
          static ƒ
        </span>
        <span
          style={{ width: COL.trajectory, flex: 'none' }}
          title="Trajectory — the latest run against a live simulation session. The solid square marks a record."
        >
          latest run · <span style={{ color: 'var(--sim-accent)' }}>∿ trajectory</span>
        </span>
        <span
          style={{ width: COL.external, flex: 'none' }}
          title="External — the latest result ingested from an outside tool. The dashed square marks ingested provenance."
        >
          latest external · <span style={{ color: 'var(--text-muted)' }}>↓ ingested</span>
        </span>
        <span style={{ width: COL.recent, flex: 'none' }}>recent</span>
        <span style={{ width: COL.qualified, flex: 'none' }} />
      </div>

      {groups.map((group) => (
        <div key={group.key || '(root)'} data-testid={`verify-latest-group-${group.label}`}>
          {/* band header — model structure, with its verdict rollup */}
          <div
            className="flex items-center"
            style={{ height: 26, background: 'var(--surface-panel)', borderBottom: '1px solid var(--border-hairline)' }}
          >
            <span className="mono-text" style={{ padding: '0 12px', fontSize: 10.5, color: 'var(--text-muted)' }}>
              {group.label}
            </span>
            <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-disabled)' }}>
              {group.rows.length} {group.rows.length === 1 ? 'case' : 'cases'}
            </span>
            <span style={{ flex: 1 }} />
            <BandRollup group={group.rows} approvals={approvals} />
          </div>
          {group.rows.map((row) => (
            <LatestStatusRowLine
              key={caseIdOf(row.row)}
              row={row}
              approval={row.elementId ? approvals.get(row.elementId) : undefined}
              recent={recentVerdicts(executions, row.caseName)}
              onOpen={() => onOpenCase(caseIdOf(row.row))}
            />
          ))}
        </div>
      ))}

      {visible.length === 0 ? (
        <EmptyNote testId="verify-latest-facet-empty">
          No case matches this filter — the cases still exist; clear the filter to see them.
        </EmptyNote>
      ) : null}

      <div
        className="flex items-center px-3"
        style={{ minHeight: 24, fontSize: 10.5, color: 'var(--text-muted)', lineHeight: 1.4 }}
      >
        an empty mode cell = no execution of that kind exists — never a fabricated default · failing
        rows sort first within their group · row click opens the case
      </div>
    </div>
  );
}

function LatestStatusRowLine({
  row,
  approval,
  recent,
  onOpen,
}: {
  row: LatestStatusRow;
  approval: string | undefined;
  recent: Array<'pass' | 'fail' | 'inconclusive' | 'error'>;
  onOpen: () => void;
}) {
  // The ruled coverage principle (§4.5): a passing standing on a case whose
  // process hasn't blessed it reads QUALIFIED wherever coverage is claimed.
  const anyPass =
    row.staticVerdict === 'pass' ||
    (row.trajectory && normalizeCaseVerdict(row.trajectory.verdict) === 'pass') ||
    (row.external && normalizeCaseVerdict(row.external.verdict) === 'pass');
  const qualified = !!anyPass && approval !== undefined && approval !== 'approved';

  return (
    <div
      data-testid={`verify-latest-row-${row.caseName}`}
      data-failing={row.failing || undefined}
      data-qualified={qualified || undefined}
      className="flex items-center"
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onOpen();
      }}
      style={{ height: 32, borderBottom: '1px solid var(--border-hairline)', cursor: 'pointer' }}
    >
      <span style={{ width: COL.approval, flex: 'none', padding: '0 12px' }}>
        {approval !== undefined ? <ApprovalRead state={approval} testId={`verify-approval-${row.caseName}`} /> : null}
      </span>
      {/* The one bright identifier — the row's scan anchor (calm-pass P1). */}
      <span
        className="mono-text truncate"
        style={{ flex: 1, minWidth: 180, fontSize: 12, color: 'var(--text-primary)', paddingRight: 8 }}
      >
        {row.caseName}
      </span>
      {/* static — a desk check has no record, so just the bare verdict glyph
          (the "static ƒ" column head already names the mode). */}
      <span style={{ width: COL.static, flex: 'none' }} className="flex items-center gap-1.5">
        {row.staticVerdict ? (
          <VerdictBadge verdict={row.staticVerdict} size="bare" name={row.caseName} testId={`verify-latest-static-${row.caseName}`} />
        ) : null}
      </span>
      {/* trajectory — geometry mark (solid = record) + bare verdict + quiet
          session ref + age; the mode word lives in the header (P3). */}
      <span style={{ width: COL.trajectory, flex: 'none' }} className="flex items-center gap-1.5">
        {row.trajectory ? (
          <>
            <EvaluationModeBadge mode="trajectory" size="mark" />
            <VerdictBadge
              verdict={normalizeVerdict(row.trajectory.verdict)}
              size="bare"
              name={row.caseName}
              testId={`verify-latest-run-${row.caseName}`}
            />
            <RefText>{shortId(row.trajectory.execution_id)}</RefText>
            <AgeText timestamp={row.trajectory.timestamp} />
            {row.trajectory.case_changed_since === true ? <ChangedSinceFlag /> : null}
          </>
        ) : null}
      </span>
      {/* external — dashed mark (provenance) + bare verdict + tool ref + age. */}
      <span style={{ width: COL.external, flex: 'none' }} className="flex items-center gap-1.5">
        {row.external ? (
          <>
            <EvaluationModeBadge
              mode="external"
              size="mark"
              stale={row.external.matches_current_model === false}
            />
            <VerdictBadge
              verdict={normalizeVerdict(row.external.verdict)}
              size="bare"
              name={row.caseName}
              testId={`verify-latest-external-${row.caseName}`}
            />
            {row.external.tool ? <RefText>{row.external.tool}</RefText> : null}
            <AgeText timestamp={row.external.timestamp} />
            {row.external.matches_current_model === false ? <StaleFlag /> : null}
            {row.external.case_changed_since === true ? <ChangedSinceFlag /> : null}
          </>
        ) : null}
      </span>
      {/* recent trend — bare coloured glyphs, no pills (calm-pass P2). */}
      <span style={{ width: COL.recent, flex: 'none' }} className="flex items-center gap-1.5">
        {recent.map((v, i) => (
          <VerdictBadge key={i} verdict={v} size="bare" name={`${row.caseName} recent`} showLabel={false} />
        ))}
      </span>
      {/* FIXED-width trailing slot on EVERY row — a conditional flexible
          suffix here steals width from the case column and shears the
          mode columns out of vertical alignment (live-caught). */}
      <span style={{ width: COL.qualified, flex: 'none', paddingRight: 12 }}>
        {qualified ? (
          <span
            data-testid={`verify-latest-qualified-${row.caseName}`}
            style={{ fontSize: 10, color: 'var(--severity-warning)', whiteSpace: 'nowrap' }}
            title="The run passed, but this case's definition is not approved — a passing execution of an unapproved case is not read as coverage without qualification."
          >
            · case not approved
          </span>
        ) : null}
      </span>
    </div>
  );
}

/** Band-level rollup: per-row standings (latest execution wins over the
 *  desk check — `rowStanding`) with the qualified-pass bucket labeled
 *  separately (never silently inside "pass"). */
function BandRollup({
  group,
  approvals,
}: {
  group: LatestStatusRow[];
  approvals: Map<string, string>;
}) {
  const counts = { pass: 0, passUnapproved: 0, fail: 0, inconclusive: 0, error: 0 };
  for (const row of group) {
    const standing = rowStanding(row);
    if (!standing) continue;
    if (standing === 'pass') {
      const approval = row.elementId ? approvals.get(row.elementId) : undefined;
      if (approval !== undefined && approval !== 'approved') counts.passUnapproved += 1;
      else counts.pass += 1;
    } else {
      counts[standing] += 1;
    }
  }
  const parts: ReactNode[] = [];
  if (counts.pass > 0) parts.push(<span key="p" style={{ color: 'var(--verdict-pass)' }}>{counts.pass} pass</span>);
  if (counts.passUnapproved > 0)
    parts.push(<span key="q" style={{ color: 'var(--severity-warning)' }}>{counts.passUnapproved} pass (unapproved)</span>);
  if (counts.fail > 0) parts.push(<span key="f" style={{ color: 'var(--verdict-fail)' }}>{counts.fail} fail</span>);
  if (counts.inconclusive > 0)
    parts.push(<span key="i" style={{ color: 'var(--verdict-inconclusive)' }}>{counts.inconclusive} inconclusive</span>);
  if (counts.error > 0) parts.push(<span key="e" style={{ color: 'var(--verdict-error)' }}>{counts.error} error</span>);
  return (
    <span className="mono-text flex items-center gap-1.5" style={{ paddingRight: 12, fontSize: 10 }}>
      {joinWithDots(parts)}
    </span>
  );
}

/** Interleave `·` separators — a bucket list must never LEAD with one
 *  (live-caught: "suite · 4 inconclusive" when the pass bucket was 0). */
function joinWithDots(parts: ReactNode[]): ReactNode[] {
  const out: ReactNode[] = [];
  parts.forEach((part, i) => {
    if (i > 0) out.push(<span key={`sep-${i}`} style={{ color: 'var(--text-muted)' }}>·</span>);
    out.push(part);
  });
  return out;
}

// ── Executions list ──────────────────────────────────────────────────

function ExecutionsList({
  executions,
  isLoading,
}: {
  executions: ExecutionRowWire[];
  isLoading: boolean;
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const toggle = (id: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  if (isLoading && executions.length === 0) {
    return <EmptyNote testId="verify-executions-loading">Loading executions…</EmptyNote>;
  }
  if (executions.length === 0) {
    return (
      <div data-testid="verify-executions-empty" style={{ padding: '32px 24px', textAlign: 'center' }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-primary)' }}>
          No executions recorded yet
        </div>
        <div
          style={{
            fontSize: 11.5,
            color: 'var(--text-secondary)',
            lineHeight: 1.6,
            maxWidth: 520,
            margin: '8px auto 0',
          }}
        >
          Every verdict so far is a desk check — computed against the model as written, recomputed
          on every edit, leaving no record. An execution is minted when a case runs against a
          simulation session (<span className="mono-text">∿ Run with simulation</span>, on the case
          view or the Matrix), or when an outside tool’s results are ingested.
        </div>
      </div>
    );
  }

  return (
    <div data-testid="verify-executions-list">
      {/* column headers */}
      <div
        className="mono-text flex items-center"
        style={{ height: 26, borderBottom: '1px solid var(--border-hairline)', fontSize: 10, color: 'var(--text-muted)' }}
      >
        <span style={{ width: 90, flex: 'none', padding: '0 12px' }}>when</span>
        <span style={{ width: 250, flex: 'none' }}>execution</span>
        <span style={{ flex: 1, minWidth: 160 }}>origin</span>
        <span style={{ width: 340, flex: 'none' }}>provenance</span>
        <span style={{ width: 170, flex: 'none', paddingRight: 12, textAlign: 'right' }}>results</span>
      </div>
      {executions.map((execution) => (
        <ExecutionRowLine
          key={execution.execution_id}
          execution={execution}
          expanded={expanded.has(execution.execution_id)}
          onToggle={() => toggle(execution.execution_id)}
        />
      ))}
      <div
        className="flex items-center px-3"
        style={{ minHeight: 24, fontSize: 10.5, color: 'var(--text-muted)', lineHeight: 1.4 }}
      >
        an execution is one recorded performance of cases in a context — static desk checks never
        appear here (they have no record) · Δ = the case’s definition moved since the execution ·
        ⚑ = the model the producer declared no longer matches
      </div>
    </div>
  );
}

export function ExecutionRowLine({
  execution,
  expanded,
  onToggle,
}: {
  execution: ExecutionRowWire;
  expanded: boolean;
  onToggle: () => void;
}) {
  const isExternal = execution.evaluation_mode === 'external';
  const external = execution.external ?? null;
  const digest7 = execution.provenance?.model_digest
    ? execution.provenance.model_digest.slice(0, 7)
    : null;
  const git7 = execution.provenance?.git?.commit ? execution.provenance.git.commit.slice(0, 7) : null;
  const manifest = execution.provenance?.file_manifest ?? [];
  const counts = execution.counts;

  return (
    <div
      data-testid={`verify-execution-${execution.execution_id}`}
      style={{ borderBottom: '1px solid var(--border-hairline)' }}
    >
      <div
        className="flex items-center"
        role="button"
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onToggle();
        }}
        style={{ height: 34, cursor: 'pointer', background: expanded ? 'var(--surface-panel)' : 'transparent' }}
      >
        <span className="mono-text" style={{ width: 90, flex: 'none', padding: '0 12px', fontSize: 10.5, color: 'var(--text-muted)' }}>
          <AgeText timestamp={execution.timestamp} suffix=" ago" />
        </span>
        <span style={{ width: 250, flex: 'none' }} className="flex items-center gap-2">
          <EvaluationModeBadge
            mode={isExternal ? 'external' : 'trajectory'}
            size="standard"
            recordRef={isExternal ? external?.tool ?? undefined : shortId(execution.execution_id)}
            stale={external?.matches_current_model === false}
          />
        </span>
        <span className="truncate" style={{ flex: 1, minWidth: 160, fontSize: 11, color: 'var(--text-secondary)' }}>
          {/* The raw SessionOrigin value ("run") reads as noise in UI copy —
              the unlabeled fallback says what the record IS. */}
          {execution.label ?? (isExternal ? 'ingested external results' : 'simulation session')}
        </span>
        <span className="mono-text flex items-center gap-1.5" style={{ width: 340, flex: 'none', fontSize: 10.5, color: 'var(--text-muted)' }}>
          {external ? (
            <>
              declared @ {external.declared_digest.slice(0, 7)}
              {external.matches_current_model === true ? (
                <span style={{ color: 'var(--verdict-pass)' }}>matches current</span>
              ) : external.matches_current_model === false ? (
                <span style={{ color: 'var(--severity-warning)' }}>≠ current</span>
              ) : null}
              {external.run_ref ? (
                <a
                  href={external.run_ref}
                  target="_blank"
                  rel="noreferrer"
                  onClick={(e) => e.stopPropagation()}
                  style={{ color: 'var(--accent-fg)', textDecoration: 'none' }}
                >
                  run ref ↗
                </a>
              ) : null}
            </>
          ) : (
            <>
              {digest7 ? <span title={execution.provenance?.model_digest ?? undefined}>@ {digest7}</span> : null}
              {git7 ? (
                <span>
                  · git {git7}
                  {execution.provenance?.git?.dirty ? ' (dirty)' : ''}
                </span>
              ) : null}
              {/* §6.2 manifest — count only on the row; the record itself
                  lives on expand. Absent/empty (pre-§6.2) renders NOTHING. */}
              {manifest.length > 0 ? (
                <span
                  data-testid={`verify-execution-manifest-count-${execution.execution_id}`}
                  title="Per-file provenance captured when the session was minted — expand for the file list."
                >
                  · {manifest.length} {manifest.length === 1 ? 'file' : 'files'}
                </span>
              ) : null}
            </>
          )}
        </span>
        <span className="mono-text flex items-center justify-end gap-1.5" style={{ width: 170, flex: 'none', paddingRight: 12, fontSize: 10.5 }}>
          {counts.pass > 0 ? <span style={{ color: 'var(--verdict-pass)' }}>{counts.pass} ✓</span> : null}
          {counts.fail > 0 ? <span style={{ color: 'var(--verdict-fail)' }}>{counts.fail} ✗</span> : null}
          {counts.inconclusive > 0 ? <span style={{ color: 'var(--verdict-inconclusive)' }}>{counts.inconclusive} ?</span> : null}
          {counts.error > 0 ? <span style={{ color: 'var(--verdict-error)' }}>{counts.error} ⚠</span> : null}
          <span style={{ color: 'var(--text-muted)' }}>{expanded ? '▾' : '▸'}</span>
        </span>
      </div>

      {expanded ? (
        <div data-testid={`verify-execution-results-${execution.execution_id}`} style={{ padding: '2px 0 8px' }}>
          {execution.results.map((result) => (
            <div key={`${result.case_id}`} className="flex items-center" style={{ minHeight: 26, fontSize: 11.5 }}>
              <span style={{ width: 90, flex: 'none' }} />
              <span className="mono-text truncate" style={{ width: 250, flex: 'none', fontSize: 11, color: 'var(--text-secondary)' }}>
                {result.case_id}
              </span>
              <span style={{ width: 110, flex: 'none' }}>
                <VerdictBadge
                  verdict={normalizeVerdict(result.verdict)}
                  size="bare"
                  showLabel
                  name={result.case_id}
                  testId={`verify-execution-result-${result.case_id}`}
                />
              </span>
              <span className="mono-text" style={{ width: 280, flex: 'none', fontSize: 10, color: 'var(--text-muted)' }}>
                {result.case_digest ? `case digest ${result.case_digest.slice(0, 7)} pinned at mint` : null}
              </span>
              {result.case_changed_since === true ? (
                <span data-testid={`verify-execution-changed-${result.case_id}`}>
                  <ChangedSinceFlag long />
                </span>
              ) : null}
              <span style={{ flex: 1 }} />
            </div>
          ))}
          <ProvenanceRecord
            provenance={execution.provenance}
            executionId={execution.execution_id}
          />
        </div>
      ) : null}
    </div>
  );
}

/**
 * The §6.2 "verified against" record — the per-file manifest captured at
 * session mint, rendered on execution expand. B10 geometry: a SOLID
 * square-cornered record (the tool minted this receipt itself — dashed
 * stays reserved for ingested external provenance). Renders NOTHING when
 * the manifest is empty/absent (pre-§6.2 records) — no null-state chips.
 */
export function ProvenanceRecord({
  provenance,
  executionId,
}: {
  provenance: ExecutionRowWire['provenance'];
  executionId: string;
}) {
  const files = provenance?.file_manifest ?? [];
  if (files.length === 0) return null;
  return (
    <div
      data-testid={`verify-execution-provenance-${executionId}`}
      className="mono-text"
      style={{
        margin: '6px 12px 4px 102px',
        padding: '6px 10px',
        maxWidth: 640,
        border: '1px solid var(--border-default)',
        borderRadius: 4, // square record, never a pill
        fontSize: 10,
        color: 'var(--text-muted)',
        lineHeight: 1.7,
      }}
    >
      <div style={{ letterSpacing: '0.04em', color: 'var(--text-secondary)' }}>
        VERIFIED AGAINST · {files.length} {files.length === 1 ? 'file' : 'files'}
        {provenance?.workspace_root ? (
          <span style={{ color: 'var(--text-muted)' }}> · {provenance.workspace_root}</span>
        ) : null}
      </div>
      {files.map((f) => (
        <div key={f.path} className="flex items-baseline gap-2">
          <span className="truncate" style={{ color: 'var(--text-secondary)' }}>
            {f.path}
          </span>
          <span title={f.content_hash}>{f.content_hash.slice(0, 7)}</span>
        </div>
      ))}
    </div>
  );
}

// ── small presentational bits ────────────────────────────────────────

function HistoryModeTab({
  id,
  label,
  active,
  onClick,
  title,
}: {
  id: HistoryMode;
  label: string;
  active: boolean;
  onClick: () => void;
  title?: string;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      title={title}
      data-testid={`verify-history-mode-${id}`}
      onClick={onClick}
      style={{
        padding: '3px 10px',
        fontSize: 11,
        border: 'none',
        cursor: 'pointer',
        background: active ? 'var(--surface-raised)' : 'transparent',
        color: active ? 'var(--text-primary)' : 'var(--text-muted)',
      }}
    >
      {label}
    </button>
  );
}

function ExecutionsSummary({ executions }: { executions: ExecutionRowWire[] }) {
  if (executions.length === 0) return null;
  const newest = executions[0];
  return (
    <span className="mono-text" style={{ fontSize: 10.5, color: 'var(--text-muted)' }} data-testid="verify-history-summary">
      {executions.length} {executions.length === 1 ? 'execution' : 'executions'} · latest{' '}
      <AgeText timestamp={newest.timestamp} suffix=" ago" plain />
    </span>
  );
}

/** A record reference (session id / producing tool) — quiet mono text
 *  beside the geometry mark; real data, so held at muted (never below the
 *  WCAG floor — calm-pass brief §4). */
function RefText({ children }: { children: ReactNode }) {
  return (
    <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>
      {children}
    </span>
  );
}

function AgeText({
  timestamp,
  suffix = '',
  plain = false,
}: {
  timestamp: number;
  suffix?: string;
  plain?: boolean;
}) {
  const age = relativeAge(timestamp);
  // "now ago" is not a phrase — the suffix only reads with a real delta.
  const label = age === 'now' ? age : `${age}${suffix}`;
  const full = new Date(timestamp).toLocaleString();
  if (plain) return <span title={full}>{label}</span>;
  return (
    <span className="mono-text" title={full} style={{ fontSize: 10, color: 'var(--text-disabled)', whiteSpace: 'nowrap' }}>
      {label}
    </span>
  );
}

/** `⚑` — the external staleness flag (warning family): the producer's
 *  declared model digest ≠ the current model. Bare glyph in the dense
 *  table, full phrase on hover (calm-pass P4). A DIFFERENT question than
 *  Δ, which is the per-case drift. */
function StaleFlag() {
  return (
    <span
      title="Produced against an older model — the digest the producer claims it tested no longer matches the current model."
      style={{ fontSize: 10, color: 'var(--severity-warning)', whiteSpace: 'nowrap' }}
    >
      ⚑ older model
    </span>
  );
}

/** `Δ case edited since` — the per-case drift flag (warning family; a
 *  DIFFERENT question than ⚑, which is the producer's whole-model claim). */
function ChangedSinceFlag({ long = false }: { long?: boolean }) {
  return (
    <span
      title="This case's definition changed after this execution ran — the result was produced against an older version of the case."
      style={{ fontSize: 10, color: 'var(--severity-warning)', whiteSpace: 'nowrap' }}
    >
      Δ {long ? 'case edited since this execution' : 'case edited since'}
    </span>
  );
}

function FacetChip({
  id,
  label,
  active,
  onClick,
}: {
  id: Facet;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={`verify-facet-${id}`}
      data-active={active || undefined}
      onClick={onClick}
      className="mono-text"
      style={{
        fontSize: 10.5,
        padding: '2px 10px',
        borderRadius: 999,
        cursor: 'pointer',
        // De-bordered until active (calm-pass P2): an idle facet is quiet
        // text; the outline appears only on the chip that is ON.
        background: active ? 'var(--surface-raised)' : 'transparent',
        color: active ? 'var(--text-primary)' : 'var(--text-muted)',
        border: `1px solid ${active ? 'var(--border-strong)' : 'transparent'}`,
      }}
    >
      {label}
    </button>
  );
}

function EmptyNote({ children, testId }: { children: ReactNode; testId: string }) {
  return (
    <div data-testid={testId} style={emptyNoteStyle}>
      {children}
    </div>
  );
}

const emptyNoteStyle: CSSProperties = {
  padding: '20px 24px',
  fontSize: 11.5,
  color: 'var(--text-muted)',
  lineHeight: 1.5,
};

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 8)}…` : id;
}
