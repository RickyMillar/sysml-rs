/**
 * CompareWorkflow — R4.2 multi-session comparison shell.
 *
 * Route: `/compare`. Replaces the R4.1 single-pair stub that wrapped
 * `CompareWorkspace`. The new workflow is built around three axes:
 *
 *   1. N-session picker (2..6 sessions) on the left sidebar.
 *   2. Shared playhead at the top — one tick cursor drives every chart,
 *      diagram, and value display.
 *   3. Mode plug-in slot on the right rail — R4.3 (Agent X) registers
 *      ensemble / golden / two-design modes here via `CompareMode`.
 *
 * The main area shows per-variable heat-band gutters (`DivergenceGutter`)
 * so the user can jump the playhead to "interesting" ticks with a click.
 * When Agent X's modes override `mainRender`, this shell yields the main
 * area to them; otherwise the default variable-first view ships today.
 *
 * Data plumbing deliberately stops at the variable level: each picked
 * session's snapshot history is read from its own archived record (via
 * `loadArchivedSession`). Live sessions are fine too — their snapshot
 * history is pulled the same way when present. Missing data produces
 * `NaN`s in the divergence math, which the selectors already handle.
 */

import { useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import {
  listArchivedSessions,
  loadArchivedSession,
  type ArchivedSession,
  type ArchivedSessionSummary,
} from '@/shared/data/SessionArchive';
import { CompareSessionPicker } from './CompareSessionPicker';
import { CompareVariablePicker } from './CompareVariablePicker';
import { DivergenceGutter } from './DivergenceGutter';
import { SharedPlayhead } from './SharedPlayhead';
import {
  autoPickVariables,
  playheadMaxTick,
  type SamplesBySession,
} from './selectors';
import {
  CompareModeProvider,
  PLACEHOLDER_MODE,
  type CompareContext,
  type CompareMode,
} from './compareMode';
import { resolveLayout, useCompareStore } from './useCompareStore';
import { isFlagEnabled } from '@/featureFlags';
import { CompareWorkflowNinebar } from './ninebar/CompareWorkflowNinebar';

// ── Registry of compare modes ───────────────────────────────────────
//
// Agent X (R4.3) pushes modes into this registry at module init via
// `registerCompareMode`. We keep it module-local (not React state) so
// registrations can fire from any file without a provider tree.

const REGISTERED_MODES: CompareMode[] = [];

/**
 * Register a compare mode. Safe to call during module import; duplicate
 * ids replace the prior entry so hot-reload doesn't leak stale plug-ins.
 */
export function registerCompareMode(mode: CompareMode): void {
  const idx = REGISTERED_MODES.findIndex((m) => m.id === mode.id);
  if (idx >= 0) REGISTERED_MODES[idx] = mode;
  else REGISTERED_MODES.push(mode);
}

/** Test/clean helper — drops all registered modes. */
export function __resetCompareModesForTesting(): void {
  REGISTERED_MODES.length = 0;
}

function listModes(): CompareMode[] {
  return REGISTERED_MODES.length > 0 ? REGISTERED_MODES : [PLACEHOLDER_MODE];
}

// ── Snapshot → samples extraction ────────────────────────────────────

/** Per-session numeric snapshot history keyed by tick. */
interface SessionSamples {
  id: string;
  label: string;
  tickCount: number;
  /** variable name → value[tick]. NaN when the session lacked a reading. */
  variables: Record<string, number[]>;
}

/**
 * Turn an archived session's stored snapshot history into a per-variable
 * numeric array. Each snapshot is expected to carry `variables: {name:
 * number}` (matching the `extractIngestPoint` shape). Non-numeric values
 * are dropped. Missing ticks become `NaN`.
 */
function archiveToSessionSamples(record: ArchivedSession): SessionSamples {
  const snapshots = record.snapshotHistory ?? [];
  const tickCount = snapshots.length;
  const variables: Record<string, number[]> = {};

  const ensure = (name: string): number[] => {
    let arr = variables[name];
    if (!arr) {
      arr = new Array<number>(tickCount).fill(NaN);
      variables[name] = arr;
    }
    return arr;
  };

  for (let t = 0; t < tickCount; t++) {
    const snap = snapshots[t] ?? {};
    const rawVars = (snap['variables'] ?? null) as Record<string, unknown> | null;
    if (!rawVars || typeof rawVars !== 'object') continue;
    for (const [k, v] of Object.entries(rawVars)) {
      if (k.startsWith('__') || k === 't_ms' || k === 'tick') continue;
      let numeric: number | null = null;
      if (typeof v === 'number' && Number.isFinite(v)) numeric = v;
      else if (typeof v === 'boolean') numeric = v ? 1 : 0;
      else if (v && typeof v === 'object' && !Array.isArray(v)) {
        const obj = v as Record<string, unknown>;
        if (typeof obj.value === 'number' && Number.isFinite(obj.value)) numeric = obj.value;
        else if (typeof obj.re === 'number' && Number.isFinite(obj.re)) numeric = obj.re;
      }
      if (numeric !== null) ensure(k)[t] = numeric;
    }
  }

  // If no snapshot had any variables, still publish tickCount so the
  // playhead can work (divergence will be 0 across the board).
  if (tickCount > 0 && Object.keys(variables).length === 0) {
    // Empty record — leave the variables map empty.
  }

  return {
    id: record.id,
    label: record.label ?? record.id.slice(0, 8),
    tickCount,
    variables,
  };
}

// ── Workflow root ───────────────────────────────────────────────────

/**
 * Route entry for /run/compare. Under the (default-on) `ninebar` flag
 * the surface is the Phase 6 diff canvas (`CompareWorkflowNinebar`);
 * flag-off keeps the legacy R4.2 body verbatim (deleted in Phase 8
 * per F17, same lifecycle as the Analyze/Verify legacy bodies).
 */
export function CompareWorkflow() {
  if (isFlagEnabled('ninebar')) return <CompareWorkflowNinebar />;
  return <CompareWorkflowLegacy />;
}

function CompareWorkflowLegacy() {
  const pickedIds = useCompareStore((s) => s.pickedSessionIds);
  const sharedTick = useCompareStore((s) => s.sharedTick);
  const setSharedTick = useCompareStore((s) => s.setSharedTick);
  const userLayout = useCompareStore((s) => s.layout);
  const setLayout = useCompareStore((s) => s.setLayout);
  const activeModeId = useCompareStore((s) => s.activeModeId);
  const setActiveModeId = useCompareStore((s) => s.setActiveModeId);
  const pickedVars = useCompareStore((s) => s.pickedVariables);

  const layout = resolveLayout(userLayout, pickedIds.length);

  // ── Archive list (lightweight summaries) ───────────────────────────
  const [archiveList, setArchiveList] = useState<ArchivedSessionSummary[]>([]);
  useEffect(() => {
    let cancelled = false;
    listArchivedSessions()
      .then((l) => {
        if (!cancelled) setArchiveList(l);
      })
      .catch(() => {
        if (!cancelled) setArchiveList([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // ── Load full records for picked sessions ──────────────────────────
  const [sessionSamples, setSessionSamples] = useState<SessionSamples[]>([]);
  useEffect(() => {
    if (pickedIds.length === 0) {
      setSessionSamples([]);
      return;
    }
    let cancelled = false;
    Promise.all(pickedIds.map((id) => loadArchivedSession(id)))
      .then((records) => {
        if (cancelled) return;
        const loaded = records
          .filter((r): r is ArchivedSession => r != null)
          .map(archiveToSessionSamples);
        setSessionSamples(loaded);
      })
      .catch(() => {
        if (!cancelled) setSessionSamples([]);
      });
    return () => {
      cancelled = true;
    };
  }, [pickedIds]);

  // ── Derived: max tick + per-variable samples ───────────────────────
  const tickCounts = useMemo(
    () => sessionSamples.map((s) => s.tickCount),
    [sessionSamples],
  );
  const maxTick = useMemo(() => playheadMaxTick(tickCounts), [tickCounts]);

  const availableVariables = useMemo(() => {
    const set = new Set<string>();
    for (const s of sessionSamples) {
      for (const v of Object.keys(s.variables)) set.add(v);
    }
    return Array.from(set).sort();
  }, [sessionSamples]);

  const variableSamples = useMemo(() => {
    const out: Record<string, SamplesBySession> = {};
    for (const name of availableVariables) {
      const perSession: SamplesBySession = sessionSamples.map(
        (s) => s.variables[name] ?? new Array<number>(s.tickCount).fill(NaN),
      );
      out[name] = perSession;
    }
    return out;
  }, [sessionSamples, availableVariables]);

  const autoPicks = useMemo(
    () => autoPickVariables(variableSamples, 6),
    [variableSamples],
  );

  const focusedVariables = useMemo(() => {
    if (pickedVars === null) return autoPicks;
    return pickedVars.filter((v) => v in variableSamples);
  }, [pickedVars, autoPicks, variableSamples]);

  // ── Mode context ──────────────────────────────────────────────────
  const compareCtx: CompareContext = useMemo(
    () => ({
      sharedTick,
      setSharedTick,
      pickedSessionIds: pickedIds,
      layout,
    }),
    [sharedTick, setSharedTick, pickedIds, layout],
  );

  const modes = listModes();
  const activeMode =
    modes.find((m) => m.id === activeModeId) ?? modes[0] ?? PLACEHOLDER_MODE;

  // Keep `activeModeId` in sync when modes register for the first time.
  useEffect(() => {
    if (activeModeId == null && modes.length > 0) {
      setActiveModeId(modes[0].id);
    }
  }, [activeModeId, modes, setActiveModeId]);

  const sessionTicks = useMemo(
    () =>
      sessionSamples.map((s) => ({
        id: s.id,
        label: s.label,
        ticks: s.tickCount,
      })),
    [sessionSamples],
  );

  // ── Render ────────────────────────────────────────────────────────
  return (
    <CompareModeProvider value={compareCtx}>
      <div
        data-testid="compare-workflow"
        className="flex h-full w-full overflow-hidden"
      >
        {/* LEFT — session picker */}
        <CompareSessionPicker archive={archiveList} />

        {/* CENTER — shared playhead + variable picker + main */}
        <div className="flex flex-col flex-1 overflow-hidden">
          <div className="flex items-center justify-between">
            <div style={{ flex: 1 }}>
              <SharedPlayhead maxTick={maxTick} sessionTicks={sessionTicks} />
            </div>
            <LayoutSwitcher
              layout={layout}
              pickCount={pickedIds.length}
              onChange={(l) => setLayout(l)}
              userOverride={userLayout}
            />
          </div>

          <CompareVariablePicker
            availableVariables={availableVariables}
            autoPicked={autoPicks}
          />

          <div
            className="flex-1 overflow-auto"
            data-testid="compare-main"
            data-layout={layout}
          >
            {pickedIds.length < 2 && (
              <NeedMoreSessions pickedCount={pickedIds.length} />
            )}
            {pickedIds.length >= 2 && activeMode.mainRender && (
              <div data-testid="compare-main-custom">
                {activeMode.mainRender(compareCtx)}
              </div>
            )}
            {pickedIds.length >= 2 && !activeMode.mainRender && (
              <DefaultMain
                variables={focusedVariables}
                samples={variableSamples}
                sharedTick={sharedTick}
                onScrubTo={setSharedTick}
                sessionLabels={sessionSamples.map((s) => s.label)}
                layout={layout}
              />
            )}
          </div>
        </div>

        {/* RIGHT — mode config slot */}
        <aside
          data-testid="compare-mode-config"
          className="flex flex-col overflow-hidden"
          style={{
            width: 260,
            borderLeft: '1px solid var(--outline-variant)',
            background: 'var(--surface-container-low)',
          }}
        >
          <ModeSwitcher
            modes={modes}
            activeId={activeMode.id}
            onPick={setActiveModeId}
          />
          <div className="flex-1 overflow-y-auto" data-testid="compare-mode-config-slot">
            {activeMode.configRender(compareCtx)}
          </div>
        </aside>
      </div>
    </CompareModeProvider>
  );
}

// ── Helper components ───────────────────────────────────────────────

function NeedMoreSessions({ pickedCount }: { pickedCount: number }) {
  const needed = 2 - pickedCount;
  return (
    <div
      data-testid="compare-need-more"
      className="flex flex-col items-center justify-center h-full gap-3"
      style={{ color: 'var(--outline)' }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 44, opacity: 0.4 }}>
        compare
      </span>
      <div style={{ fontSize: 14, fontWeight: 500 }}>
        Pick {needed} more session{needed === 1 ? '' : 's'}
      </div>
      <div style={{ fontSize: 12, maxWidth: 320, textAlign: 'center', lineHeight: 1.5 }}>
        Select between 2 and 6 sessions from the left sidebar. The shared
        playhead spans the longest session; shorter sessions freeze on
        their last tick.
      </div>
    </div>
  );
}

function LayoutSwitcher({
  layout,
  pickCount,
  userOverride,
  onChange,
}: {
  layout: 'overlay' | 'side-by-side';
  pickCount: number;
  userOverride: 'overlay' | 'side-by-side' | null;
  onChange: (layout: 'overlay' | 'side-by-side' | null) => void;
}) {
  const chip = (value: 'overlay' | 'side-by-side', label: string, icon: string) => (
    <button
      key={value}
      type="button"
      data-testid={`compare-layout-${value}`}
      data-active={layout === value ? 'true' : 'false'}
      onClick={() => onChange(value)}
      style={{
        fontSize: 10,
        padding: '3px 8px',
        border: `1px solid ${layout === value ? 'var(--primary)' : 'var(--outline-variant)'}`,
        background:
          layout === value
            ? 'var(--primary-container, #004b7a33)'
            : 'var(--surface-container)',
        color: 'var(--on-surface)',
        cursor: 'pointer',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 3,
        borderRadius: 3,
      }}
      title={`Layout: ${label}`}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
        {icon}
      </span>
      {label}
    </button>
  );
  return (
    <div
      className="flex items-center gap-1 px-3 shrink-0"
      style={{ height: 44, borderBottom: '1px solid var(--outline-variant)' }}
    >
      {chip('overlay', 'Overlay', 'layers')}
      {chip('side-by-side', 'Side-by-side', 'view_column')}
      {userOverride !== null && (
        <button
          type="button"
          data-testid="compare-layout-auto"
          onClick={() => onChange(null)}
          title={`Reset to auto (${pickCount <= 3 ? 'overlay' : 'side-by-side'})`}
          style={{
            fontSize: 9,
            padding: '3px 6px',
            border: '1px solid var(--outline-variant)',
            background: 'var(--surface-container)',
            color: 'var(--outline)',
            cursor: 'pointer',
            borderRadius: 3,
          }}
        >
          Auto
        </button>
      )}
    </div>
  );
}

function ModeSwitcher({
  modes,
  activeId,
  onPick,
}: {
  modes: CompareMode[];
  activeId: string;
  onPick: (id: string) => void;
}) {
  return (
    <div
      data-testid="compare-mode-switcher"
      style={{
        padding: 8,
        borderBottom: '1px solid var(--outline-variant)',
        display: 'flex',
        flexWrap: 'wrap',
        gap: 4,
      }}
    >
      {modes.map((m) => (
        <button
          key={m.id}
          type="button"
          data-testid={`compare-mode-${m.id}`}
          data-active={m.id === activeId ? 'true' : 'false'}
          onClick={() => onPick(m.id)}
          title={m.description}
          aria-label={`${m.label} — ${m.description}`}
          style={{
            fontSize: 10,
            padding: '3px 8px',
            border: `1px solid ${m.id === activeId ? 'var(--primary)' : 'var(--outline-variant)'}`,
            background:
              m.id === activeId
                ? 'var(--primary-container, #004b7a33)'
                : 'var(--surface-container)',
            color: 'var(--on-surface)',
            cursor: 'pointer',
            borderRadius: 3,
          }}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}

function DefaultMain({
  variables,
  samples,
  sharedTick,
  onScrubTo,
  sessionLabels,
  layout,
}: {
  variables: string[];
  samples: Record<string, SamplesBySession>;
  sharedTick: number;
  onScrubTo: (tick: number) => void;
  sessionLabels: string[];
  layout: 'overlay' | 'side-by-side';
}) {
  if (variables.length === 0) {
    return (
      <div
        data-testid="compare-main-empty-vars"
        className="flex items-center justify-center h-full"
        style={{ color: 'var(--outline)', fontSize: 12 }}
      >
        No variables available in these sessions.
      </div>
    );
  }

  const gridStyle: CSSProperties =
    layout === 'side-by-side'
      ? {
          display: 'grid',
          gridTemplateColumns: `repeat(${Math.max(1, sessionLabels.length)}, minmax(200px, 1fr))`,
          gap: 12,
          padding: 12,
        }
      : {
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
          gap: 12,
          padding: 12,
        };

  return (
    <div style={gridStyle} data-testid="compare-variable-grid">
      {variables.map((name) => (
        <VariableCard
          key={name}
          name={name}
          samples={samples[name] ?? []}
          sharedTick={sharedTick}
          onScrubTo={onScrubTo}
          sessionLabels={sessionLabels}
          layout={layout}
        />
      ))}
    </div>
  );
}

function VariableCard({
  name,
  samples,
  sharedTick,
  onScrubTo,
  sessionLabels,
  layout,
}: {
  name: string;
  samples: SamplesBySession;
  sharedTick: number;
  onScrubTo: (tick: number) => void;
  sessionLabels: string[];
  layout: 'overlay' | 'side-by-side';
}) {
  const currentValues = samples.map((row, s) => {
    const lastIdx = Math.min(sharedTick, row.length - 1);
    const v = lastIdx >= 0 ? row[lastIdx] : NaN;
    return { label: sessionLabels[s] ?? `s${s}`, value: v };
  });

  return (
    <div
      data-testid={`compare-variable-card-${name}`}
      style={{
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 4,
        padding: 8,
        display: 'flex',
        gap: 8,
      }}
    >
      <DivergenceGutter
        samples={samples}
        onScrubTo={onScrubTo}
        label={name}
        currentTick={sharedTick}
        height={layout === 'side-by-side' ? 100 : 120}
      />
      <div className="flex-1 min-w-0 flex flex-col gap-1">
        <div
          className="mono-text"
          style={{
            fontSize: 12,
            fontWeight: 600,
            color: 'var(--on-surface)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {name}
        </div>
        <div className="flex flex-col gap-0.5">
          {currentValues.map((row, i) => (
            <div
              key={i}
              className="flex items-center justify-between"
              style={{ fontSize: 10 }}
            >
              <span
                style={{
                  color: 'var(--outline)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  maxWidth: 110,
                }}
              >
                {row.label}
              </span>
              <span
                className="mono-text"
                style={{
                  color: Number.isFinite(row.value)
                    ? 'var(--on-surface)'
                    : 'var(--outline)',
                }}
              >
                {Number.isFinite(row.value) ? row.value.toFixed(3) : '—'}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ── Re-exports for Agent X integration ──────────────────────────────
export type { CompareContext, CompareMode } from './compareMode';
export { useCompareStore } from './useCompareStore';
