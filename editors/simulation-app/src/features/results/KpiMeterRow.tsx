/**
 * KpiMeterRow + ConstraintChip — the bottom strip's meta row (ninebar
 * Phase 3 W3-A / W3-B; plan §0 strip slot: "waveform + a compact
 * ninebar KPI row + constraint summary chip").
 *
 * KPI chips: name · live numeral (mono/tabular) · unit · `<Ninebar
 * value/>` when a comparator+threshold exist — meter fill is
 * clamp(value / threshold), "measure vs target", which satisfies the
 * Ninebar contract (a live measure, never decoration). The pass/fail
 * dot uses `--verdict-*` (a KPI verdict IS a verdict). The row is
 * read-only; the "+" / chip click opens the KPI manager MODAL (the old
 * workbench tab re-homed, same pattern as equations — plan §1 row 4b).
 *
 * Definitions reuse `KpisTab`'s storage + evaluation verbatim
 * (session-scoped localStorage; re-read per render — the row renders
 * once per tick via `useTick`, and the JSON is tiny).
 *
 * ConstraintChip: `n/N` counts off `snapshot.constraint_results`,
 * subscribed via a STRING-summary selector so frame-rate deltas that
 * don't change the rollup never re-render it (F15). Failing names live
 * in a popover with an "Open in Verify" drill.
 */
import { useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Ninebar } from '@/components/Ninebar';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionLiveStore, useTick } from '@/features/sessions/sessionLiveStore';
import type { ConstraintVerdictKind } from '@/features/sessions/sessionLiveStore';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useModalStore } from '@/shared/overlays/modalStore';
import { Popover } from '@/shared/overlays/Popover';
import {
  evaluateKpi,
  readStoredDefinitions,
  type KpiResult,
} from '@/features/results/kpis/KpisTab';
import { KPI_MANAGER_MODAL_ID } from '@/features/results/kpis/KpiManagerModal';

const VERDICT_DOT: Record<KpiResult['verdict'], string> = {
  pass: 'var(--verdict-pass)',
  fail: 'var(--verdict-fail)',
  unknown: 'var(--text-disabled)',
};

function meterValue(result: KpiResult): number | null {
  const { threshold } = result.definition;
  if (result.value === null || threshold === undefined || threshold === 0) return null;
  return Math.max(0, Math.min(1, result.value / threshold));
}

function formatValue(v: number | null): string {
  if (v === null) return '—';
  if (Math.abs(v) >= 1000 || (Math.abs(v) < 0.01 && v !== 0)) return v.toExponential(2);
  return Number(v.toFixed(3)).toString();
}

export function KpiMeterRow() {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  // One render per tick — the cadence at which values can change.
  useTick();

  const definitions = readStoredDefinitions(sessionId) ?? [];
  const timeSeries = useTimeSeriesStore.getState().getTimeSeries();
  const results = definitions.map((d) => evaluateKpi(d, timeSeries));

  const openManager = () => useModalStore.getState().openModal(KPI_MANAGER_MODAL_ID);

  return (
    <div data-testid="kpi-meter-row" className="flex items-center gap-2 min-w-0 overflow-hidden">
      {results.map((r) => (
        <button
          key={r.definition.id}
          type="button"
          data-testid={`kpi-chip-${r.definition.id}`}
          onClick={openManager}
          title={`${r.definition.name} — ${r.definition.aggregator}(${r.definition.variable})${
            r.definition.threshold !== undefined
              ? ` ${r.definition.comparator ?? ''} ${r.definition.threshold}`
              : ''
          }`}
          className="inline-flex items-center gap-1.5 shrink-0"
          style={{
            height: 'var(--row-compact)',
            padding: '0 8px',
            background: 'none',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            cursor: 'pointer',
          }}
        >
          <span
            aria-hidden
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: VERDICT_DOT[r.verdict],
            }}
          />
          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)' }}>
            {r.definition.name}
          </span>
          <span className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
            {formatValue(r.value)}
          </span>
          {r.definition.unit && (
            <span className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
              {r.definition.unit}
            </span>
          )}
          {meterValue(r) !== null && <Ninebar compact value={meterValue(r)!} size={10} label={`${r.definition.name} vs target`} />}
        </button>
      ))}

      <button
        type="button"
        data-testid="kpi-add"
        onClick={openManager}
        title="Manage KPIs"
        className="material-symbols-outlined shrink-0"
        style={{
          fontSize: 14,
          width: 24,
          height: 22,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--text-secondary)',
          background: 'none',
          border: '1px dashed var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          cursor: 'pointer',
        }}
      >
        add
      </button>
    </div>
  );
}

