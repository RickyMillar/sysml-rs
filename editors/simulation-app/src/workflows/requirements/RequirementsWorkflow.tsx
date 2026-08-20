/**
 * RequirementsWorkflow — the `/requirements` route (ninebar Phase 7.5,
 * Requirements workbench v1, read + navigate).
 *
 * "A normal requirements tool that happens to speak SysML v2": the
 * document-shaped table over live `sysml.workspace.requirement_rows`
 * (B2) — one item store rendered as a grid AND an outline document,
 * with a view toggle that preserves selection. Five-slot composition:
 * left rail = package list + view presets (portaled), right rail =
 * links context (registered below), bottom strip = coverage summary.
 *
 * v1 is READ-ONLY: no authoring, no writeback, no sidecar. Baselines /
 * suspect flags are v1.5 (B3 wiring); method columns are B4.
 *
 * Verification truth: the UI reads `row.verification` — the three-state
 * rollup computed backend-side from per-case verdicts. It never joins
 * `sysml.aggregate` (design doc §5 correction: aggregate is two-bucket
 * owner-keyed counts and cannot distinguish fail/incomplete/pass).
 */

import { useEffect, useMemo, useState } from 'react';
import { BottomStripContent, LeftRailContent } from '@/app/slots';
import { useRightRailStore } from '@/app/rail/railStore';
import { Ninebar } from '@/components/Ninebar';
import { useBaselines, useCreateBaseline, useSuspects } from '@/features/baselines/queries';
import { useBaselineStore } from '@/features/baselines/store';
import { useRequirementRows } from '@/features/requirements/queries';
import {
  coverageStats,
  filterRows,
  groupRowsByPackage,
  summarizePackages,
  VIEW_PRESETS,
  type RequirementsViewId,
} from '@/features/requirements/rollup';
import type { RequirementRow } from '@/features/requirements/types';
// Stable public component (the ONLY sanctioned import from the
// traceability feature — internals stay fenced).
import { TraceabilityMatrixPanel } from '@/features/traceability/TraceabilityMatrixPanel';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { BaselinePicker, formatBaselineDate } from './BaselinePicker';
import { RequirementsDocument } from './RequirementsDocument';
import { RequirementsGrid } from './RequirementsGrid';
import { RequirementsEmptyState, RequirementsLanding } from './RequirementsLanding';
import { RequirementsRail } from './RequirementsRail';
import { useRequirementsSelectionStore } from './requirementsSelectionStore';
// Side-effect import: registers the `requirements-links` rail context
// before any `openRail(...)` call can reference it.
import { REQUIREMENTS_LINKS_CONTEXT_ID } from './requirementsLinksRailContext';

type TableMode = 'document' | 'grid' | 'trace';

/** Document ⇄ Grid per the demo toolbar, plus Trace — the coverage
 *  sub-view REUSES the Phase-7 trace-matrix component (plan task; the
 *  Browse workflow's Source/Trace switch is the precedent). */
const MODE_OPTIONS: Array<{ value: TableMode; label: string }> = [
  { value: 'document', label: 'Document' },
  { value: 'grid', label: 'Grid' },
  { value: 'trace', label: 'Trace' },
];

