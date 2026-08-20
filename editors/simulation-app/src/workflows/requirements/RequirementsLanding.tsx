/**
 * RequirementsLanding — the guided activity landing (demo 1d, R15):
 * "N requirements, M unverified → start here". Shown on first entry;
 * every card drops the user into the table with the relevant view.
 *
 * Card grammar (demo ruling): solid border = data card, dashed border =
 * create/authoring affordance. v1 is read-only, so the dashed card
 * teaches the source-authoring path instead of opening an editor.
 *
 * The demo's "recently changed" list is deliberately absent: rows carry
 * no change timestamps until B3 baseline diffing is wired (v1.5) —
 * rendering a fake recency list would be dishonest.
 */

import type { CSSProperties, ReactNode } from 'react';
import type { CoverageStats, RequirementsViewId } from '@/features/requirements/rollup';

const CARD: CSSProperties = {
  border: '1px solid var(--border-default)',
  borderRadius: 'var(--radius-md)',
  padding: '14px 16px',
  background: 'var(--surface-panel)',
  cursor: 'pointer',
  textAlign: 'left',
};

function ActivityCard({
  numeral,
  label,
  action,
  onClick,
  testid,
  dashed = false,
}: {
  numeral: ReactNode;
  label: string;
  action: string;
  onClick?: () => void;
  testid: string;
  dashed?: boolean;
}) {
  return (
    <button
      type="button"
      data-testid={testid}
      onClick={onClick}
      style={{
        ...CARD,
        ...(dashed
          ? { border: '1px dashed var(--border-default)', background: 'transparent' }
          : {}),
        ...(onClick ? {} : { cursor: 'default' }),
      }}
    >
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-2xl)',
          fontVariantNumeric: 'tabular-nums',
          color: 'var(--text-primary)',
        }}
      >
        {numeral}
      </div>
      <div style={{ fontSize: 'var(--text-base)', color: 'var(--text-primary)', marginTop: 2 }}>
        {label}
      </div>
      <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-muted)', marginTop: 8 }}>
        {action}
      </div>
    </button>
  );
}

export interface RequirementsLandingProps {
  stats: CoverageStats;
  workspaceLabel: string | null;
  packageCount: number;
  onEnter: (view: RequirementsViewId, mode: 'grid' | 'document') => void;
}

export function RequirementsLanding({
  stats,
  workspaceLabel,
  packageCount,
  onEnter,
}: RequirementsLandingProps) {
  return (
    <div
      data-testid="requirements-landing"
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        minWidth: 0,
        overflowY: 'auto',
      }}
    >
      <div style={{ width: 720, maxWidth: '90%' }}>
        <div style={{ fontSize: 'var(--text-xl)', fontWeight: 500, color: 'var(--text-primary)' }}>
          This model declares {stats.total} requirement{stats.total === 1 ? '' : 's'}
        </div>
        <div
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-xs)',
            color: 'var(--text-muted)',
            marginTop: 6,
          }}
        >
          {workspaceLabel ?? 'workspace'} · {packageCount} package
          {packageCount === 1 ? '' : 's'}
        </div>
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gap: 12,
            marginTop: 24,
          }}
        >
          <ActivityCard
            testid="requirements-landing-unverified"
            numeral={stats.unverified}
            label={`unverified requirement${stats.unverified === 1 ? '' : 's'}`}
            action="→ review coverage"
            onClick={() => onEnter('unverified', 'grid')}
          />
          <ActivityCard
            testid="requirements-landing-failing"
            numeral={stats.failed}
            label={`with a recorded fail`}
            action="→ triage failures"
            onClick={() => onEnter('failing', 'grid')}
          />
          <ActivityCard
            testid="requirements-landing-maturity"
            numeral={stats.maturityOpen}
            label="maturity still open"
            action="→ work the grid"
            onClick={() => onEnter('all', 'grid')}
          />
          <ActivityCard
            testid="requirements-landing-document"
            numeral="§"
            label="Read the set as a document"
            action="→ open document mode"
            onClick={() => onEnter('all', 'document')}
          />
        </div>
      </div>
    </div>
  );
}

/**
 * Teaching empty state (R15): the model declares no requirements —
 * point at authoring with an example snippet (same posture as
 * create-view). v1 is read-only, so authoring happens in the source.
 */
export function RequirementsEmptyState() {
  return (
    <div
      data-testid="requirements-empty"
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        minWidth: 0,
      }}
    >
      <div style={{ width: 560, maxWidth: '90%' }}>
        <div style={{ fontSize: 'var(--text-xl)', fontWeight: 500, color: 'var(--text-primary)' }}>
          This model declares no requirements yet
        </div>
        <div
          style={{
            fontSize: 'var(--text-base)',
            color: 'var(--text-secondary)',
            marginTop: 8,
            lineHeight: 1.5,
          }}
        >
          Requirements are authored in the source — declare one in any loaded
          <span style={{ fontFamily: 'var(--font-mono)' }}> .sysml </span>
          file and it appears here with its verification rollup.
        </div>
        <pre
          style={{
            border: '1px dashed var(--border-default)',
            borderRadius: 'var(--radius-md)',
            padding: '14px 16px',
            marginTop: 16,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-sm)',
            color: 'var(--text-secondary)',
            lineHeight: 1.6,
            overflowX: 'auto',
          }}
        >
          {`requirement def <'REQ-001'> TripTime {
    doc /* The breaker shall trip within 40 ms. */
    subject breaker : Breaker;
}`}
        </pre>
      </div>
    </div>
  );
}
