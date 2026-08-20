/**
 * CompareVariablePicker — narrow the cross-session compare to a chosen
 * subset of variables.
 *
 * Default state is "auto" (`pickedVariables === null`), which means the
 * workflow picks the top-6 by cross-session variance via
 * `autoPickVariables` in selectors.ts. The user can override by clicking
 * variable chips — toggling out of auto mode and into a manual set.
 *
 * Props take the full variable catalogue (union of variable names across
 * picked sessions) plus the auto-picked default so the chips can show a
 * disabled "on by auto" state clearly.
 */

import type { CSSProperties } from 'react';
import { useCompareStore } from './useCompareStore';

export interface CompareVariablePickerProps {
  /** All variables available across picked sessions (union). */
  availableVariables: string[];
  /** The names the auto-pick algorithm would choose right now. */
  autoPicked: string[];
}

export function CompareVariablePicker({
  availableVariables,
  autoPicked,
}: CompareVariablePickerProps) {
  const picked = useCompareStore((s) => s.pickedVariables);
  const setPicked = useCompareStore((s) => s.setPickedVariables);

  const isAuto = picked === null;
  const effective = isAuto ? autoPicked : picked;
  const effectiveSet = new Set(effective);

  const toggle = (name: string) => {
    const current = isAuto ? autoPicked.slice() : (picked ?? []).slice();
    const has = current.includes(name);
    const next = has ? current.filter((v) => v !== name) : [...current, name];
    setPicked(next);
  };

  const resetToAuto = () => setPicked(null);

  const headerStyle: CSSProperties = {
    fontSize: 10,
    fontWeight: 700,
    textTransform: 'uppercase',
    letterSpacing: '0.06em',
    color: 'var(--outline)',
    padding: '8px 12px',
    borderBottom: '1px solid var(--outline-variant)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 8,
  };

  return (
    <div
      data-testid="compare-variable-picker"
      className="flex flex-col"
      style={{
        background: 'var(--surface-container-low)',
        borderBottom: '1px solid var(--outline-variant)',
      }}
    >
      <div style={headerStyle}>
        <span>Variables</span>
        <div className="flex items-center gap-2">
          <span
            data-testid="compare-variable-picker-mode"
            style={{
              fontSize: 9,
              padding: '1px 6px',
              borderRadius: 3,
              color: isAuto ? 'var(--primary)' : 'var(--tertiary)',
              background: isAuto
                ? 'var(--primary-container, #004b7a22)'
                : 'var(--tertiary-container, #625b7122)',
            }}
          >
            {isAuto ? 'AUTO' : 'MANUAL'}
          </span>
          {!isAuto && (
            <button
              type="button"
              data-testid="compare-variable-picker-reset"
              onClick={resetToAuto}
              style={{
                fontSize: 10,
                padding: '2px 6px',
                background: 'var(--surface-container-high)',
                border: '1px solid var(--outline-variant)',
                borderRadius: 3,
                color: 'var(--on-surface-variant)',
                cursor: 'pointer',
              }}
            >
              Reset
            </button>
          )}
        </div>
      </div>

      <div
        className="flex flex-wrap gap-1"
        style={{ padding: 8, maxHeight: 140, overflowY: 'auto' }}
      >
        {availableVariables.length === 0 && (
          <span
            style={{ fontSize: 11, color: 'var(--outline)', padding: '4px 0' }}
          >
            No variables available yet.
          </span>
        )}
        {availableVariables.map((name) => {
          const active = effectiveSet.has(name);
          return (
            <button
              key={name}
              type="button"
              data-testid={`compare-variable-chip-${name}`}
              data-active={active ? 'true' : 'false'}
              onClick={() => toggle(name)}
              style={{
                fontSize: 10,
                padding: '3px 8px',
                borderRadius: 12,
                border: `1px solid ${
                  active ? 'var(--primary)' : 'var(--outline-variant)'
                }`,
                background: active
                  ? 'var(--primary-container, #004b7a33)'
                  : 'var(--surface-container-high)',
                color: active ? 'var(--on-primary-container, #fff)' : 'var(--on-surface-variant)',
                cursor: 'pointer',
                fontFamily: 'var(--font-mono, monospace)',
              }}
            >
              {name}
            </button>
          );
        })}
      </div>
    </div>
  );
}