export function RequirementsWorkflow() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  // Verification check occurrences are bookkeeping owned by their case
  // (steward ruling 2026-07-16) — never rows here. Their home is the
  // Verify workbench; the API reveal flag exists for agents/debugging.
  const rowsQuery = useRequirementRows();

  const [mode, setMode] = useState<TableMode>('grid');
  const [activeView, setActiveView] = useState<RequirementsViewId>('all');
  // undefined = no package filter; null = "rows with no package".
  const [packageFilter, setPackageFilter] = useState<string | null | undefined>(undefined);
  const [entered, setEntered] = useState(false);

  const selectedRow = useRequirementsSelectionStore((s) => s.selectedRow);
  const setSelectedRow = useRequirementsSelectionStore((s) => s.setSelectedRow);
  const openRail = useRightRailStore((s) => s.open);

  // Route-scoped selection: leaving the workbench clears it so the rail
  // context never shows a stale row on re-entry.
  useEffect(() => () => setSelectedRow(null), [setSelectedRow]);

  // ── Baselines + suspect (v1.5) ─────────────────────────────────────
  const baselinesQuery = useBaselines();
  const createBaseline = useCreateBaseline();
  const selectedBaseline = useBaselineStore((s) => s.selected);
  const setSelectedBaseline = useBaselineStore((s) => s.setSelected);
  const baselines = useMemo(() => baselinesQuery.data ?? [], [baselinesQuery.data]);
  // Default to the most recently created baseline (list is newest-first);
  // clear a selection whose baseline vanished (backend restart — the
  // store is in-memory per server process).
  useEffect(() => {
    if (selectedBaseline !== null && !baselines.some((b) => b.name === selectedBaseline)) {
      setSelectedBaseline(null);
      return;
    }
    if (selectedBaseline === null && baselines.length > 0) {
      setSelectedBaseline(baselines[0].name);
    }
  }, [baselines, selectedBaseline, setSelectedBaseline]);

  const suspectsQuery = useSuspects(selectedBaseline, rowsQuery.data?.revision ?? null);
  const suspects = suspectsQuery.data ?? null;
  const suspectIds = useMemo(
    () => (suspects ? new Set(suspects.keys()) : null),
    [suspects],
  );
  // A deselected/vanished baseline makes the Suspect view meaningless.
  useEffect(() => {
    if (activeView === 'suspect' && selectedBaseline === null) {
      setActiveView('all');
    }
  }, [activeView, selectedBaseline]);

  const rows = useMemo(() => rowsQuery.data?.rows ?? [], [rowsQuery.data]);
  const stats = useMemo(() => coverageStats(rows), [rows]);
  const packages = useMemo(() => summarizePackages(rows), [rows]);
  const visibleRows = useMemo(
    () => filterRows(rows, activeView, packageFilter, suspectIds ?? undefined),
    [rows, activeView, packageFilter, suspectIds],
  );
  const groups = useMemo(() => groupRowsByPackage(visibleRows), [visibleRows]);
  const suspectCount = useMemo(
    () => (suspectIds ? rows.filter((r) => suspectIds.has(r.id)).length : null),
    [rows, suspectIds],
  );
  const currentBaseline = baselines.find((b) => b.name === selectedBaseline) ?? null;

  const selectRow = (row: RequirementRow) => {
    setSelectedRow(row);
    openRail(REQUIREMENTS_LINKS_CONTEXT_ID);
  };

  const enterTable = (view: RequirementsViewId, nextMode: TableMode) => {
    setActiveView(view);
    setMode(nextMode);
    setEntered(true);
  };

  // ── Pre-table states ───────────────────────────────────────────────
  if (!workspaceRoot) {
    return (
      <CenteredNote testid="requirements-no-workspace">
        Load a workspace to see its requirements.
      </CenteredNote>
    );
  }
  if (rowsQuery.isLoading) {
    return (
      <CenteredNote testid="requirements-loading">
        <Ninebar label="loading requirement rows" size={14} />
        <span style={{ marginLeft: 10 }}>Loading requirements…</span>
      </CenteredNote>
    );
  }
  if (rowsQuery.isError) {
    return (
      <CenteredNote testid="requirements-error">
        <span>Failed to load requirement rows.</span>
        <button
          type="button"
          data-testid="requirements-retry"
          onClick={() => void rowsQuery.refetch()}
          style={{
            marginLeft: 10,
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            background: 'transparent',
            color: 'var(--text-primary)',
            padding: '4px 10px',
            cursor: 'pointer',
            fontSize: 'var(--text-sm)',
          }}
        >
          Retry
        </button>
      </CenteredNote>
    );
  }
  if (rows.length === 0) {
    return (
      <div data-testid="requirements-workflow" style={{ display: 'flex', height: '100%' }}>
        <RequirementsEmptyState />
      </div>
    );
  }
  // Landing (demo 1d): rails collapsed, strip absent — full-primary.
  if (!entered) {
    return (
      <div data-testid="requirements-workflow" style={{ display: 'flex', height: '100%' }}>
        <RequirementsLanding
          stats={stats}
          workspaceLabel={workspaceRoot}
          packageCount={packages.length}
          onEnter={enterTable}
        />
      </div>
    );
  }

  // ── The table (grid | document), rails + strip mounted ─────────────
  return (
    <>
      <LeftRailContent>
        <RequirementsRail
          rows={rows}
          packages={packages}
          activeView={activeView}
          packageFilter={packageFilter}
          onViewChange={setActiveView}
          onPackageFilterChange={setPackageFilter}
          suspectIds={suspectIds}
          baselineName={selectedBaseline}
        />
      </LeftRailContent>

      <BottomStripContent collapsed>
        <CoverageStripRow
          stats={stats}
          truncated={rowsQuery.data?.truncated ?? false}
          suspectCount={suspectCount}
          baselineLabel={
            currentBaseline
              ? `Baseline ${currentBaseline.name} (${formatBaselineDate(currentBaseline.created_at)})`
              : null
          }
        />
      </BottomStripContent>

      <div
        data-testid="requirements-workflow"
        className="flex flex-col h-full w-full overflow-hidden"
      >
        {/* Toolbar (40px): mode toggle + active filter chips */}
        <div
          className="flex items-center gap-2 px-3 shrink-0"
          style={{
            height: 'var(--row-comfortable)',
            borderBottom: '1px solid var(--border-default)',
          }}
        >
          <div
            data-testid="requirements-mode-switch"
            role="radiogroup"
            aria-label="Table mode"
            className="inline-flex"
            style={{
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              overflow: 'hidden',
            }}
          >
            {MODE_OPTIONS.map((opt, idx) => {
              const active = opt.value === mode;
              return (
                <button
                  key={opt.value}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  data-testid={`requirements-mode-${opt.value}`}
                  onClick={() => setMode(opt.value)}
                  style={{
                    border: 'none',
                    borderLeft: idx > 0 ? '1px solid var(--border-default)' : 'none',
                    background: active ? 'var(--surface-raised)' : 'transparent',
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    padding: '4px 12px',
                    fontSize: 'var(--text-sm)',
                    fontWeight: active ? 600 : 500,
                    cursor: 'pointer',
                  }}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
          {activeView !== 'all' && (
            <FilterChip
              testid="requirements-view-chip"
              label={`view: ${VIEW_PRESETS.find((v) => v.id === activeView)?.label ?? activeView}`}
              onClear={() => setActiveView('all')}
            />
          )}
          {packageFilter !== undefined && (
            <FilterChip
              testid="requirements-package-chip"
              label={`package: ${
                packages.find((p) => p.packageId === packageFilter)?.label ?? '(no package)'
              }`}
              onClear={() => setPackageFilter(undefined)}
            />
          )}
          <div style={{ flex: 1 }} />
          <UnsavedEditsIndicator />
          <span
            data-testid="requirements-count"
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--text-xs)',
              color: 'var(--text-muted)',
            }}
          >
            {visibleRows.length} of {rows.length}
          </span>
          {!baselinesQuery.isError && (
            <BaselinePicker
              baselines={baselines}
              selected={selectedBaseline}
              onSelect={setSelectedBaseline}
              onCreate={(name) => createBaseline.mutate(name)}
              creating={createBaseline.isPending}
            />
          )}
        </div>

        <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
          {mode === 'grid' && (
            <RequirementsGrid
              groups={groups}
              selectedId={selectedRow?.id ?? null}
              onSelect={selectRow}
              suspects={suspects}
              baselineName={selectedBaseline}
            />
          )}
          {mode === 'document' && (
            <RequirementsDocument
              groups={groups}
              selectedId={selectedRow?.id ?? null}
              onSelect={selectRow}
            />
          )}
          {mode === 'trace' && <TraceabilityMatrixPanel />}
        </div>
      </div>
    </>
  );
}

