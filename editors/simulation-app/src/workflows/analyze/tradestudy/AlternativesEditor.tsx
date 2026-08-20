/**
 * AlternativesEditor — compact per-alternative editor for the trade study
 * config panel.
 *
 * Each alternative row has:
 *   - a label text input (renamed in place)
 *   - a collapsed summary "N overrides" chip
 *   - an expand/collapse toggle that reveals:
 *       • one key / number-value pair per override
 *       • an "Add parameter" button to insert new key/value pairs
 *   - a remove (×) button
 *
 * UI-only — the actual state lives in `useTradeStudyConfig` and is
 * passed in via the `config` prop. Matches the split-panel convention of
 * the VerifyConfig component.
 */

import { useState } from 'react';
import type { TradeStudyConfigState, AlternativeConfig } from './useTradeStudyConfig';

export interface AlternativesEditorProps {
  config: TradeStudyConfigState;
}

export function AlternativesEditor({ config }: AlternativesEditorProps) {
  const {
    alternatives,
    addAlternative,
    removeAlternative,
    renameAlternative,
    setOverride,
    removeOverride,
  } = config;

  return (
    <section
      data-testid="tradestudy-config-alternatives"
      className="flex flex-col gap-2 px-3 py-3"
      style={{ borderBottom: '1px solid var(--outline-variant)' }}
    >
      <div className="flex items-center gap-2">
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Alternatives
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="tradestudy-alternatives-count"
        >
          {alternatives.length}
        </span>
      </div>

      {alternatives.length === 0 ? (
        <div
          data-testid="tradestudy-alternatives-empty"
          style={{
            fontSize: 11,
            color: 'var(--outline)',
            lineHeight: 1.4,
          }}
        >
          Add at least two design alternatives to compare.
        </div>
      ) : (
        <ul
          data-testid="tradestudy-alternatives-list"
          style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 4 }}
        >
          {alternatives.map((alt) => (
            <AlternativeRow
              key={alt.id}
              alternative={alt}
              onRename={(label) => renameAlternative(alt.id, label)}
              onRemove={() => removeAlternative(alt.id)}
              onSetOverride={(k, v) => setOverride(alt.id, k, v)}
              onRemoveOverride={(k) => removeOverride(alt.id, k)}
            />
          ))}
        </ul>
      )}

      <button
        type="button"
        data-testid="tradestudy-add-alternative"
        onClick={() => addAlternative()}
        style={{
          height: 26,
          background: 'var(--surface-container)',
          color: 'var(--primary)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 6,
          fontSize: 11,
          cursor: 'pointer',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 4,
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
          add
        </span>
        Add alternative
      </button>
    </section>
  );
}

// ── AlternativeRow ──────────────────────────────────────────────────

