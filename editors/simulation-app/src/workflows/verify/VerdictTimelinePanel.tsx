/**
 * VerdictTimelinePanel — the case-scoped **process log**: verdict records
 * and signed attestations for one case, as a newest-first LIST.
 *
 * ## Why a list, not a time axis (calm pass, turn 4 — register item iv)
 *
 * This was a horizontal time axis with one lane per case and absolutely-
 * positioned markers + an attestation strip. Two problems the calm pass
 * retires: (1) the attestation labels smudged into each other whenever
 * acts clustered in time (the axis packs them by timestamp, so a busy day
 * overlaps), and (2) an axis implies metric spacing this data does not
 * earn — it is a register of acts, not a density chart. A newest-first
 * list orders honestly by recency, lets each line breathe, and carries the
 * same geometry channel as a leading-or-trailing mark:
 *
 *   · trajectory record — a SOLID square (a run's own receipt: session, tick)
 *   · external record   — a DASHED square (ingested provenance) + ⚑ if stale
 *   · attestation       — a round dot (a signed human act, never a verdict)
 *
 * Verdict colour still appears ONLY on the verdict glyph. Drill is
 * preserved: a trajectory record activates `onVerdictSelect` (→ Simulate @
 * tick); an external record opens its `run_ref`.
 *
 * ## Data source
 *
 * `sysml.verify.timeline` (verdict records) + `sysml.workflow.state`
 * (attestations, via `useVerificationAttestations`), merged and sorted by
 * timestamp descending. Static verdicts have no archive and never appear.
 */

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { VerdictBadge, normalizeVerdict, type VerdictKind } from '@/components/VerdictBadge';
import { EvaluationModeBadge } from '@/components/EvaluationModeBadge';
import {
  useVerificationAttestations,
  type CaseVerificationAttestation,
} from '@/features/workflow/queries';
import { relativeAge } from './useExecutionHistory';

// ── Wire types — mirror `crates/tooling/sysml-service/src/verify_timeline.rs`
//
// When the backend module evolves, keep these in sync. The contract is
// locked by the Rust-side `response_serializes_to_expected_json_shape`
// unit test.

export interface VerdictTimelineEvidence {
  session_id: string;
  tick: number;
  element_id?: string | null;
}

/**
 * External-ingestion provenance (B10) carried by an `evaluation_mode:
 * "external"` entry — recorded, never computed here. `matches_current_model`
 * is SERVER-computed staleness: render `⚑ older model` whenever it is false.
 */
export interface VerdictTimelineExternal {
  tool: string;
  declared_digest?: string | null;
  run_ref?: string | null;
  artifacts?: string[] | null;
  element_id?: string | null;
  /** false ⇒ the digest the producer claims it tested ≠ the current model. */
  matches_current_model: boolean;
}

export interface VerdictTimelineEntry {
  session_id: string;
  /** Unix millisecond timestamp. */
  timestamp: number;
  case_id: string;
  /** Lowercase `pass`/`fail`/`inconclusive`/`error`. */
  verdict: VerdictKind | string;
  /** How the verdict was computed (B10 layer 2). Static verdicts have no
   *  archive and never appear here; external entries carry `external`. */
  evaluation_mode?: string | null;
  evidence?: VerdictTimelineEvidence | null;
  external?: VerdictTimelineExternal | null;
}

export interface VerdictTimelineResponse {
  entries: VerdictTimelineEntry[];
}

/** True when an entry is an external ingestion (dashed provenance). */
export function isExternalEntry(entry: VerdictTimelineEntry): boolean {
  return String(entry.evaluation_mode).toLowerCase() === 'external' || !!entry.external;
}

/**
 * Static verdicts have NO archive (their evidence is the current model
 * itself), so they never enter the log. The backend does not archive
 * them; this filter keeps the FE honest if one ever leaks through.
 */
export function isArchivableEntry(entry: VerdictTimelineEntry): boolean {
  return String(entry.evaluation_mode).toLowerCase() !== 'static';
}

// ── Request helper ────────────────────────────────────────────────────

export interface VerdictTimelineRequest {
  case_ids?: string[];
  since_timestamp?: number;
  /**
   * Gate: only fetch once a workspace is loaded. The request carries no
   * workspace identity — scoping is SERVER-SIDE: the backend keys the
   * log on each archived session's provenance `workspace_root` (B6)
   * against its own resolved root, so the FE never needs to (and cannot
   * correctly) supply one.
   */
  enabled: boolean;
}

