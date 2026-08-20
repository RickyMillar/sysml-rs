/**
 * PlotsTab — configurable plotting workspace.
 *
 * Supports time-series, XY/phase, scatter, and lightweight 3D trajectory
 * plots. This keeps physics examples such as bouncing-ball trajectories
 * from being forced into time-on-x charts.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CardShell } from '@/components/cards/CardShell';
import type { ExportAction } from '@/components/cards/CardShell';
import { PlotVariablePicker } from '@/features/results/plots/PlotVariablePicker';
import { openOverridePopover } from '@/features/sessions/OverridePopover';
import { useSessionStore } from '@/features/sessions/store';
import { classifyVariableDomain, usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { exportCSV, exportPNG } from '@/shared/export';
import { metricRegistry, syncVariableMetrics } from '@/shared/metrics/registry';
import { computeStableSortedKeys } from '@/shared/data/useStableSortedKeys';
import type { MetricDescriptor } from '@/shared/metrics/types';
import { timeSeriesViewer } from '@/shared/viewers/TimeSeriesViewer';
import type { TimeSeriesData } from '@/shared/viewers/types';
import type { TimePoint } from '@/features/sessions/types';

interface PlotsTabProps {
  timeSeries: Record<string, TimePoint[]>;
  /**
   * F2b: lazy full-fidelity accessor for CSV export. `timeSeries` above
   * may be a source-decimated sample (chart hot path); export must never
   * silently truncate data, so it re-reads every stored point on click.
   */
  getFullTimeSeries: () => Record<string, TimePoint[]>;
  running: boolean;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

type PlotKind = 'time-series' | 'xy' | 'scatter' | 'trajectory-3d';

interface PlotDefinition {
  id: string;
  title: string;
  kind: PlotKind;
  variables: string[];
  hidden: string[];
  x?: string;
  y?: string;
  z?: string;
  colorBy?: string | 'time';
}

interface JoinedSample {
  t: number;
  x: number;
  y: number;
  z?: number;
  color?: number;
}

// Exported (ninebar Phase 3) so `WaveformCard` — the bottom-strip shell
// that shares these chart internals rather than forking them — can tint
// its legend/series to match PlotsTab's domain colouring exactly.
export const DOMAIN_COLORS: Record<string, string> = {
  electrical: '#2A5C8F', // domain-electrical — keep in sync with tokens.css --domain-electrical
  thermal: '#8E3A6B', // domain-thermal — keep in sync with tokens.css --domain-thermal
  protection: '#74438A', // domain-protection — keep in sync with tokens.css --domain-protection
  signal: '#1D6E62', // domain-signal — keep in sync with tokens.css --domain-signal
  // domain-mechanical-translational — keep in sync with tokens.css --domain-mechanical-translational
  // (this map doesn't distinguish translational vs rotational; translational picked as the default)
  mechanical: '#4A5F72',
};

export function guessColor(name: string): string | undefined {
  const lower = name.toLowerCase();
  if (lower.includes('current') || lower.includes('voltage') || lower.includes('power')) return DOMAIN_COLORS.electrical;
  if (lower.startsWith('t_') || lower.includes('temp') || lower.includes('thermal')) return DOMAIN_COLORS.thermal;
  if (lower.includes('trip') || lower.includes('protect')) return DOMAIN_COLORS.protection;
  return undefined;
}

