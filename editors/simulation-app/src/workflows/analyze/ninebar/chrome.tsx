/**
 * Analyze ninebar chrome — the rail scaffold + hero notices shared by
 * the four flag-on method bodies (Sweep / Monte Carlo / Trade Study /
 * Sensitivity), ninebar Phase 5.
 *
 * One home for the crib-sheet 3a rail composition (header → titled list
 * section with a Configure action → summary footer → Run) and the hero
 * empty/running/error notices, so the per-method bodies only supply
 * their content. Repo principle 4: the second copy-paste is the signal
 * to extract.
 */

import { useMemo, type ReactNode } from 'react';
import { Ninebar } from '@/components/Ninebar';
import { useSessionQuota } from '@/features/sessions/queries';

// ── Rail scaffold ───────────────────────────────────────────────────

export interface RailSummaryEntry {
  label: string;
  value: string;
  /** 'fail' renders the value in the verdict-fail colour. */
  tone?: 'fail';
}

export interface AnalyzeRailProps {
  /** Material Symbols icon for the header. */
  icon: string;
  /** Method title, e.g. "Sweep". */
  title: string;
  /** Right-aligned mono count in the header, e.g. "2 factors". */
  headerCount: string;
  /** Uppercase label of the list section, e.g. "Factors". */
  sectionTitle: string;
  /** Opens the method's config modal. */
  onConfigure: () => void;
  /** List body — rows or the empty hint. */
  children: ReactNode;
  /** Footer rows (crib 3a: combinations / evaluated / failing …). */
  summary: RailSummaryEntry[];
  /**
   * Planned child-session count — when it exceeds the free quota slots
   * a warning line renders (quota surfacing per the existing
   * QuotaChip/`useSessionQuota` pattern). Pass 0 to skip.
   */
  plannedChildren: number;
  /** What the planned units are called in the warning, e.g. "children". */
  plannedNoun?: string;
  runLabel: string;
  canRun: boolean;
  isRunning: boolean;
  onRun: () => void;
  testIdPrefix: string;
}

