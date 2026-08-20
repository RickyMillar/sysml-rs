/**
 * KpisTab — user-defined key performance indicators.
 *
 * Replaces the old auto-only KPI card. Users can define metrics from live
 * variables, choose aggregators, set optional thresholds, and export KPI
 * results. Auto-detected metrics are retained as suggestions.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CardShell } from '@/components/cards/CardShell';
import type { ExportAction } from '@/components/cards/CardShell';
import { exportCSV } from '@/shared/export';
import { computeStableSortedKeys } from '@/shared/data/useStableSortedKeys';
import type { TimePoint } from '@/features/sessions/types';
import { useSessionStore } from '@/features/sessions/store';

export type KpiAggregator = 'final' | 'min' | 'max' | 'mean' | 'rms';
type KpiComparator = '<' | '<=' | '>' | '>=' | '=';

export interface KpiDefinition {
  id: string;
  name: string;
  variable: string;
  aggregator: KpiAggregator;
  comparator?: KpiComparator;
  threshold?: number;
  unit?: string;
}

export interface KpiResult {
  definition: KpiDefinition;
  value: number | null;
  verdict: 'pass' | 'fail' | 'unknown';
}

interface KpisTabProps {
  timeSeries: Record<string, TimePoint[]>;
  clockTime: number;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

function makeKpiId(): string {
  return `kpi-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

function storageKey(sessionId: string | null): string | null {
  return sessionId ? `sysml.results.kpis.${sessionId}` : null;
}

function normalizeDefinition(raw: Partial<KpiDefinition>): KpiDefinition | null {
  if (!raw || typeof raw.id !== 'string' || typeof raw.name !== 'string' || typeof raw.variable !== 'string') return null;
  const aggregator: KpiAggregator = ['final', 'min', 'max', 'mean', 'rms'].includes(raw.aggregator as string)
    ? raw.aggregator as KpiAggregator
    : 'final';
  const comparator = ['<', '<=', '>', '>=', '='].includes(raw.comparator as string)
    ? raw.comparator as KpiComparator
    : undefined;
  const threshold = typeof raw.threshold === 'number' && Number.isFinite(raw.threshold) ? raw.threshold : undefined;
  return {
    id: raw.id,
    name: raw.name,
    variable: raw.variable,
    aggregator,
    comparator,
    threshold,
    unit: raw.unit,
  };
}

export function readStoredDefinitions(sessionId: string | null): KpiDefinition[] | null {
  const key = storageKey(sessionId);
  if (!key || typeof localStorage === 'undefined') return null;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<KpiDefinition>[];
    if (!Array.isArray(parsed)) return null;
    const defs = parsed.map(normalizeDefinition).filter((d): d is KpiDefinition => !!d);
    return defs.length > 0 ? defs : null;
  } catch {
    return null;
  }
}

function writeStoredDefinitions(sessionId: string | null, defs: KpiDefinition[]) {
  const key = storageKey(sessionId);
  if (!key || typeof localStorage === 'undefined') return;
  localStorage.setItem(key, JSON.stringify(defs));
}

export function KpisTab({ timeSeries, clockTime, expanded, onHeaderClick }: KpisTabProps) {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  // UX closeout #4 / #17: stable reference when the variable SET hasn't
  // changed — see PlotsTab's identical pattern / computeStableSortedKeys
  // doc comment for why this matters on a big, fast-ticking model.
  const stableKeysRef = useRef<string[] | null>(null);
  const variableNames = useMemo(() => {
    stableKeysRef.current = computeStableSortedKeys(timeSeries, stableKeysRef.current);
    return stableKeysRef.current;
  }, [timeSeries]);
  const suggestions = useMemo(() => suggestKpis(timeSeries, clockTime), [timeSeries, clockTime]);
  const [definitions, setDefinitions] = useState<KpiDefinition[]>(() =>
    readStoredDefinitions(activeSessionId) ?? [],
  );

  useEffect(() => {
    setDefinitions(readStoredDefinitions(activeSessionId) ?? []);
  }, [activeSessionId]);

  useEffect(() => {
    writeStoredDefinitions(activeSessionId, definitions);
  }, [activeSessionId, definitions]);

  const results = useMemo(
    () => definitions.map((definition) => evaluateKpi(definition, timeSeries)),
    [definitions, timeSeries],
  );

  const addDefinition = useCallback(() => {
    const variable = variableNames[0] ?? '';
    setDefinitions((prev) => [
      ...prev,
      {
        id: makeKpiId(),
        name: variable ? `Final ${variable}` : 'New KPI',
        variable,
        aggregator: 'final',
      },
    ]);
  }, [variableNames]);

  const addSuggestion = useCallback((definition: Omit<KpiDefinition, 'id'>) => {
    setDefinitions((prev) => [
      ...prev,
      { ...definition, id: makeKpiId() },
    ]);
  }, []);

  const updateDefinition = useCallback((id: string, patch: Partial<KpiDefinition>) => {
    setDefinitions((prev) => prev.map((definition) => definition.id === id ? { ...definition, ...patch } : definition));
  }, []);

  const removeDefinition = useCallback((id: string) => {
    setDefinitions((prev) => prev.filter((definition) => definition.id !== id));
  }, []);

  const handleExportCSV = useCallback(() => {
    const headers = ['name', 'variable', 'aggregator', 'value', 'comparator', 'threshold', 'verdict', 'unit'];
    const rows = results.map((r) => [
      r.definition.name,
      r.definition.variable,
      r.definition.aggregator,
      r.value ?? '',
      r.definition.comparator ?? '',
      r.definition.threshold ?? '',
      r.verdict,
      r.definition.unit ?? '',
    ]);
    exportCSV(headers, rows, 'kpis.csv');
  }, [results]);

  const exportActions: ExportAction[] = results.length > 0
    ? [{ label: 'Export CSV', icon: 'csv', onClick: handleExportCSV }]
    : [];

  return (
    <CardShell title="KPIs" icon="speed" accentColor="var(--text-secondary)" expanded={expanded} onHeaderClick={onHeaderClick} exportActions={exportActions}>
      <div data-testid="kpis-tab" className="flex flex-col gap-3">
        <div className="flex items-center justify-between gap-2">
          <div style={{ fontSize: 11, color: 'var(--text-secondary)' }}>
            Define engineering metrics from live variables and optional pass/fail thresholds.
          </div>
          <button type="button" data-testid="kpis-add" onClick={addDefinition} style={primaryButtonStyle}>
            <span className="material-symbols-outlined" style={{ fontSize: 13 }}>add</span>
            Add KPI
          </button>
        </div>

        {definitions.length > 0 ? (
          <div className="flex flex-col gap-2">
            {results.map((result) => (
              <KpiRow
                key={result.definition.id}
                result={result}
                variableNames={variableNames}
                onUpdate={(patch) => updateDefinition(result.definition.id, patch)}
                onRemove={() => removeDefinition(result.definition.id)}
              />
            ))}
          </div>
        ) : (
          <div data-testid="kpis-empty" style={emptyStyle}>
            <span className="material-symbols-outlined" style={{ fontSize: 26 }}>speed</span>
            <div style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-secondary)' }}>No user-defined KPIs yet</div>
            <div style={{ fontSize: 10 }}>Add a KPI or promote one of the suggestions below.</div>
          </div>
        )}

        {suggestions.length > 0 && (
          <section className="flex flex-col gap-1.5" data-testid="kpis-suggestions">
            <div style={{ fontSize: 10, color: 'var(--text-muted)', textTransform: 'uppercase', letterSpacing: '0.06em' }}>
              Suggestions
            </div>
            <div className="flex flex-wrap gap-1.5">
              {suggestions.map((suggestion) => (
                <button
                  key={`${suggestion.variable}-${suggestion.aggregator}-${suggestion.name}`}
                  type="button"
                  onClick={() => addSuggestion(suggestion)}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded mono-text"
                  style={suggestionStyle}
                >
                  <span className="material-symbols-outlined" style={{ fontSize: 12 }}>{iconForVariable(suggestion.variable)}</span>
                  {suggestion.name}
                </button>
              ))}
            </div>
          </section>
        )}
      </div>
    </CardShell>
  );
}

function KpiRow({
  result,
  variableNames,
  onUpdate,
  onRemove,
}: {
  result: KpiResult;
  variableNames: string[];
  onUpdate: (patch: Partial<KpiDefinition>) => void;
  onRemove: () => void;
}) {
  const definition = result.definition;
  return (
    <section data-testid="kpi-row" className="rounded-lg overflow-hidden" style={{ border: '1px solid var(--border-default)', background: 'var(--surface-sunken)' }}>
      <div className="grid gap-2 p-2" style={{ gridTemplateColumns: '1.3fr 1.4fr 0.9fr 0.7fr 0.9fr 0.8fr auto', alignItems: 'center' }}>
        <input
          value={definition.name}
          onChange={(e) => onUpdate({ name: e.target.value })}
          aria-label="KPI name"
          style={inputStyle}
        />
        <select
          value={definition.variable}
          onChange={(e) => onUpdate({ variable: e.target.value })}
          aria-label="KPI variable"
          style={inputStyle}
        >
          {variableNames.length === 0 && <option value="">No variables yet</option>}
          {variableNames.map((name) => <option key={name} value={name}>{name}</option>)}
        </select>
        <select
          value={definition.aggregator}
          onChange={(e) => onUpdate({ aggregator: e.target.value as KpiAggregator })}
          aria-label="KPI aggregator"
          style={inputStyle}
        >
          <option value="final">final</option>
          <option value="min">min</option>
          <option value="max">max</option>
          <option value="mean">mean</option>
          <option value="rms">rms</option>
        </select>
        <select
          value={definition.comparator ?? ''}
          onChange={(e) => onUpdate({ comparator: e.target.value ? e.target.value as KpiComparator : undefined })}
          aria-label="KPI comparator"
          style={inputStyle}
        >
          <option value="">—</option>
          <option value="<">&lt;</option>
          <option value="<=">≤</option>
          <option value=">">&gt;</option>
          <option value=">=">≥</option>
          <option value="=">=</option>
        </select>
        <input
          value={definition.threshold ?? ''}
          onChange={(e) => onUpdate({ threshold: e.target.value === '' ? undefined : Number(e.target.value) })}
          aria-label="KPI threshold"
          placeholder="threshold"
          type="number"
          style={inputStyle}
        />
        <KpiValue result={result} />
        <button type="button" onClick={onRemove} title="Remove KPI" style={iconButtonStyle}>
          <span className="material-symbols-outlined" style={{ fontSize: 14 }}>close</span>
        </button>
      </div>
    </section>
  );
}

function KpiValue({ result }: { result: KpiResult }) {
  const color = result.verdict === 'pass'
    ? 'var(--verdict-pass)'
    : result.verdict === 'fail'
      ? 'var(--verdict-fail)'
      : 'var(--verdict-inconclusive)';
  return (
    <div className="mono-text" style={{ color, fontSize: 12, fontWeight: 700 }}>
      {result.value === null ? '—' : formatNumber(result.value)}
      {result.definition.unit && <span style={{ fontSize: 9, marginLeft: 2 }}>{result.definition.unit}</span>}
      <span style={{ display: 'block', fontSize: 9, fontWeight: 500 }}>{result.verdict}</span>
    </div>
  );
}

export function evaluateKpi(definition: KpiDefinition, timeSeries: Record<string, TimePoint[]>): KpiResult {
  const points = timeSeries[definition.variable] ?? [];
  const value = aggregate(points, definition.aggregator);
  return {
    definition,
    value,
    verdict: verdictFor(value, definition),
  };
}

function aggregate(points: TimePoint[], aggregator: KpiAggregator): number | null {
  if (points.length === 0) return null;
  const values = points.map((p) => p.v).filter(Number.isFinite);
  if (values.length === 0) return null;
  switch (aggregator) {
    case 'final': return values[values.length - 1] ?? null;
    case 'min': return Math.min(...values);
    case 'max': return Math.max(...values);
    case 'mean': return values.reduce((sum, v) => sum + v, 0) / values.length;
    case 'rms': return Math.sqrt(values.reduce((sum, v) => sum + v * v, 0) / values.length);
    default: return null;
  }
}

function verdictFor(value: number | null, definition: KpiDefinition): KpiResult['verdict'] {
  if (value === null || definition.threshold === undefined || !definition.comparator) return 'unknown';
  switch (definition.comparator) {
    case '<': return value < definition.threshold ? 'pass' : 'fail';
    case '<=': return value <= definition.threshold ? 'pass' : 'fail';
    case '>': return value > definition.threshold ? 'pass' : 'fail';
    case '>=': return value >= definition.threshold ? 'pass' : 'fail';
    case '=': return Math.abs(value - definition.threshold) < 1e-9 ? 'pass' : 'fail';
    default: return 'unknown';
  }
}

function suggestKpis(timeSeries: Record<string, TimePoint[]>, clockTime: number): Array<Omit<KpiDefinition, 'id'>> {
  const suggestions: Array<Omit<KpiDefinition, 'id'>> = [];
  for (const [name, points] of Object.entries(timeSeries)) {
    if (points.length === 0) continue;
    const lower = name.toLowerCase();
    if (lower.includes('current') || lower.startsWith('i_') || lower === 'totalcurrent') {
      suggestions.push({ name: `Peak ${name}`, variable: name, aggregator: 'max', unit: 'A' });
    }
    if (lower.startsWith('t_') || lower.includes('temp')) {
      suggestions.push({ name: `Max ${name}`, variable: name, aggregator: 'max', unit: 'K' });
    }
  }
  if (clockTime > 0 && Object.keys(timeSeries).length > 0) {
    const first = Object.keys(timeSeries)[0]!;
    suggestions.push({ name: 'Final sample variable', variable: first, aggregator: 'final' });
  }
  return suggestions.slice(0, 8);
}

function iconForVariable(variable: string): string {
  const lower = variable.toLowerCase();
  if (lower.includes('current') || lower.includes('voltage') || lower.includes('power')) return 'bolt';
  if (lower.includes('temp') || lower.startsWith('t_')) return 'thermostat';
  return 'speed';
}

function formatNumber(value: number): string {
  if (Math.abs(value) >= 1000 || Math.abs(value) < 0.001 && value !== 0) return value.toExponential(3);
  return Number(value.toPrecision(5)).toString();
}

const primaryButtonStyle = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  background: 'color-mix(in srgb, var(--accent) 20%, transparent)',
  color: 'var(--accent-fg)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '4px 8px',
  fontSize: 10,
  fontWeight: 700,
  cursor: 'pointer',
} as const;

const inputStyle = {
  minWidth: 0,
  background: 'var(--surface-panel)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '4px 6px',
  fontSize: 10,
} as const;

const iconButtonStyle = {
  background: 'transparent',
  color: 'var(--text-muted)',
  border: 'none',
  cursor: 'pointer',
  padding: 4,
} as const;

const suggestionStyle = {
  background: 'var(--surface-panel)',
  color: 'var(--text-secondary)',
  border: '1px solid var(--border-default)',
  fontSize: 10,
  cursor: 'pointer',
} as const;

const emptyStyle = {
  minHeight: 100,
  border: '1px dashed var(--border-default)',
  borderRadius: 6,
  color: 'var(--text-muted)',
  textAlign: 'center' as const,
  padding: 16,
  display: 'flex',
  flexDirection: 'column' as const,
  alignItems: 'center',
  justifyContent: 'center',
  gap: 6,
};
