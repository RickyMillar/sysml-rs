/**
 * Pure presentation logic for the Requirements workbench — grouping,
 * coverage stats, view-preset filters, and the chip models. Kept free
 * of React so the vitest node environment can exercise it directly.
 */

import type {
  RequirementRow,
  RequirementVerificationRollup,
} from './types';

// ── Package grouping ─────────────────────────────────────────────────

export interface RequirementGroup {
  /** Package ElementId, or null for rows with no Package ancestor. */
  packageId: string | null;
  /** Display label ("(no package)" fallback). */
  label: string;
  rows: RequirementRow[];
}

/**
 * Group document-ordered rows into contiguous runs by owning package.
 * Contiguous runs (not a global bucket-by-id) so document order — the
 * outline contract — is preserved exactly; a package that reappears
 * later in the document legitimately appears as a second group.
 */
export function groupRowsByPackage(rows: RequirementRow[]): RequirementGroup[] {
  const groups: RequirementGroup[] = [];
  for (const row of rows) {
    const pkgId = row.owning_package?.id ?? null;
    const last = groups[groups.length - 1];
    if (last && last.packageId === pkgId) {
      last.rows.push(row);
    } else {
      groups.push({
        packageId: pkgId,
        label: row.owning_package?.name ?? '(no package)',
        rows: [row],
      });
    }
  }
  return groups;
}

/** Unique packages in first-appearance (document) order, with row counts. */
export interface PackageSummary {
  packageId: string | null;
  label: string;
  count: number;
}

export function summarizePackages(rows: RequirementRow[]): PackageSummary[] {
  const byId = new Map<string | null, PackageSummary>();
  for (const row of rows) {
    const pkgId = row.owning_package?.id ?? null;
    const existing = byId.get(pkgId);
    if (existing) {
      existing.count += 1;
    } else {
      byId.set(pkgId, {
        packageId: pkgId,
        label: row.owning_package?.name ?? '(no package)',
        count: 1,
      });
    }
  }
  return [...byId.values()];
}

// ── Coverage stats (strip + landing) ─────────────────────────────────

export interface CoverageStats {
  total: number;
  passed: number;
  failed: number;
  incomplete: number;
  /** total − passed: everything not fully verified. */
  unverified: number;
  /** passed / total in [0,1]; 0 when the set is empty. */
  coverage: number;
  /** Rows whose maturity is still open/tbd/tbr/tbc (or absent). */
  maturityOpen: number;
}

export function coverageStats(rows: RequirementRow[]): CoverageStats {
  let passed = 0;
  let failed = 0;
  let incomplete = 0;
  let maturityOpen = 0;
  for (const row of rows) {
    if (row.verification.state === 'pass') passed += 1;
    else if (row.verification.state === 'fail') failed += 1;
    else incomplete += 1;
    if (!row.maturity || !['done', 'closed'].includes(row.maturity)) {
      maturityOpen += 1;
    }
  }
  const total = rows.length;
  return {
    total,
    passed,
    failed,
    incomplete,
    unverified: total - passed,
    coverage: total === 0 ? 0 : passed / total,
    maturityOpen,
  };
}

// ── View presets (left-rail "saved views", v1 fixed set) ─────────────

export type RequirementsViewId = 'all' | 'unverified' | 'failing' | 'orphans' | 'suspect';

export interface RequirementsViewPreset {
  id: RequirementsViewId;
  label: string;
  matches: (row: RequirementRow) => boolean;
}

/**
 * Fixed presets; user-defined saved views (columns + filters + sort) are
 * the R2 follow-up, "My subsystem" waits on the sidecar. The `suspect`
 * preset (v1.5) matches against the baseline-diff record set passed to
 * `filterRows` — its static `matches` is never used (the workbench hides
 * the preset entirely when no baseline is selected).
 */
export const VIEW_PRESETS: RequirementsViewPreset[] = [
  { id: 'all', label: 'All', matches: () => true },
  {
    id: 'unverified',
    label: 'Unverified',
    matches: (row) => row.verification.state !== 'pass',
  },
  {
    id: 'failing',
    label: 'Failing',
    matches: (row) => row.verification.state === 'fail',
  },
  {
    // R7's orphan lens: requirements nothing claims to satisfy.
    id: 'orphans',
    label: 'No satisfiers',
    matches: (row) => row.satisfied_by.length === 0,
  },
  {
    // R9: rows changed (or changed-upstream) since the selected
    // baseline. Data-driven — see filterRows' `suspectIds` param.
    id: 'suspect',
    label: 'Suspect',
    matches: () => false,
  },
];

export function filterRows(
  rows: RequirementRow[],
  view: RequirementsViewId,
  packageId: string | null | undefined,
  /** Requirement ids suspect vs the selected baseline (v1.5). Only
   *  consulted by the `suspect` view; undefined = no baseline data,
   *  which makes that view honestly empty rather than stale. */
  suspectIds?: ReadonlySet<string>,
): RequirementRow[] {
  const preset = VIEW_PRESETS.find((v) => v.id === view) ?? VIEW_PRESETS[0];
  const matches = (row: RequirementRow) =>
    view === 'suspect' ? (suspectIds?.has(row.id) ?? false) : preset.matches(row);
  return rows.filter(
    (row) =>
      matches(row) &&
      (packageId === undefined || (row.owning_package?.id ?? null) === packageId),
  );
}

// ── Verified rollup chip (the three-state ruling, design doc §5) ─────

export type VerifiedChipVariant = 'pass' | 'fail' | 'outline' | 'none';

export interface VerifiedChipModel {
  variant: VerifiedChipVariant;
  label: string;
  /** In-UI labeling is REQUIRED by the §5 ruling — the demo left the
   *  three states unexplained; the tooltip is the legend. */
  title: string;
}

export function verifiedChipModel(
  rollup: RequirementVerificationRollup,
): VerifiedChipModel {
  const counts = `${rollup.cases_passed}/${rollup.cases_total}`;
  // Evaluation-mode label is BINDING (§2.1a ruling (d)) but is now RAISED
  // out of this tooltip suffix into the visible `EvaluationModeBadge` that
  // `VerifiedChip` renders beside the chip — so it is no longer appended
  // here (a static verdict on an ODE-backed case still answers a different
  // question; the badge is where that now reads at a glance, not on hover).
  if (rollup.state === 'fail') {
    return {
      variant: 'fail',
      label: counts,
      title: `${counts} verification cases passed — at least one recorded fail`,
    };
  }
  if (rollup.state === 'pass') {
    return {
      variant: 'pass',
      label: `${counts} ✓`,
      title: `All ${rollup.cases_total} linked verification cases passed`,
    };
  }
  if (rollup.cases_total === 0) {
    return {
      variant: 'none',
      label: '—',
      title: 'No verification cases linked — unverified',
    };
  }
  return {
    variant: 'outline',
    label: counts,
    title: `${counts} verification cases passed — incomplete (no recorded fail, but not every case has run to Pass)`,
  };
}

// ── Maturity chip (glyph-differentiated neutral — never colour-coded) ─

/** StatusKind { open · tbd · tbr · tbc · done · closed } → fill glyph. */
export function maturityGlyph(maturity: string): string {
  switch (maturity) {
    case 'done':
    case 'closed':
      return '●';
    case 'tbr':
    case 'tbc':
      return '◐';
    default:
      return '○'; // open, tbd, unknown literals
  }
}

/** Display label for a row: requirement ID when declared, else name. */
export function rowDisplayId(row: RequirementRow): string {
  return row.req_id ?? row.name ?? row.qualified_name ?? row.id;
}
