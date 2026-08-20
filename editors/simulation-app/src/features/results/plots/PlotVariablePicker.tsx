/**
 * PlotVariablePicker — modal panel for picking variables to chart.
 *
 * Discoverability surface for the Plots tab: the inline chips only
 * toggle visibility for already-charted variables, but new users
 * expected a single, obvious "pick what to chart" affordance.
 *
 * Variables are grouped by heuristic domain, filterable by search, and
 * each domain has Select-All / Clear controls.
 */

import { useMemo, useState } from 'react';
import {
  classifyVariableDomain,
  DOMAIN_LABELS,
  DOMAIN_ORDER,
  type VariableDomain,
} from '@/features/results/usePlotSelectionStore';

interface PlotVariablePickerProps {
  /** Every variable name from the latest snapshot (sorted). */
  allVariables: string[];
  /** Currently-selected variable names. */
  selected: string[];
  /** Replace the entire selection list. */
  onChange: (names: string[]) => void;
  /** Close the picker without further action. */
  onClose: () => void;
}

const DOMAIN_ACCENT: Record<VariableDomain, string> = {
  electrical: '#2A5C8F', // domain-electrical — keep in sync with tokens.css --domain-electrical
  thermal: '#8E3A6B', // domain-thermal — keep in sync with tokens.css --domain-thermal
  // domain-mechanical-translational — keep in sync with tokens.css --domain-mechanical-translational
  // (this map doesn't distinguish translational vs rotational; translational picked as the default)
  mechanical: '#4A5F72',
  protection: '#74438A', // domain-protection — keep in sync with tokens.css --domain-protection
  signal: '#1D6E62', // domain-signal — keep in sync with tokens.css --domain-signal
  other: '#7E6E58', // domain-uncategorized (nb-n-500) — keep in sync with tokens.css --domain-uncategorized
};