// ── Bits ─────────────────────────────────────────────────────────────

function CenteredNote({
  children,
  testid,
}: {
  children: React.ReactNode;
  testid: string;
}) {
  return (
    <div
      data-testid={testid}
      style={{
        height: '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--text-secondary)',
        fontSize: 'var(--text-base)',
      }}
    >
      {children}
    </div>
  );
}

/**
 * The crib-sheet's `N unsaved edit(s)` amber indicator (demo 1b) —
 * editor-owns-save stays VISIBLE (design §1 guard / §7.5): inline edits
 * dirty buffers; nothing here writes disk.
 */
function UnsavedEditsIndicator() {
  const dirtyCount = useWorkspaceStore((s) => {
    let n = 0;
    for (const file of s.loadedFiles.values()) if (file.dirty) n++;
    return n;
  });
  if (dirtyCount === 0) return null;
  return (
    <span
      data-testid="requirements-unsaved-edits"
      title="Inline edits live in the open buffers — the editor owns save"
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--text-xs)',
        color: 'var(--accent-fg)',
      }}
    >
      {dirtyCount} unsaved {dirtyCount === 1 ? 'edit' : 'edits'}
    </span>
  );
}

function FilterChip({
  label,
  onClear,
  testid,
}: {
  label: string;
  onClear: () => void;
  testid: string;
}) {
  return (
    <span
      data-testid={testid}
      style={{
        fontSize: 'var(--text-sm)',
        padding: '3px 10px',
        borderRadius: 999,
        background: 'var(--surface-raised)',
        border: '1px solid var(--border-default)',
        color: 'var(--text-primary)',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
      }}
    >
      {label}
      <button
        type="button"
        aria-label={`clear ${label}`}
        onClick={onClear}
        style={{
          border: 'none',
          background: 'transparent',
          color: 'var(--text-muted)',
          cursor: 'pointer',
          padding: 0,
          fontSize: 'var(--text-sm)',
          lineHeight: 1,
        }}
      >
        ✕
      </button>
    </span>
  );
}