export function AnalyzeRail({
  icon,
  title,
  headerCount,
  sectionTitle,
  onConfigure,
  children,
  summary,
  plannedChildren,
  plannedNoun = 'children',
  runLabel,
  canRun,
  isRunning,
  onRun,
  testIdPrefix,
}: AnalyzeRailProps) {
  const { data: quota } = useSessionQuota();
  const quotaShort = useMemo(() => {
    if (!quota || plannedChildren <= 0) return null;
    const used = (quota.simulation?.used ?? 0) + (quota.action?.used ?? 0) + (quota.orchestrator?.used ?? 0);
    const cap = (quota.simulation?.cap ?? 0) + (quota.action?.cap ?? 0) + (quota.orchestrator?.cap ?? 0);
    if (cap <= 0) return null;
    return plannedChildren > cap - used ? { need: plannedChildren, free: cap - used } : null;
  }, [quota, plannedChildren]);

  const runEnabled = canRun && !isRunning;

  return (
    <div data-testid={`${testIdPrefix}-rail`} className="flex flex-col h-full min-h-0" style={{ color: 'var(--text-primary)' }}>
      <div className="flex items-center gap-2 px-3 shrink-0" style={{ height: 32, borderBottom: '1px solid var(--border-hairline)' }}>
        <span className="material-symbols-outlined" style={{ fontSize: 15, color: 'var(--accent-fg)' }}>{icon}</span>
        <span style={{ fontSize: 11, fontWeight: 600 }}>{title}</span>
        <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-muted)', marginLeft: 'auto' }}>{headerCount}</span>
      </div>

      <section className="flex flex-col flex-1 min-h-0 overflow-hidden">
        <div className="flex items-center gap-2 px-3 py-1.5 shrink-0" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
          <span style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-muted)', letterSpacing: '0.04em', textTransform: 'uppercase' }}>
            {sectionTitle}
          </span>
          <button
            type="button"
            data-testid={`${testIdPrefix}-rail-configure`}
            onClick={onConfigure}
            style={{ marginLeft: 'auto', background: 'transparent', border: 'none', color: 'var(--accent-fg)', fontSize: 11, cursor: 'pointer' }}
          >
            Configure
          </button>
        </div>
        <div className="flex-1 overflow-y-auto">{children}</div>
      </section>

      <section data-testid={`${testIdPrefix}-rail-summary`} className="px-3 py-2 shrink-0 flex flex-col gap-1" style={{ borderTop: '1px solid var(--border-hairline)', fontSize: 11 }}>
        {summary.map((row) => (
          <div key={row.label} className="flex items-center">
            <span style={{ color: 'var(--text-muted)' }}>{row.label}</span>
            <span className="mono-text" style={{ marginLeft: 'auto', color: row.tone === 'fail' ? 'var(--verdict-fail)' : 'var(--text-primary)' }}>
              {row.value}
            </span>
          </div>
        ))}
        {quotaShort && (
          <div data-testid={`${testIdPrefix}-rail-quota-warning`} style={{ color: 'var(--severity-warning)', fontSize: 10, lineHeight: 1.4 }}>
            {quotaShort.need} {plannedNoun} exceed the {quotaShort.free} free session slots — the
            batch may hit the quota; reap stale sessions first.
          </div>
        )}
      </section>

      <section className="px-3 py-2 shrink-0" style={{ borderTop: '1px solid var(--border-hairline)' }}>
        <button
          type="button"
          data-testid={`${testIdPrefix}-rail-run`}
          disabled={!runEnabled}
          onClick={onRun}
          style={{
            width: '100%',
            height: 30,
            background: runEnabled ? 'var(--accent)' : 'var(--surface-raised)',
            color: runEnabled ? 'var(--text-inverse)' : 'var(--text-muted)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: runEnabled ? 'pointer' : 'not-allowed',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          {isRunning ? <Ninebar compact size={12} color="var(--text-muted)" label="running" /> : <span className="material-symbols-outlined" style={{ fontSize: 15 }}>play_arrow</span>}
          {isRunning ? 'Running…' : runLabel}
        </button>
      </section>
    </div>
  );
}

/** Dense clickable rail list row: mono name + right-aligned mono detail. */
export function RailListRow({
  testId,
  name,
  detail,
  onClick,
}: {
  testId: string;
  name: string;
  detail: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      className="flex items-center gap-2 px-3 w-full"
      style={{
        height: 'var(--row-dense)',
        fontSize: 11,
        background: 'transparent',
        border: 'none',
        cursor: 'pointer',
        color: 'var(--text-primary)',
        textAlign: 'left',
      }}
    >
      <span className="truncate mono-text">{name}</span>
      <span className="mono-text" style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--text-muted)' }}>{detail}</span>
    </button>
  );
}

/** Muted hint for an empty rail list. */
export function RailEmptyHint({ children }: { children: ReactNode }) {
  return (
    <div className="px-3 py-4" style={{ fontSize: 11, color: 'var(--text-muted)' }}>
      {children}
    </div>
  );
}

// ── Hero notices ────────────────────────────────────────────────────

export function HeroNotice({
  testId,
  icon,
  title,
  detail,
  tone,
  action,
}: {
  testId: string;
  icon: string;
  title: string;
  detail: string;
  tone?: 'error';
  action?: { label: string; testId: string; onClick: () => void };
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center h-full w-full gap-2 p-6"
      style={{ color: tone === 'error' ? 'var(--severity-error)' : 'var(--text-muted)' }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 32, opacity: 0.85 }}>{icon}</span>
      <span style={{ fontSize: 13, fontWeight: 600 }}>{title}</span>
      <span style={{ fontSize: 11, maxWidth: 400, textAlign: 'center', lineHeight: 1.5 }}>{detail}</span>
      {action && (
        <button
          type="button"
          data-testid={action.testId}
          onClick={action.onClick}
          style={{
            marginTop: 6,
            padding: '5px 14px',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: 'pointer',
            background: 'var(--accent)',
            color: 'var(--text-inverse)',
            border: 'none',
          }}
        >
          {action.label}
        </button>
      )}
    </div>
  );
}