export function PlotVariablePicker({
  allVariables,
  selected,
  onChange,
  onClose,
}: PlotVariablePickerProps) {
  const [query, setQuery] = useState('');
  const selectedSet = useMemo(() => new Set(selected), [selected]);

  const grouped = useMemo(() => {
    const lowerQ = query.trim().toLowerCase();
    const groups = new Map<VariableDomain, string[]>();
    for (const domain of DOMAIN_ORDER) groups.set(domain, []);
    for (const name of allVariables) {
      if (lowerQ && !name.toLowerCase().includes(lowerQ)) continue;
      const d = classifyVariableDomain(name);
      groups.get(d)!.push(name);
    }
    return groups;
  }, [allVariables, query]);

  const totalShown = useMemo(
    () => Array.from(grouped.values()).reduce((acc, list) => acc + list.length, 0),
    [grouped],
  );

  const setMany = (toAdd: string[]) => {
    const merged = new Set(selectedSet);
    for (const n of toAdd) merged.add(n);
    onChange(Array.from(merged));
  };

  const removeMany = (toRemove: string[]) => {
    const dropping = new Set(toRemove);
    onChange(selected.filter((n) => !dropping.has(n)));
  };

  const clearAll = () => onChange([]);

  return (
    <div
      data-testid="plot-variable-picker"
      role="dialog"
      aria-label="Select plot variables"
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,0.5)',
        zIndex: 1000,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 480,
          maxHeight: '80vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--surface-raised)',
          border: '1px solid var(--border-default)',
          borderRadius: 8,
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{
            borderBottom: '1px solid var(--border-default)',
            background: 'var(--surface-panel)',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: '16px', color: 'var(--text-secondary)' }}>
            tune
          </span>
          <span
            className="flex-1 mono-text"
            style={{ fontSize: 'var(--text-xs)', color: 'var(--text-primary)', fontWeight: 600 }}
          >
            Select Variables to Chart
          </span>
          <span style={{ fontSize: '10px', color: 'var(--text-muted)' }}>
            {selected.length} selected
          </span>
          <button
            onClick={onClose}
            aria-label="Close picker"
            style={{
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              color: 'var(--text-muted)',
              padding: '0 2px',
            }}
            title="Close"
          >
            <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>close</span>
          </button>
        </div>

        {/* Search + global actions */}
        <div className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: '1px solid var(--border-default)' }}>
          <input
            data-testid="plot-picker-search"
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search variables..."
            style={{
              flex: 1,
              background: 'var(--surface-sunken)',
              border: '1px solid var(--border-default)',
              borderRadius: 4,
              color: 'var(--text-primary)',
              fontSize: '11px',
              padding: '4px 8px',
              outline: 'none',
            }}
          />
          <button
            onClick={clearAll}
            disabled={selected.length === 0}
            style={{
              background: 'transparent',
              border: '1px solid var(--border-default)',
              color: selected.length === 0 ? 'var(--border-default)' : 'var(--text-muted)',
              borderRadius: 4,
              fontSize: '10px',
              padding: '4px 8px',
              cursor: selected.length === 0 ? 'not-allowed' : 'pointer',
            }}
            title="Clear all selected"
          >
            Clear all
          </button>
        </div>

        {/* Body — domain groups */}
        <div className="flex-1 overflow-auto" style={{ padding: '8px 12px' }}>
          {totalShown === 0 ? (
            <div style={{ fontSize: '11px', color: 'var(--text-muted)', padding: '12px 0' }}>
              {allVariables.length === 0
                ? 'No variables available yet — start the simulation to populate the snapshot.'
                : `No variables match "${query}".`}
            </div>
          ) : (
            DOMAIN_ORDER.map((domain) => {
              const list = grouped.get(domain) ?? [];
              if (list.length === 0) return null;
              const allInDomainSelected = list.every((n) => selectedSet.has(n));
              return (
                <div key={domain} style={{ marginBottom: '12px' }}>
                  <div
                    className="flex items-center gap-2"
                    style={{
                      marginBottom: '4px',
                      paddingBottom: '2px',
                      borderBottom: '1px solid var(--border-default)',
                    }}
                  >
                    <span
                      style={{
                        width: 8,
                        height: 8,
                        borderRadius: '50%',
                        background: DOMAIN_ACCENT[domain],
                        display: 'inline-block',
                      }}
                    />
                    <span
                      className="flex-1 mono-text"
                      style={{ fontSize: '10px', fontWeight: 600, color: 'var(--text-primary)' }}
                    >
                      {DOMAIN_LABELS[domain]} ({list.length})
                    </span>
                    <button
                      onClick={() => (allInDomainSelected ? removeMany(list) : setMany(list))}
                      style={{
                        background: 'transparent',
                        border: 'none',
                        color: 'var(--accent-fg)',
                        fontSize: '10px',
                        cursor: 'pointer',
                        padding: '0 4px',
                      }}
                    >
                      {allInDomainSelected ? 'Clear domain' : 'Select all'}
                    </button>
                  </div>
                  <div className="flex flex-col">
                    {list.map((name) => {
                      const checked = selectedSet.has(name);
                      return (
                        <label
                          key={name}
                          className="flex items-center gap-2 mono-text"
                          style={{
                            fontSize: '11px',
                            padding: '2px 4px',
                            cursor: 'pointer',
                            color: checked ? 'var(--text-primary)' : 'var(--text-muted)',
                            borderRadius: 2,
                          }}
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => {
                              if (checked) removeMany([name]);
                              else setMany([name]);
                            }}
                            style={{ accentColor: DOMAIN_ACCENT[domain] }}
                          />
                          <span style={{ flex: 1 }}>{name}</span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-end gap-2 px-3 py-2"
          style={{
            borderTop: '1px solid var(--border-default)',
            background: 'var(--surface-panel)',
          }}
        >
          <button
            onClick={onClose}
            style={{
              background: 'var(--accent)',
              color: 'var(--on-accent)',
              border: 'none',
              borderRadius: 4,
              fontSize: '11px',
              padding: '4px 12px',
              cursor: 'pointer',
              fontWeight: 600,
            }}
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
