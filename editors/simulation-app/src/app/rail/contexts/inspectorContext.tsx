/**
 * inspector — right-rail context (ninebar Phase 1.5, plan §1 row 3 /
 * Phase 1.5 "Model readiness & Browse floor").
 *
 * Phase 1's five contexts (variables/breakpoints/diagnostics/views/
 * archive) all re-home an existing SESSION- or workspace-tool panel.
 * None of them show "what element is selected" — that's this context.
 *
 * FINDING: the app had no reusable element-inspector component before
 * this. `useSelectionStore` (`features/selection/store.ts`) already
 * fetches full element detail (kind, owner, qualifiedName, props,
 * children, spans) via `GET /models/:uri/elements/:id` (+ `/children`)
 * on every `select()` call — but nothing ever rendered
 * `elementDetail`; a repo-wide grep confirms it was read only inside
 * the store itself. This context is a new, deliberately minimal
 * key-value presentational component over that existing (already
 * fetched, previously unconsumed) data — not a fork of an existing
 * viewer, because none existed.
 */
import type { CSSProperties } from 'react';
import { useSelectionStore } from '@/features/selection/store';
import { registerRailContext } from '../railRegistry';

const LABEL_STYLE: CSSProperties = {
  flex: '0 0 92px',
  color: 'var(--text-secondary)',
  fontSize: 'var(--text-xs)',
  textTransform: 'uppercase',
  letterSpacing: '0.03em',
};

const VALUE_STYLE: CSSProperties = {
  margin: 0,
  flex: 1,
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--text-sm)',
  wordBreak: 'break-word',
  color: 'var(--text-primary)',
};

const SECTION_HEADER_STYLE: CSSProperties = {
  padding: '8px 12px 4px',
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  textTransform: 'uppercase',
  letterSpacing: '0.03em',
};

function InspectorRailContext() {
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const selectedUri = useSelectionStore((s) => s.selectedUri);
  const detail = useSelectionStore((s) => s.elementDetail);
  const loading = useSelectionStore((s) => s.loading);

  if (!selectedElementId) {
    return (
      <div
        data-testid="rail-context-inspector-empty"
        style={{
          padding: 16,
          fontSize: 'var(--text-sm)',
          color: 'var(--text-secondary)',
          textAlign: 'center',
        }}
      >
        Select an element to inspect it.
      </div>
    );
  }

  if (loading || !detail) {
    return (
      <div
        data-testid="rail-context-inspector-loading"
        style={{ padding: 16, fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}
      >
        Loading…
      </div>
    );
  }

  const rows: Array<[string, string]> = [
    ['Kind', detail.kind],
    ['Name', detail.name ?? '(unnamed)'],
    ['Qualified name', detail.qualifiedName ?? '—'],
    ['Owner', detail.owner ?? '—'],
    ['URI', selectedUri ?? '—'],
  ];
  const propEntries = Object.entries(detail.props);

  return (
    <div
      data-testid="rail-context-inspector"
      className="flex flex-col h-full overflow-auto"
    >
      <dl style={{ margin: 0, padding: '8px 12px' }}>
        {rows.map(([label, value]) => (
          <div
            key={label}
            style={{
              display: 'flex',
              gap: 8,
              padding: '4px 0',
              borderBottom: '1px solid var(--border-default)',
            }}
          >
            <dt style={LABEL_STYLE}>{label}</dt>
            <dd style={VALUE_STYLE}>{value}</dd>
          </div>
        ))}
      </dl>

      {propEntries.length > 0 && (
        <>
          <div style={SECTION_HEADER_STYLE}>Properties</div>
          <dl style={{ margin: 0, padding: '0 12px 8px' }}>
            {propEntries.map(([key, value]) => (
              <div key={key} style={{ display: 'flex', gap: 8, padding: '3px 0' }}>
                <dt style={{ ...LABEL_STYLE, fontFamily: 'var(--font-mono)', textTransform: 'none' }}>
                  {key}
                </dt>
                <dd style={{ ...VALUE_STYLE, fontSize: 'var(--text-xs)' }}>{value}</dd>
              </div>
            ))}
          </dl>
        </>
      )}

      {detail.children.length > 0 && (
        <>
          <div style={SECTION_HEADER_STYLE}>Children ({detail.children.length})</div>
          <ul style={{ margin: 0, padding: '0 12px 12px', listStyle: 'none' }}>
            {detail.children.map((child) => (
              <li
                key={child.id}
                style={{ padding: '2px 0', fontSize: 'var(--text-xs)', fontFamily: 'var(--font-mono)' }}
              >
                {child.name ?? '(unnamed)'}{' '}
                <span style={{ color: 'var(--text-secondary)' }}>· {child.kind}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

registerRailContext({
  id: 'inspector',
  title: 'Inspector',
  icon: 'info',
  render: () => <InspectorRailContext />,
});