/**
 * Coverage strip (24px): Trace-Score-style honest summary. Coverage =
 * share of requirements whose rollup is `pass` (labeled via title).
 * The bar fill is NEUTRAL — info is neutral, verdict colours stay on
 * chips. Sans base wrapper, mono only on numerals (typography ruling).
 */
function CoverageStripRow({
  stats,
  truncated,
  suspectCount = null,
  baselineLabel = null,
}: {
  stats: ReturnType<typeof coverageStats>;
  truncated: boolean;
  /** Rows suspect vs the selected baseline; null = no baseline. */
  suspectCount?: number | null;
  /** e.g. "Baseline B2 (2026-07-02)"; null = no baseline selected. */
  baselineLabel?: string | null;
}) {
  const pct = Math.round(stats.coverage * 100);
  return (
    <div
      data-testid="requirements-strip"
      style={{
        height: 'var(--statusbar-height)',
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '0 12px',
        fontFamily: 'var(--font-body)',
        fontSize: 'var(--text-xs)',
        color: 'var(--text-muted)',
      }}
    >
      <span
        style={{ display: 'flex', alignItems: 'center', gap: 8 }}
        title="Share of requirements whose verification rollup is pass (every linked case passed)"
      >
        Coverage{' '}
        <span style={{ color: 'var(--text-primary)', fontFamily: 'var(--font-mono)' }}>
          {pct}%
        </span>
        <span
          style={{
            width: 72,
            height: 5,
            background: 'var(--surface-raised)',
            borderRadius: 2,
            overflow: 'hidden',
            display: 'flex',
          }}
        >
          <span style={{ width: `${pct}%`, background: 'var(--text-muted)' }} />
        </span>
      </span>
      <span>
        <span style={{ color: 'var(--text-primary)', fontFamily: 'var(--font-mono)' }}>
          {stats.unverified}
        </span>{' '}
        unverified
      </span>
      <span>
        <span style={{ color: 'var(--text-primary)', fontFamily: 'var(--font-mono)' }}>
          {stats.failed}
        </span>{' '}
        failing
      </span>
      {suspectCount !== null && (
        <span
          data-testid="requirements-strip-suspect"
          title="Rows changed (or changed-upstream) since the selected baseline"
        >
          <span style={{ color: 'var(--severity-warning)', fontFamily: 'var(--font-mono)' }}>
            {suspectCount}
          </span>{' '}
          suspect
        </span>
      )}
      {truncated && (
        <span data-testid="requirements-strip-truncated" style={{ color: 'var(--severity-warning)' }}>
          row set truncated — showing the first pages only
        </span>
      )}
      <div style={{ flex: 1 }} />
      {baselineLabel && (
        <span
          data-testid="requirements-strip-baseline"
          style={{ fontFamily: 'var(--font-mono)' }}
        >
          {baselineLabel}
        </span>
      )}
      <span>
        <span style={{ fontFamily: 'var(--font-mono)' }}>{stats.total}</span> requirements
      </span>
    </div>
  );
}