export async function fetchVerdictTimeline(
  req: Pick<VerdictTimelineRequest, 'case_ids' | 'since_timestamp'>,
): Promise<VerdictTimelineResponse> {
  return httpPost<VerdictTimelineResponse>('/api/command', {
    command: 'sysml.verify.timeline',
    params: {
      case_ids: req.case_ids ?? null,
      since_timestamp: req.since_timestamp ?? null,
    },
  });
}

// ── Query key + hook ──────────────────────────────────────────────────

export const verdictTimelineKeys = {
  all: ['verdict-timeline'] as const,
  forArchive: (caseIds?: string[], since?: number) =>
    [
      ...verdictTimelineKeys.all,
      // Normalise case_ids so reordering doesn't invalidate the cache.
      caseIds ? [...caseIds].sort() : null,
      since ?? null,
    ] as const,
};

export function useVerdictTimeline(req: VerdictTimelineRequest) {
  return useQuery({
    queryKey: verdictTimelineKeys.forArchive(req.case_ids, req.since_timestamp),
    queryFn: () => fetchVerdictTimeline(req),
    enabled: req.enabled,
    staleTime: 30_000,
  });
}

// ── Tooltips (pure, exported for tests) ───────────────────────────────

/**
 * Build the tooltip for a record — matches the contract the test asserts
 * against. Exported for test reuse.
 */
export function buildMarkerTooltip(entry: VerdictTimelineEntry): string {
  const when = new Date(entry.timestamp).toISOString();
  return `${entry.case_id} — ${String(entry.verdict).toLowerCase()} @ ${when} (session ${entry.session_id})`;
}

/** External entries carry the producing tool + a ci run ref, not a session. */
export function buildExternalTooltip(entry: VerdictTimelineEntry): string {
  const when = new Date(entry.timestamp).toISOString();
  const tool = entry.external?.tool ?? 'external';
  const stale = entry.external?.matches_current_model === false ? ' · produced against an older model' : '';
  return `${entry.case_id} — ${String(entry.verdict).toLowerCase()} @ ${when} · external evidence from ${tool} (ingested, not computed here)${stale}`;
}

// ── Merged process-event model ────────────────────────────────────────

type ProcessEvent =
  | { kind: 'record'; ts: number; entry: VerdictTimelineEntry }
  | { kind: 'attestation'; ts: number; att: CaseVerificationAttestation };

/** Merge verdict records + attestations into ONE newest-first list. Pure,
 *  exported for tests. */
export function mergeProcessEvents(
  entries: VerdictTimelineEntry[],
  attestations: CaseVerificationAttestation[],
): ProcessEvent[] {
  const events: ProcessEvent[] = [
    ...entries.map((entry): ProcessEvent => ({ kind: 'record', ts: entry.timestamp, entry })),
    ...attestations.map((att): ProcessEvent => ({ kind: 'attestation', ts: att.timestamp_ms, att })),
  ];
  // Newest first. Ties keep records before attestations (records read as the
  // concrete act, attestations as the sign-off on it).
  return events.sort((a, b) => b.ts - a.ts || (a.kind === b.kind ? 0 : a.kind === 'record' ? -1 : 1));
}

// ── Component ─────────────────────────────────────────────────────────

export interface VerdictTimelinePanelProps {
  /** Optional restriction to specific verification cases. */
  caseIds?: string[];
  /**
   * Element ids of the cases in view — the targets whose verification
   * attestations feed the log. Attestations are folded per element
   * (`sysml.workflow.state`); the log is the union across these. Absent/empty
   * ⇒ no attestations (honest empty), never a fabricated act.
   */
  caseElementIds?: string[];
  /** Optional Unix-ms lower bound. */
  sinceTimestamp?: number;
  /**
   * Called when a user activates a trajectory record (click or Enter/Space) —
   * opens RunWorkflow at the evidence tick.
   */
  onVerdictSelect?: (entry: VerdictTimelineEntry) => void;
  /** Optional testid passthrough for integration tests. */
  testId?: string;
}