function makePlotId(): string {
  return `plot-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

function defaultPlot(selected: string[]): PlotDefinition {
  return {
    id: makePlotId(),
    title: 'Plot 1',
    kind: 'time-series',
    variables: selected,
    hidden: [],
    x: selected[0],
    y: selected[1] ?? selected[0],
    z: selected[2],
    colorBy: 'time',
  };
}

function normalizePlot(raw: Partial<PlotDefinition>): PlotDefinition | null {
  if (!raw || typeof raw.id !== 'string' || typeof raw.title !== 'string') return null;
  const variables = Array.isArray(raw.variables) ? raw.variables.filter((v): v is string => typeof v === 'string') : [];
  const hidden = Array.isArray(raw.hidden) ? raw.hidden.filter((v): v is string => typeof v === 'string') : [];
  const kind: PlotKind =
    raw.kind === 'xy' || raw.kind === 'scatter' || raw.kind === 'trajectory-3d'
      ? raw.kind
      : 'time-series';
  return {
    id: raw.id,
    title: raw.title,
    kind,
    variables,
    hidden,
    x: raw.x,
    y: raw.y,
    z: raw.z,
    colorBy: raw.colorBy,
  };
}

function storageKey(sessionId: string | null): string | null {
  return sessionId ? `sysml.results.plots.${sessionId}` : null;
}

function readStoredPlots(sessionId: string | null): PlotDefinition[] | null {
  const key = storageKey(sessionId);
  if (!key || typeof localStorage === 'undefined') return null;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PlotDefinition>[];
    if (!Array.isArray(parsed)) return null;
    const plots = parsed.map(normalizePlot).filter((p): p is PlotDefinition => !!p);
    return plots.length > 0 ? plots : null;
  } catch {
    return null;
  }
}

function writeStoredPlots(sessionId: string | null, plots: PlotDefinition[]) {
  const key = storageKey(sessionId);
  if (!key || typeof localStorage === 'undefined') return;
  localStorage.setItem(key, JSON.stringify(plots));
}

export function PlotsTab({ timeSeries, getFullTimeSeries, running, expanded, onHeaderClick }: PlotsTabProps) {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const selectionsBySession = usePlotSelectionStore((s) => s.selectionsBySession);
  const setSelected = usePlotSelectionStore((s) => s.setSelected);
  const [pickerForPlot, setPickerForPlot] = useState<string | null>(null);
  const [registryVersion, setRegistryVersion] = useState(0);
  const chartRefs = useRef<Record<string, HTMLDivElement | null>>({});

  // UX closeout #4 / #17: `timeSeries` gets a fresh object identity every
  // tick, but the variable SET is stable for a running session's
  // lifetime — only values change. `computeStableSortedKeys` returns the
  // same array reference when the key set hasn't changed, so the
  // registry-sync effect below only re-fires when a variable is
  // actually added or removed, not on every tick's value update.
  const stableKeysRef = useRef<string[] | null>(null);
  const variableNames = useMemo(() => {
    stableKeysRef.current = computeStableSortedKeys(timeSeries, stableKeysRef.current);
    return stableKeysRef.current;
  }, [timeSeries]);

  useEffect(() => {
    syncVariableMetrics(metricRegistry, variableNames, classifyVariableDomain);
    setRegistryVersion((v) => v + 1);
  }, [variableNames]);

  const availableMetrics = useMemo<MetricDescriptor[]>(
    () => metricRegistry.filter((m) => m.source === 'variable'),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [registryVersion],
  );
  const allVars = useMemo(() => availableMetrics.map((m) => m.id), [availableMetrics]);
  const promotedSelection = useMemo(
    () => (activeSessionId ? selectionsBySession[activeSessionId] ?? [] : []),
    [activeSessionId, selectionsBySession],
  );

  const [plots, setPlots] = useState<PlotDefinition[]>(() =>
    readStoredPlots(activeSessionId) ?? [defaultPlot(promotedSelection)],
  );

  useEffect(() => {
    setPlots(readStoredPlots(activeSessionId) ?? [defaultPlot(promotedSelection)]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId]);

  useEffect(() => {
    if (promotedSelection.length === 0) return;
    setPlots((prev) => {
      const base = prev.length > 0 ? prev : [defaultPlot([])];
      const first = base[0]!;
      const merged = [...first.variables];
      for (const variable of promotedSelection) {
        if (!merged.includes(variable)) merged.push(variable);
      }
      if (merged.length === first.variables.length) return prev;
      return [{ ...first, variables: merged, x: first.x ?? merged[0], y: first.y ?? merged[1] ?? merged[0] }, ...base.slice(1)];
    });
  }, [promotedSelection]);

  useEffect(() => {
    writeStoredPlots(activeSessionId, plots);
  }, [activeSessionId, plots]);

  const updatePlot = useCallback((plotId: string, patch: Partial<PlotDefinition>) => {
    setPlots((prev) => prev.map((plot) => (plot.id === plotId ? { ...plot, ...patch } : plot)));
  }, []);

  const addPlot = useCallback(() => {
    setPlots((prev) => [
      ...prev,
      {
        id: makePlotId(),
        title: `Plot ${prev.length + 1}`,
        kind: 'time-series',
        variables: [],
        hidden: [],
        colorBy: 'time',
      },
    ]);
  }, []);

  const removePlot = useCallback((plotId: string) => {
    setPlots((prev) => (prev.length <= 1 ? prev : prev.filter((plot) => plot.id !== plotId)));
  }, []);

  const pickerPlot = plots.find((plot) => plot.id === pickerForPlot) ?? null;
  const hasAnyData = plots.some((plot) => plot.variables.some((v) => (timeSeries[v] ?? []).length > 0));
  const exportActions: ExportAction[] = hasAnyData
    ? [{ label: 'Export plots CSV', icon: 'csv', onClick: () => exportAllPlotsCsv(plots, getFullTimeSeries()) }]
    : [];

  return (
    <CardShell title="Plots" icon="show_chart" accentColor="var(--text-secondary)" expanded={expanded} onHeaderClick={onHeaderClick} exportActions={exportActions}>
      <div className="flex flex-col gap-2" data-testid="plots-tab">
        <div className="flex items-center justify-between gap-2">
          <div style={{ fontSize: 11, color: 'var(--text-secondary)' }}>
            Build time, XY, scatter, and 3D trajectory plots from live session variables.
          </div>
          <button type="button" data-testid="plots-add-plot" onClick={addPlot} className="inline-flex items-center gap-1 px-2 py-1 rounded mono-text" style={primaryButtonStyle}>
            <span className="material-symbols-outlined" style={{ fontSize: 13 }}>add</span>
            Add plot
          </button>
        </div>

        {plots.map((plot, index) => (
          <PlotCard
            key={plot.id}
            plot={plot}
            index={index}
            canRemove={plots.length > 1}
            timeSeries={timeSeries}
            running={running}
            allVars={allVars}
            chartRef={(el) => { chartRefs.current[plot.id] = el; }}
            onRename={(title) => updatePlot(plot.id, { title })}
            onKindChange={(kind) => updatePlot(plot.id, defaultAxesForKind({ ...plot, kind }, allVars))}
            onAxisChange={(axis, value) => updatePlot(plot.id, { [axis]: value || undefined })}
            onPick={() => setPickerForPlot(plot.id)}
            onRemove={() => removePlot(plot.id)}
            onToggleVariable={(variable) => {
              const hidden = plot.hidden.includes(variable)
                ? plot.hidden.filter((v) => v !== variable)
                : [...plot.hidden, variable];
              updatePlot(plot.id, { hidden });
            }}
            onOverride={(name, value) => openOverridePopover(name, value)}
            onExportCsv={() => exportPlotCsv(plot, getFullTimeSeries())}
            onExportPng={() => {
              const node = chartRefs.current[plot.id];
              if (node) exportPNG(node, `${plot.title || 'plot'}.png`);
            }}
          />
        ))}
      </div>

      {pickerPlot && (
        <PlotVariablePicker
          allVariables={allVars}
          selected={pickerPlot.variables}
          onChange={(next) => {
            updatePlot(pickerPlot.id, {
              ...defaultAxesForKind({ ...pickerPlot, variables: next }, next),
              hidden: pickerPlot.hidden.filter((v) => next.includes(v)),
            });
            if (activeSessionId && plots[0]?.id === pickerPlot.id) setSelected(activeSessionId, next);
          }}
          onClose={() => setPickerForPlot(null)}
        />
      )}

      {/* Overrides render through the app-level OverridePopoverHost
          (ninebar Phase 3 — one consolidated override surface). */}
    </CardShell>
  );
}

function defaultAxesForKind(plot: PlotDefinition, fallbackVars: string[]): Partial<PlotDefinition> {
  const vars = plot.variables.length > 0 ? plot.variables : fallbackVars;
  return {
    kind: plot.kind,
    variables: plot.variables,
    x: plot.x && vars.includes(plot.x) ? plot.x : vars[0],
    y: plot.y && vars.includes(plot.y) ? plot.y : vars[1] ?? vars[0],
    z: plot.z && vars.includes(plot.z) ? plot.z : vars[2],
    colorBy: plot.colorBy && (plot.colorBy === 'time' || vars.includes(plot.colorBy)) ? plot.colorBy : 'time',
  };
}

function PlotCard({
  plot,
  index,
  canRemove,
  timeSeries,
  running,
  allVars,
  chartRef,
  onRename,
  onKindChange,
  onAxisChange,
  onPick,
  onRemove,
  onToggleVariable,
  onOverride,
  onExportCsv,
  onExportPng,
}: {
  plot: PlotDefinition;
  index: number;
  canRemove: boolean;
  timeSeries: Record<string, TimePoint[]>;
  running: boolean;
  allVars: string[];
  chartRef: (el: HTMLDivElement | null) => void;
  onRename: (title: string) => void;
  onKindChange: (kind: PlotKind) => void;
  onAxisChange: (axis: 'x' | 'y' | 'z' | 'colorBy', value: string) => void;
  onPick: () => void;
  onRemove: () => void;
  onToggleVariable: (variable: string) => void;
  onOverride: (name: string, value: string) => void;
  onExportCsv: () => void;
  onExportPng: () => void;
}) {
  const visibleVars = plot.variables.filter((v) => !plot.hidden.includes(v));
  const hasAnySelectedData = plot.variables.some((v) => (timeSeries[v] ?? []).length > 0);
  const canRender = plot.kind === 'time-series'
    ? visibleVars.some((v) => (timeSeries[v] ?? []).length > 0)
    : joinedSamples(plot, timeSeries).length > 0;
  const allVisibleFlat = canRender && !running && visibleVars.length > 0 &&
    visibleVars.every((v) => {
      const pts = timeSeries[v] ?? [];
      return pts.length < 2 || isSeriesFlat(pts);
    });

  return (
    <section data-testid={`plot-card-${index}`} className="rounded-lg overflow-hidden" style={{ border: '1px solid var(--border-default)', background: 'var(--surface-sunken)' }}>
      <div className="flex items-center gap-2 px-2 py-1.5" style={{ borderBottom: '1px solid var(--border-default)' }}>
        <input data-testid={`plot-title-${index}`} value={plot.title} onChange={(e) => onRename(e.target.value)} style={titleInputStyle} aria-label="Plot title" />
        <select data-testid={`plot-kind-${index}`} value={plot.kind} onChange={(e) => onKindChange(e.target.value as PlotKind)} style={selectStyle} aria-label="Plot kind">
          <option value="time-series">Time series</option>
          <option value="xy">XY / phase</option>
          <option value="scatter">Scatter</option>
          <option value="trajectory-3d">3D trajectory</option>
        </select>
        <button type="button" data-testid={`plot-pick-${index}`} onClick={onPick} style={smallButtonStyle}>
          <span className="material-symbols-outlined" style={{ fontSize: 13 }}>tune</span>
          Variables ({plot.variables.length})
        </button>
        <button type="button" onClick={onExportCsv} disabled={!hasAnySelectedData} style={smallButtonStyle} title="Export CSV"><span className="material-symbols-outlined" style={{ fontSize: 13 }}>csv</span></button>
        <button type="button" onClick={onExportPng} disabled={!canRender} style={smallButtonStyle} title="Export PNG"><span className="material-symbols-outlined" style={{ fontSize: 13 }}>image</span></button>
        {canRemove && <button type="button" data-testid={`plot-remove-${index}`} onClick={onRemove} style={smallButtonStyle} title="Remove plot"><span className="material-symbols-outlined" style={{ fontSize: 13 }}>close</span></button>}
      </div>

      <div className="p-2 flex flex-col gap-2">
        <AxisControls plot={plot} allVars={allVars} onAxisChange={onAxisChange} />
        {plot.variables.length === 0 ? (
          <PlotEmpty index={index} allVars={allVars} running={running} />
        ) : !canRender ? (
          <div className="flex flex-col gap-2" style={{ color: 'var(--text-muted)', fontSize: 11 }}>
            {running ? 'Waiting for selected variables to produce data…' : missingDataHint(plot)}
            <VariableChips variables={plot.variables} hidden={plot.hidden} timeSeries={timeSeries} onToggle={onToggleVariable} onOverride={onOverride} />
          </div>
        ) : (
          <>
            <VariableChips variables={plot.variables} hidden={plot.hidden} timeSeries={timeSeries} onToggle={onToggleVariable} onOverride={onOverride} />
            <div ref={chartRef}>
              <PlotRenderer plot={plot} visibleVars={visibleVars} timeSeries={timeSeries} />
            </div>
            {allVisibleFlat && (
              <div style={{ fontSize: 10, color: 'var(--text-muted)', background: 'var(--surface-panel)', borderRadius: 4, padding: '4px 8px', display: 'flex', alignItems: 'center', gap: 4 }}>
                <span className="material-symbols-outlined" style={{ fontSize: 13 }}>info</span>
                All selected variables are flat — they may not be observed by the active run target.
              </div>
            )}
          </>
        )}
      </div>
    </section>
  );
}

function AxisControls({
  plot,
  allVars,
  onAxisChange,
}: {
  plot: PlotDefinition;
  allVars: string[];
  onAxisChange: (axis: 'x' | 'y' | 'z' | 'colorBy', value: string) => void;
}) {
  if (plot.kind === 'time-series') return null;
  return (
    <div className="flex flex-wrap items-center gap-2" style={{ fontSize: 10, color: 'var(--text-muted)' }}>
      <AxisSelect label="x" value={plot.x ?? ''} vars={plot.variables.length ? plot.variables : allVars} onChange={(v) => onAxisChange('x', v)} />
      <AxisSelect label="y" value={plot.y ?? ''} vars={plot.variables.length ? plot.variables : allVars} onChange={(v) => onAxisChange('y', v)} />
      {plot.kind === 'trajectory-3d' && <AxisSelect label="z" value={plot.z ?? ''} vars={plot.variables.length ? plot.variables : allVars} onChange={(v) => onAxisChange('z', v)} />}
      {(plot.kind === 'scatter' || plot.kind === 'trajectory-3d') && (
        <label className="inline-flex items-center gap-1">
          color
          <select value={plot.colorBy ?? 'time'} onChange={(e) => onAxisChange('colorBy', e.target.value)} style={selectStyle}>
            <option value="time">time</option>
            {(plot.variables.length ? plot.variables : allVars).map((v) => <option key={v} value={v}>{labelFor(v)}</option>)}
          </select>
        </label>
      )}
    </div>
  );
}

function AxisSelect({ label, value, vars, onChange }: { label: string; value: string; vars: string[]; onChange: (v: string) => void }) {
  return (
    <label className="inline-flex items-center gap-1">
      {label}
      <select value={value} onChange={(e) => onChange(e.target.value)} style={selectStyle}>
        <option value="">—</option>
        {vars.map((v) => <option key={v} value={v}>{labelFor(v)}</option>)}
      </select>
    </label>
  );
}

function PlotRenderer({ plot, visibleVars, timeSeries }: { plot: PlotDefinition; visibleVars: string[]; timeSeries: Record<string, TimePoint[]> }) {
  if (plot.kind === 'time-series') {
    const series = visibleVars.map((name) => ({ name: labelFor(name), points: timeSeries[name] ?? [], color: guessColor(name) }));
    const viewerData: TimeSeriesData = { kind: 'time-series', series };
    return <>{timeSeriesViewer.render(viewerData, { height: Math.max(170, visibleVars.length * 48) })}</>;
  }
  const samples = joinedSamples(plot, timeSeries);
  return <ParametricPlot kind={plot.kind} samples={samples} xLabel={plot.x ?? 'x'} yLabel={plot.y ?? 'y'} zLabel={plot.z} />;
}

function ParametricPlot({ kind, samples, xLabel, yLabel, zLabel }: { kind: PlotKind; samples: JoinedSample[]; xLabel: string; yLabel: string; zLabel?: string }) {
  const width = 720;
  const height = 260;
  const pad = { left: 46, right: 18, top: 18, bottom: 34 };
  const projected = samples.map((s) => kind === 'trajectory-3d' ? project3d(s) : { x: s.x, y: s.y, color: s.color ?? s.t });
  const xs = projected.map((p) => p.x);
  const ys = projected.map((p) => p.y);
  const cs = projected.map((p) => p.color ?? 0);
  const [xMin, xMax] = extent(xs);
  const [yMin, yMax] = extent(ys);
  const [cMin, cMax] = extent(cs);
  const sx = (x: number) => pad.left + ((x - xMin) / (xMax - xMin || 1)) * (width - pad.left - pad.right);
  const sy = (y: number) => height - pad.bottom - ((y - yMin) / (yMax - yMin || 1)) * (height - pad.top - pad.bottom);
  const path = projected.map((p, i) => `${i === 0 ? 'M' : 'L'} ${sx(p.x).toFixed(2)} ${sy(p.y).toFixed(2)}`).join(' ');
  return (
    <svg data-testid={`plot-${kind}-viewer`} viewBox={`0 0 ${width} ${height}`} style={{ width: '100%', minHeight: 220, color: 'var(--text-primary)' }} role="img" aria-label={`${kind} plot`}>
      <rect x={0} y={0} width={width} height={height} fill="var(--surface-panel)" />
      <line x1={pad.left} y1={height - pad.bottom} x2={width - pad.right} y2={height - pad.bottom} stroke="var(--border-default)" />
      <line x1={pad.left} y1={pad.top} x2={pad.left} y2={height - pad.bottom} stroke="var(--border-default)" />
      {kind !== 'scatter' && <path d={path} fill="none" stroke="var(--chart-series-2)" strokeWidth={2} opacity={0.85} />}
      {projected.map((p, i) => (
        <circle key={i} cx={sx(p.x)} cy={sy(p.y)} r={kind === 'scatter' ? 3 : 2.2} fill={colorRamp(p.color ?? i, cMin, cMax)} opacity={0.85} />
      ))}
      <text x={width / 2} y={height - 8} textAnchor="middle" fontSize={11} fill="currentColor" opacity={0.75}>{labelFor(xLabel)}</text>
      <text x={14} y={height / 2} textAnchor="middle" transform={`rotate(-90 14 ${height / 2})`} fontSize={11} fill="currentColor" opacity={0.75}>{labelFor(yLabel)}</text>
      {kind === 'trajectory-3d' && <text x={width - pad.right} y={pad.top + 10} textAnchor="end" fontSize={10} fill="currentColor" opacity={0.65}>projected 3D {zLabel ? `(${labelFor(zLabel)})` : ''}</text>}
    </svg>
  );
}

function project3d(s: JoinedSample): { x: number; y: number; color: number } {
  const z = s.z ?? 0;
  return { x: s.x - z * 0.35, y: s.y - z * 0.25, color: s.color ?? s.t };
}

function joinedSamples(plot: PlotDefinition, timeSeries: Record<string, TimePoint[]>): JoinedSample[] {
  if (!plot.x || !plot.y) return [];
  if (plot.kind === 'trajectory-3d' && !plot.z) return [];
  const xPoints = timeSeries[plot.x] ?? [];
  const yByT = new Map((timeSeries[plot.y] ?? []).map((p) => [p.t, p.v]));
  const zByT = plot.z ? new Map((timeSeries[plot.z] ?? []).map((p) => [p.t, p.v])) : null;
  const colorByT = plot.colorBy && plot.colorBy !== 'time' ? new Map((timeSeries[plot.colorBy] ?? []).map((p) => [p.t, p.v])) : null;
  const samples: JoinedSample[] = [];
  for (const xp of xPoints) {
    const y = yByT.get(xp.t);
    if (y === undefined) continue;
    const z = zByT?.get(xp.t);
    if (zByT && z === undefined) continue;
    samples.push({ t: xp.t, x: xp.v, y, z, color: colorByT?.get(xp.t) ?? xp.t });
  }
  return samples;
}

function VariableChips({ variables, hidden, timeSeries, onToggle, onOverride }: { variables: string[]; hidden: string[]; timeSeries: Record<string, TimePoint[]>; onToggle: (variable: string) => void; onOverride: (name: string, value: string) => void }) {
  return (
    <div className="flex flex-wrap gap-1">
      {variables.map((variable) => {
        const pts = timeSeries[variable] ?? [];
        const lastValue = pts.length > 0 ? pts[pts.length - 1]!.v : 0;
        const isHidden = hidden.includes(variable);
        // `domainColor` is a literal hex when guessColor() finds a domain match — the
        // `${accent}22` alpha-suffix trick below only works on a literal hex string, so
        // the neutral fallback (no domain match) must use color-mix instead of a var().
        const domainColor = guessColor(variable);
        const accent = domainColor ?? 'var(--text-secondary)';
        return (
          <span key={variable} className="inline-flex items-center rounded-full mono-text" style={{ fontSize: 9, background: isHidden ? 'transparent' : domainColor ? `${domainColor}22` : 'color-mix(in srgb, var(--text-secondary) 13%, transparent)', color: isHidden ? 'var(--text-muted)' : accent, opacity: isHidden ? 0.5 : 1, paddingLeft: 6 }}>
            <button type="button" onClick={() => onToggle(variable)} title={isHidden ? `Show ${variable}` : `Hide ${variable}`} style={{ background: 'none', border: 'none', color: 'inherit', cursor: 'pointer', padding: '2px 0', font: 'inherit' }}>{variable}</button>
            <button type="button" data-testid={`plot-edit-${variable}`} onClick={() => onOverride(variable, String(lastValue))} title={`Edit override for ${variable}`} style={{ background: 'none', border: 'none', color: 'inherit', cursor: 'pointer', padding: '0 4px', display: 'inline-flex', alignItems: 'center' }}>
              <span className="material-symbols-outlined" style={{ fontSize: 11 }}>tune</span>
            </button>
          </span>
        );
      })}
    </div>
  );
}

function PlotEmpty({ index, allVars, running }: { index: number; allVars: string[]; running: boolean }) {
  return (
    <div data-testid={`plot-empty-${index}`} className="flex flex-col items-center justify-center gap-2" style={{ minHeight: 130, border: '1px dashed var(--border-default)', borderRadius: 6, color: 'var(--text-muted)', textAlign: 'center', padding: 16 }}>
      <span className="material-symbols-outlined" style={{ fontSize: 24 }}>add_chart</span>
      <div style={{ fontSize: 12, color: 'var(--text-secondary)', fontWeight: 600 }}>Choose variables for this plot</div>
      <div style={{ fontSize: 10 }}>{allVars.length > 0 ? `${allVars.length} variables are available.` : running ? 'Waiting for the first simulation snapshot…' : 'Run or step the model to populate variables.'}</div>
    </div>
  );
}

function missingDataHint(plot: PlotDefinition): string {
  if (plot.kind !== 'time-series' && (!plot.x || !plot.y || (plot.kind === 'trajectory-3d' && !plot.z))) {
    return 'Choose x/y axes (and z for 3D) before this plot can render.';
  }
  return 'Variables selected. Step the simulation to see traces.';
}

function exportPlotCsv(plot: PlotDefinition, timeSeries: Record<string, TimePoint[]>) {
  if (plot.kind === 'time-series') {
    const visible = plot.variables.filter((v) => !plot.hidden.includes(v));
    const times = collectTimes(visible, timeSeries);
    const headers = ['time', ...visible.map(labelFor)];
    const rows = times.map((t) => [t, ...visible.map((name) => timeSeries[name]?.find((p) => p.t === t)?.v ?? NaN)]);
    exportCSV(headers, rows, `${plot.title || 'plot'}.csv`);
    return;
  }
  const samples = joinedSamples(plot, timeSeries);
  const headers = ['time', labelFor(plot.x ?? 'x'), labelFor(plot.y ?? 'y'), ...(plot.z ? [labelFor(plot.z)] : [])];
  const rows = samples.map((s) => [s.t, s.x, s.y, ...(plot.z ? [s.z ?? NaN] : [])]);
  exportCSV(headers, rows, `${plot.title || 'plot'}.csv`);
}

function exportAllPlotsCsv(plots: PlotDefinition[], timeSeries: Record<string, TimePoint[]>) {
  const variables = Array.from(new Set(plots.flatMap((plot) => plot.variables.filter((v) => !plot.hidden.includes(v)))));
  const times = collectTimes(variables, timeSeries);
  const headers = ['time', ...variables.map(labelFor)];
  const rows = times.map((t) => [t, ...variables.map((name) => timeSeries[name]?.find((p) => p.t === t)?.v ?? NaN)]);
  exportCSV(headers, rows, 'plots.csv');
}

function collectTimes(variables: string[], timeSeries: Record<string, TimePoint[]>): number[] {
  const timeSet = new Set<number>();
  for (const variable of variables) for (const point of timeSeries[variable] ?? []) timeSet.add(point.t);
  return [...timeSet].sort((a, b) => a - b);
}

function labelFor(id: string): string {
  return metricRegistry.get(id)?.name ?? id;
}

function isSeriesFlat(pts: TimePoint[]): boolean {
  if (pts.length < 2) return true;
  const first = pts[0]!.v;
  return pts.every((p) => Math.abs(p.v - first) < 1e-10);
}

function extent(values: number[]): [number, number] {
  if (values.length === 0) return [0, 1];
  let min = Infinity;
  let max = -Infinity;
  for (const v of values) {
    if (Number.isFinite(v)) {
      min = Math.min(min, v);
      max = Math.max(max, v);
    }
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0, 1];
  if (min === max) return [min - 0.5, max + 0.5];
  return [min, max];
}

function colorRamp(value: number, min: number, max: number): string {
  const t = Math.max(0, Math.min(1, (value - min) / (max - min || 1)));
  const r = Math.round(96 + t * 160);
  const g = Math.round(165 - t * 80);
  const b = Math.round(250 - t * 120);
  return `rgb(${r}, ${g}, ${b})`;
}

const primaryButtonStyle = {
  background: 'color-mix(in srgb, var(--accent) 20%, transparent)',
  color: 'var(--accent-fg)',
  border: '1px solid var(--border-default)',
  fontSize: 10,
  fontWeight: 700,
  cursor: 'pointer',
} as const;

const smallButtonStyle = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  background: 'var(--surface-panel)',
  color: 'var(--text-secondary)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '3px 6px',
  fontSize: 10,
  cursor: 'pointer',
} as const;

const titleInputStyle = {
  flex: 1,
  minWidth: 0,
  background: 'transparent',
  border: 'none',
  color: 'var(--text-primary)',
  fontSize: 12,
  fontWeight: 700,
  outline: 'none',
} as const;

const selectStyle = {
  background: 'var(--surface-panel)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '2px 5px',
  fontSize: 10,
} as const;
