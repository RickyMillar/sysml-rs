/**
 * RequirementsRail — the left-rail content (portaled via
 * `<LeftRailContent>`): package list + view presets.
 *
 * Two stacked sections per the demo: "packages" (counts per package,
 * filters the table) and "views" (the v1 fixed preset list — see
 * `VIEW_PRESETS`; user-defined saved views are the R2 follow-up).
 * Current item = accent ink-bar + tint, identical treatment in both
 * lists (visual gate ruling: one rendering of "current" everywhere).
 */

import type { CSSProperties } from 'react';
import type { PackageSummary, RequirementsViewId } from '@/features/requirements/rollup';
import { VIEW_PRESETS, filterRows } from '@/features/requirements/rollup';
import type { RequirementRow } from '@/features/requirements/types';

const SECTION_HEADER: CSSProperties = {
  height: 'var(--row-default)',
  flex: 'none',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '0 12px',
  borderBottom: '1px solid var(--border-default)',
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
};

function railRow(current: boolean): CSSProperties {
  return {
    display: 'flex',
    alignItems: 'center',
    gap: 6,
    height: 'var(--row-compact)',
    padding: '0 12px',
    fontFamily: 'var(--font-mono)',
    fontSize: 11.5,
    color: current ? 'var(--text-primary)' : 'var(--text-secondary)',
    cursor: 'pointer',
    ...(current
      ? { background: 'var(--accent-tint)', boxShadow: 'inset 2px 0 0 var(--accent)' }
      : {}),
  };
}

export interface RequirementsRailProps {
  rows: RequirementRow[];
  packages: PackageSummary[];
  activeView: RequirementsViewId;
  packageFilter: string | null | undefined;
  onViewChange: (view: RequirementsViewId) => void;
  onPackageFilterChange: (packageId: string | null | undefined) => void;
  /** Suspect requirement ids vs the selected baseline; null = no
   *  baseline selected → the Suspect view row is hidden (v1.5). */
  suspectIds?: ReadonlySet<string> | null;
  /** Baseline the suspect set was computed against (labels the row). */
  baselineName?: string | null;
}

export function RequirementsRail({
  rows,
  packages,
  activeView,
  packageFilter,
  onViewChange,
  onPackageFilterChange,
  suspectIds = null,
  baselineName = null,
}: RequirementsRailProps) {
  return (
    <div
      data-testid="requirements-rail"
      style={{ display: 'flex', flexDirection: 'column', overflowY: 'auto', minHeight: 0 }}
    >
      <div style={SECTION_HEADER}>
        packages
        <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-muted)' }}>
          {rows.length} reqs
        </span>
      </div>
      <div style={{ padding: '4px 0' }}>
        <div
          data-testid="requirements-rail-package-all"
          onClick={() => onPackageFilterChange(undefined)}
          style={railRow(packageFilter === undefined)}
        >
          all packages
          <span style={{ flex: 1 }} />
          <span style={{ color: 'var(--text-muted)' }}>{rows.length}</span>
        </div>
        {packages.map((pkg) => {
          const current = packageFilter === pkg.packageId;
          return (
            <div
              key={pkg.packageId ?? 'none'}
              data-testid={`requirements-rail-package-${pkg.label}`}
              onClick={() => onPackageFilterChange(current ? undefined : pkg.packageId)}
              style={{ ...railRow(current), paddingLeft: 26 }}
            >
              {pkg.label}
              <span style={{ flex: 1 }} />
              <span style={{ color: 'var(--text-muted)' }}>{pkg.count}</span>
            </div>
          );
        })}
      </div>
      <div style={{ ...SECTION_HEADER, borderTop: '1px solid var(--border-default)' }}>
        views
      </div>
      <div style={{ padding: '4px 0' }}>
        {VIEW_PRESETS.map((preset) => {
          // The Suspect view only exists relative to a selected baseline.
          if (preset.id === 'suspect' && (suspectIds === null || baselineName === null)) {
            return null;
          }
          const count = filterRows(rows, preset.id, undefined, suspectIds ?? undefined).length;
          const label =
            preset.id === 'suspect' ? `Suspect since ${baselineName}` : preset.label;
          return (
            <div
              key={preset.id}
              data-testid={`requirements-rail-view-${preset.id}`}
              onClick={() => onViewChange(preset.id)}
              style={railRow(activeView === preset.id)}
            >
              {label}
              <span style={{ flex: 1 }} />
              <span style={{ color: 'var(--text-muted)' }}>{count}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