export function VerdictTimelinePanel(props: VerdictTimelinePanelProps) {
  const { caseIds, caseElementIds, sinceTimestamp, onVerdictSelect, testId } = props;

  const query = useVerdictTimeline({
    case_ids: caseIds,
    since_timestamp: sinceTimestamp,
    enabled: true,
  });

  // Attestations for the cases in view — reuses the ONE workflow-state read.
  const { attestations } = useVerificationAttestations(caseElementIds ?? []);

  // Static verdicts have no archive and never appear (footnote below).
  const entries = useMemo(
    () => (query.data?.entries ?? []).filter(isArchivableEntry),
    [query.data],
  );
  const events = useMemo(
    () => mergeProcessEvents(entries, attestations),
    [entries, attestations],
  );

  const rootTestId = testId ?? 'verdict-timeline-panel';

  if (query.isLoading) {
    return (
      <div role="status" data-testid={`${rootTestId}-loading`} style={containerStyle}>
        Loading process log…
      </div>
    );
  }

  if (query.isError) {
    return (
      <div
        role="alert"
        data-testid={`${rootTestId}-error`}
        style={{ ...containerStyle, color: 'var(--severity-error)' }}
      >
        Failed to load process log: {(query.error as Error)?.message ?? 'unknown error'}
      </div>
    );
  }

  if (events.length === 0) {
    return (
      <div
        role="status"
        data-testid={`${rootTestId}-empty`}
        style={{ ...containerStyle, ...emptyStyle }}
      >
        <div style={{ fontWeight: 600, marginBottom: 4 }}>No recorded acts yet</div>
        <div style={{ color: 'var(--text-muted)' }}>
          Runs, ingested external results, and sign-offs appear here newest first.
        </div>
      </div>
    );
  }

  return (
    <div
      data-testid={rootTestId}
      style={containerStyle}
      aria-label="Process log — verdict records and attestations, newest first"
    >
      <div role="list" style={{ display: 'flex', flexDirection: 'column' }}>
        {events.map((event, idx) =>
          event.kind === 'record' ? (
            <RecordRow
              key={`rec-${event.entry.session_id}-${event.ts}-${idx}`}
              entry={event.entry}
              rootTestId={rootTestId}
              index={idx}
              onVerdictSelect={onVerdictSelect}
            />
          ) : (
            <AttestationRow
              key={`att-${event.att.element_id}-${event.att.seq}`}
              att={event.att}
              rootTestId={rootTestId}
              index={idx}
            />
          ),
        )}
      </div>

      <div style={footnoteStyle} data-testid={`${rootTestId}-footnote`}>
        newest first · static verdicts never appear (they have no archive) · ⚑ = the
        declared digest ≠ the current model · a dot is a signed act, not a verdict
      </div>
    </div>
  );
}

// ── Rows ──────────────────────────────────────────────────────────────

function RecordRow({
  entry,
  rootTestId,
  index,
  onVerdictSelect,
}: {
  entry: VerdictTimelineEntry;
  rootTestId: string;
  index: number;
  onVerdictSelect?: (entry: VerdictTimelineEntry) => void;
}) {
  const external = isExternalEntry(entry);
  const verdict = normalizeVerdict(entry.verdict);
  const stale = external && entry.external?.matches_current_model === false;
  const runRef = entry.external?.run_ref ?? null;
  const tooltip = external ? buildExternalTooltip(entry) : buildMarkerTooltip(entry);
  const ref = external ? entry.external?.tool ?? 'external' : entry.session_id.slice(0, 8);
  const tick = entry.evidence?.tick;

  const activate = () => {
    // External records drill to their run ref (never computed here);
    // trajectory records drill to Simulate @ tick.
    if (external && runRef) {
      window.open(runRef, '_blank', 'noopener,noreferrer');
      return;
    }
    if (!external) onVerdictSelect?.(entry);
  };
  const clickable = external ? !!runRef : !!onVerdictSelect;

  return (
    <div
      role="listitem"
      data-testid={`${rootTestId}-record-${index}`}
      data-external={external || undefined}
      data-verdict={verdict}
      data-tool={external ? entry.external?.tool : undefined}
      data-stale={stale || undefined}
      tabIndex={clickable ? 0 : undefined}
      title={tooltip}
      onClick={clickable ? activate : undefined}
      onKeyDown={
        clickable
          ? (ev) => {
              if (ev.key === 'Enter' || ev.key === ' ') {
                ev.preventDefault();
                activate();
              }
            }
          : undefined
      }
      style={{ ...rowStyle, cursor: clickable ? 'pointer' : 'default' }}
    >
      <AgeCell ts={entry.timestamp} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={titleLineStyle}>
          <span style={{ color: 'var(--text-primary)' }}>
            {external ? 'external attested' : 'run recorded'}
          </span>
          <VerdictBadge verdict={verdict} size="bare" showLabel titleOverride={tooltip} testId={`${rootTestId}-badge-${index}`} />
          {stale ? (
            <span
              data-testid={`${rootTestId}-stale-${index}`}
              style={{ fontSize: 10, color: 'var(--severity-warning)', whiteSpace: 'nowrap' }}
            >
              ⚑ older model
            </span>
          ) : null}
        </div>
        <div style={subLineStyle}>
          {external ? `ingested · ${ref}` : `${ref}${tick != null ? ` · tick ${tick}` : ''}`}
          {clickable ? (
            <span style={{ color: 'var(--accent-fg)', marginLeft: 8 }}>
              {external ? 'run ref ↗' : 'open in Simulate ↗'}
            </span>
          ) : null}
        </div>
      </div>
      <EvaluationModeBadge mode={external ? 'external' : 'trajectory'} size="mark" stale={stale} />
    </div>
  );
}