// ── Constraint summary chip (W3-B) ──────────────────────────────────

/** One verdict group in the chip popover: a labelled heading plus the
 *  constraint names under it. Used for both failing and undecided so the
 *  two read as peers rather than one being the "real" list. */
function VerdictGroup(props: {
  testId: string;
  label: string;
  names: string[];
  color: string;
  hint?: string;
}) {
  return (
    <div data-testid={props.testId} className="flex flex-col gap-0.5">
      <span
        style={{
          fontSize: 'var(--text-xs)',
          color: props.color,
          padding: '2px 4px',
          fontWeight: 600,
        }}
      >
        {props.label}
      </span>
      {props.hint && (
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', padding: '0 4px 2px' }}>
          {props.hint}
        </span>
      )}
      {props.names.map((name, i) => (
        <span
          key={`${name}-${i}`}
          className="mono-text"
          style={{ fontSize: 'var(--text-sm)', color: props.color, padding: '2px 4px 2px 10px' }}
        >
          {name}
        </span>
      ))}
    </div>
  );
}

/** Display label for a constraint row.
 *
 *  Most constraints are anonymous — on the espresso workspace 9 of 11 arrive
 *  with an empty `name`, because only `assert constraint` usages are named
 *  and `require constraint` usages carry their text in `expression`. Mirrors
 *  `ConstraintCard`'s own name → expression → positional fallback so the two
 *  surfaces label the same row the same way. */
function rowLabel(r: { name: string; expression?: string | null }, i: number): string {
  return r.name?.trim() || r.expression?.trim() || `constraint ${i + 1}`;
}

/** String-summary selector: re-renders only when the rollup changes.
 *
 *  Three independent quantities come out of here, and all three are needed in
 *  every render branch. Only a decided `fail` (or `error`) counts as failing;
 *  an `inconclusive` row established nothing, so it is neither passing nor
 *  failing. Collapsing it into either one is what "48 failing" was doing when
 *  the wire carried a bare boolean.
 *
 *  Encoded as JSON rather than delimiter-joined. The predecessor flattened the
 *  name lists with a separator and rebuilt them with `list ? list.split(FS) :
 *  []` — which cannot distinguish "no rows" from "rows whose names are all
 *  empty". With 9 of 11 espresso constraints anonymous, a single unnamed
 *  FAILING row joined to `''`, read back as zero failures, and disappeared:
 *  the chip showed `3/11 · 7 undecided`, 3 + 7 = 10, eleventh row gone. A
 *  count must never be inferred from the truthiness of a flattened string.
 *  JSON round-trips losslessly and is still a stable string for the store's
 *  equality check. */
export function constraintSummary(s: {
  snapshot: {
    constraint_results: Array<{
      name: string;
      expression?: string | null;
      verdict: ConstraintVerdictKind;
    }>;
  } | null;
}): string {
  const rows = s.snapshot?.constraint_results ?? [];
  if (rows.length === 0) return '';
  // One pass, every row assigned to exactly one bucket. Three independent
  // filters let a row match none of them and vanish from the totals — which
  // is how the unnamed failing row went missing — so the invariant
  // `pass + failing + undecided === total` is enforced by construction here
  // rather than only asserted in a test.
  //
  // An unrecognised verdict lands in `undecided`, the bucket that claims
  // nothing. Case-folded because `VerdictKind` genuinely has two spellings in
  // this system (serde PascalCase on snapshots, `Display` lowercase on the
  // archive and CLI — punch-list finding 37), and the sibling consumers
  // (mergeLiveState, ResultsWorkbench, selectors) all fold. The chip was the
  // one place still comparing exact PascalCase.
  let pass = 0;
  const failing: string[] = [];
  const undecided: string[] = [];
  rows.forEach((r, i) => {
    const label = rowLabel(r, i);
    switch (String(r.verdict).toLowerCase()) {
      case 'pass':
        pass += 1;
        break;
      case 'fail':
      case 'error':
        failing.push(label);
        break;
      default:
        undecided.push(label);
    }
  });
  return JSON.stringify({
    pass,
    total: rows.length,
    failing,
    undecided,
  } satisfies ConstraintChipCounts);
}

/** Inverse of {@link constraintSummary}. Exported so tests read the rollup the
 *  same way the component does, rather than re-deriving the encoding. */