function AlternativeRow({
  alternative,
  onRename,
  onRemove,
  onSetOverride,
  onRemoveOverride,
}: {
  alternative: AlternativeConfig;
  onRename: (label: string) => void;
  onRemove: () => void;
  onSetOverride: (key: string, value: number | string | boolean) => void;
  onRemoveOverride: (key: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const overrideKeys = Object.keys(alternative.overrides);

  return (
    <li
      data-testid={`tradestudy-alternative-${alternative.id}`}
      style={{
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
        padding: 6,
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          aria-label={expanded ? 'Collapse' : 'Expand'}
          data-testid={`tradestudy-alternative-expand-${alternative.id}`}
          onClick={() => setExpanded((v) => !v)}
          style={{
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--outline)',
            padding: 0,
            display: 'flex',
            alignItems: 'center',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            {expanded ? 'expand_more' : 'chevron_right'}
          </span>
        </button>
        <input
          type="text"
          data-testid={`tradestudy-alternative-label-${alternative.id}`}
          value={alternative.label}
          onChange={(e) => onRename(e.target.value)}
          style={{
            flex: 1,
            height: 24,
            padding: '0 6px',
            background: 'var(--surface-container-high)',
            color: 'var(--on-surface)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 4,
            fontSize: 12,
          }}
        />
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)' }}
        >
          {overrideKeys.length} override{overrideKeys.length === 1 ? '' : 's'}
        </span>
        <button
          type="button"
          aria-label="Remove alternative"
          data-testid={`tradestudy-alternative-remove-${alternative.id}`}
          onClick={onRemove}
          style={{
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--outline)',
            display: 'flex',
            alignItems: 'center',
            padding: 2,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            close
          </span>
        </button>
      </div>

      {expanded && (
        <div
          data-testid={`tradestudy-alternative-overrides-${alternative.id}`}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            paddingLeft: 20,
          }}
        >
          {overrideKeys.map((k) => (
            <OverrideRow
              key={k}
              paramKey={k}
              value={alternative.overrides[k]}
              onChangeValue={(v) => onSetOverride(k, v)}
              onRemove={() => onRemoveOverride(k)}
            />
          ))}
          <AddOverrideRow
            onAdd={(k, v) => {
              if (k.trim()) onSetOverride(k.trim(), v);
            }}
          />
        </div>
      )}
    </li>
  );
}

function OverrideRow({
  paramKey,
  value,
  onChangeValue,
  onRemove,
}: {
  paramKey: string;
  value: unknown;
  onChangeValue: (v: number | string | boolean) => void;
  onRemove: () => void;
}) {
  const text = typeof value === 'number' || typeof value === 'string' || typeof value === 'boolean'
    ? String(value)
    : JSON.stringify(value);
  return (
    <div className="flex items-center gap-1">
      <span
        className="mono-text"
        style={{ fontSize: 11, color: 'var(--on-surface-variant)', minWidth: 0, flex: 1 }}
      >
        {paramKey}
      </span>
      <input
        type="text"
        data-testid={`tradestudy-override-value-${paramKey}`}
        value={text}
        onChange={(e) => {
          const raw = e.target.value;
          const asNum = Number(raw);
          if (raw.trim() !== '' && !Number.isNaN(asNum)) {
            onChangeValue(asNum);
          } else {
            onChangeValue(raw);
          }
        }}
        style={{
          width: 80,
          height: 22,
          padding: '0 4px',
          background: 'var(--surface-container-high)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          fontSize: 11,
          fontFamily: 'monospace',
        }}
      />
      <button
        type="button"
        aria-label="Remove parameter"
        data-testid={`tradestudy-override-remove-${paramKey}`}
        onClick={onRemove}
        style={{
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--outline)',
          display: 'flex',
          alignItems: 'center',
          padding: 2,
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
          close
        </span>
      </button>
    </div>
  );
}

function AddOverrideRow({ onAdd }: { onAdd: (key: string, value: number | string) => void }) {
  const [key, setKey] = useState('');
  const [val, setVal] = useState('');
  return (
    <div className="flex items-center gap-1">
      <input
        type="text"
        placeholder="param"
        data-testid="tradestudy-override-add-key"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        style={{
          flex: 1,
          height: 22,
          padding: '0 4px',
          background: 'var(--surface-container-high)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          fontSize: 11,
          fontFamily: 'monospace',
        }}
      />
      <input
        type="text"
        placeholder="value"
        data-testid="tradestudy-override-add-value"
        value={val}
        onChange={(e) => setVal(e.target.value)}
        style={{
          width: 80,
          height: 22,
          padding: '0 4px',
          background: 'var(--surface-container-high)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          fontSize: 11,
          fontFamily: 'monospace',
        }}
      />
      <button
        type="button"
        data-testid="tradestudy-override-add-commit"
        onClick={() => {
          if (!key.trim()) return;
          const asNum = Number(val);
          const out = val.trim() !== '' && !Number.isNaN(asNum) ? asNum : val;
          onAdd(key, out);
          setKey('');
          setVal('');
        }}
        style={{
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--primary)',
          display: 'flex',
          alignItems: 'center',
          padding: 2,
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
          add
        </span>
      </button>
    </div>
  );
}