function AttestationRow({
  att,
  rootTestId,
  index,
}: {
  att: CaseVerificationAttestation;
  rootTestId: string;
  index: number;
}) {
  const title =
    `${att.actor} attested ${att.method}` +
    (att.statement ? ` — “${att.statement}”` : '') +
    ` · @ ${att.attested_commit}` +
    (att.superseded ? ' · superseded (content moved past the attested commit)' : '') +
    ' · a signed act, not a verdict';
  return (
    <div
      role="listitem"
      data-testid={`${rootTestId}-attestation-${index}`}
      data-actor={att.actor}
      data-method={att.method}
      data-superseded={att.superseded || undefined}
      title={title}
      style={rowStyle}
    >
      <AgeCell ts={att.timestamp_ms} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={titleLineStyle}>
          <span
            style={{
              color: att.superseded ? 'var(--text-muted)' : 'var(--text-primary)',
              textDecoration: att.superseded ? 'line-through' : 'none',
            }}
          >
            ✎ attested
          </span>
          <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
            {att.actor} · {att.method}
            {att.superseded ? ' · superseded' : ''}
          </span>
        </div>
        {att.statement ? (
          <div style={subLineStyle}>“{att.statement}”</div>
        ) : null}
      </div>
      {/* A signed human act — a round dot, never a square record, never a
          verdict colour. */}
      <span
        aria-hidden="true"
        style={{
          width: 8,
          height: 8,
          flex: 'none',
          borderRadius: '50%',
          boxSizing: 'border-box',
          border: '1.5px solid var(--text-muted)',
          background: att.superseded ? 'transparent' : 'var(--text-muted)',
        }}
      />
    </div>
  );
}

/** Relative age in the fixed left gutter — quiet mono, the demoted tier. */
function AgeCell({ ts }: { ts: number }) {
  return (
    <span
      className="mono-text"
      title={new Date(ts).toLocaleString()}
      style={{ width: 52, flex: 'none', fontSize: 10, color: 'var(--text-muted)' }}
    >
      {relativeAge(ts)}
    </span>
  );
}

// ── Styles ─────────────────────────────────────────────────────────────

const containerStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
  padding: 12,
  borderRadius: 8,
  background: 'color-mix(in srgb, currentColor 4%, transparent)',
  border: '1px solid color-mix(in srgb, currentColor 12%, transparent)',
  fontSize: 12,
};

const emptyStyle: React.CSSProperties = {
  alignItems: 'center',
  justifyContent: 'center',
  textAlign: 'center',
  minHeight: 96,
};

const rowStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'baseline',
  gap: 10,
  padding: '7px 4px',
  borderBottom: '1px solid color-mix(in srgb, currentColor 10%, transparent)',
};

const titleLineStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  fontSize: 12,
  flexWrap: 'wrap',
};

const subLineStyle: React.CSSProperties = {
  marginTop: 2,
  fontFamily: 'var(--font-mono)',
  fontSize: 10,
  color: 'var(--text-muted)',
};

const footnoteStyle: React.CSSProperties = {
  fontSize: 10,
  lineHeight: 1.4,
  color: 'var(--text-muted)',
  borderTop: '1px solid color-mix(in srgb, currentColor 12%, transparent)',
  paddingTop: 6,
  marginTop: 2,
};