export function parseSummary(summary: string): ConstraintChipCounts | null {
  if (!summary) return null;
  return JSON.parse(summary) as ConstraintChipCounts;
}

export interface ConstraintChipCounts {
  pass: number;
  total: number;
  failing: string[];
  undecided: string[];
}

/**
 * Decide how the chip reads and paints. Pure, so the three-state behaviour
 * can be pinned without rendering.
 *
 * Three states, not two. The predecessor branched on a boolean named
 * `allPass` that actually meant "nothing failed" — two distinct bugs fell
 * out of that single flag:
 *
 *   - 0 fail + N undecided painted the calm neutral chip, reporting
 *     constraints the run never evaluated as though everything were fine.
 *     The more dangerous direction, because nothing prompts a second look.
 *   - the moment anything DID fail, the undecided count vanished from the
 *     label entirely (`1 failing · 3/11`), so the user lost it exactly when
 *     they had most reason to want the full picture.
 *
 * Undecided therefore gets its own colour and its own term in the label, in
 * every branch.
 */
export function constraintChipView(parsed: ConstraintChipCounts) {
  const hasFailures = parsed.failing.length > 0;
  const hasUndecided = parsed.undecided.length > 0;
  return {
    hasFailures,
    hasUndecided,
    accent: hasFailures
      ? 'var(--verdict-fail)'
      : hasUndecided
        ? 'var(--verdict-inconclusive)'
        : 'var(--text-secondary)',
    border: hasFailures
      ? 'var(--verdict-fail)'
      : hasUndecided
        ? 'var(--verdict-inconclusive)'
        : 'var(--border-default)',
    // Every non-zero category is named, worst-first, so none is dropped.
    label: [
      hasFailures ? `${parsed.failing.length} failing` : null,
      hasUndecided ? `${parsed.undecided.length} undecided` : null,
      `${parsed.pass}/${parsed.total}${hasFailures || hasUndecided ? '' : ' constraints'}`,
    ]
      .filter(Boolean)
      .join(' · '),
  };
}

export function ConstraintChip() {
  const summary = useSessionLiveStore(constraintSummary);
  const navigate = useNavigate();
  const anchorRef = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);

  const parsed = useMemo(() => parseSummary(summary), [summary]);

  if (!parsed) return null;
  const { hasFailures, hasUndecided, accent, border, label } = constraintChipView(parsed);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        data-testid="constraint-chip"
        onClick={() => setOpen(true)}
        title="Constraint satisfaction — click for details"
        className="mono-text inline-flex items-center gap-1.5 shrink-0"
        style={{
          height: 'var(--row-compact)',
          padding: '0 8px',
          fontSize: 'var(--text-xs)',
          color: accent,
          background: 'none',
          border: `1px solid ${border}`,
          borderRadius: 'var(--radius-sm)',
          cursor: 'pointer',
        }}
      >
        {label}
      </button>

      <Popover anchorEl={anchorRef.current} open={open} onClose={() => setOpen(false)} placement="top">
        <div data-testid="constraint-chip-popover" className="flex flex-col gap-1" style={{ padding: 8, minWidth: 220 }}>
          {!hasFailures && !hasUndecided && (
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', padding: '2px 4px' }}>
              All constraints satisfied at the current tick.
            </span>
          )}

          {/* Both groups are always listed when non-empty. Naming only the
              failures hid the undecided rows from the one surface a user
              opens to find out what is wrong. */}
          {hasFailures && (
            <VerdictGroup
              testId="constraint-chip-failing"
              label={`Failing (${parsed.failing.length})`}
              names={parsed.failing}
              color="var(--verdict-fail)"
            />
          )}
          {hasUndecided && (
            <VerdictGroup
              testId="constraint-chip-undecided"
              label={`Undecided (${parsed.undecided.length})`}
              names={parsed.undecided}
              color="var(--verdict-inconclusive)"
              hint="Not evaluated — parameters unbound. Neither satisfied nor violated."
            />
          )}
          <button
            type="button"
            data-testid="constraint-chip-open-verify"
            onClick={() => {
              setOpen(false);
              navigate('/verify');
            }}
            style={{
              alignSelf: 'flex-end',
              marginTop: 4,
              background: 'none',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              color: 'var(--text-secondary)',
              padding: '3px 10px',
              fontSize: 'var(--text-xs)',
              cursor: 'pointer',
            }}
          >
            Open in Verify
          </button>
        </div>
      </Popover>
    </>
  );
}
