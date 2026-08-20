/**
 * VariableRow — one node (group or leaf) in the Variables pane tree.
 *
 * Memoised so scroll-heavy tree renders don't re-compute rows whose
 * (value, constraint verdict, pinned, selected) tuple hasn't changed.
 * Flash animation on value change uses a CSS-driven `animation-name`
 * keyframe so React state stays clean between ticks.
 *
 * In `live` mode (active streaming session) the leaf's displayed value
 * comes from this row's own `useVar`/`useStringVar` subscription, not
 * from `node.entry.value` — VariablesPane only rebuilds the tree on
 * cheap tick/name-list triggers, so a single value change never forces
 * a fresh `node` prop. The row still updates because the store
 * subscription re-renders this component instance directly (F15
 * guardrail — see VariablesPane.tsx).
 */

import { memo, useEffect, useRef, useState } from 'react';
import type { CSSProperties, MouseEvent, ReactNode } from 'react';
import { VerdictBadge } from '@/components/VerdictBadge';
import { useVar, useStringVar } from '@/features/sessions/sessionLiveStore';
import { Sparkline } from './Sparkline';
import type { ConstraintVerdict, VariableTreeNode, VariableValue } from './VariableTree';
import { formatVariableValue } from './VariableTree';

export interface VariableRowProps {
  node: VariableTreeNode;
  /** Current tree collapse state for groups (ignored on leaves). */
  collapsed: boolean;
  onToggleCollapse: (path: string) => void;
  /** Whether this row is selected via keyboard nav. */
  selected: boolean;
  onSelect: (path: string) => void;
  /** Pin state (leaves only). */
  pinned: boolean;
  /** Whether to render the sparkline column (honours pane-level toggle). */
  showSparkline: boolean;
  /** Last N samples for this leaf — empty array for groups or untracked leaves. */
  sparklineSamples: number[];
  /** Called when the user right-clicks this row. */
  onContextMenu: (path: string, position: { x: number; y: number }) => void;
  /** Called on primary click (leaves drill into plot, groups collapse). */
  onActivate: (path: string) => void;
  /** Controls where the pinned virtual-group pseudo-depth lives. */
  indent?: number;
  /**
   * When true, this row sources its displayed value directly from the
   * session-live store (`useVar`/`useStringVar`) instead of trusting
   * `node.entry.value` — VariablesPane no longer keeps that field
   * reactive at tick rate (F15 guardrail; see VariablesPane.tsx). This
   * is what makes "one changed variable re-renders one row" true: the
   * live subscription re-renders THIS row instance directly, independent
   * of whether the pane (or any sibling row) re-rendered at all. Default
   * false — falls back to `node.entry.value` (archive / compare / no
   * stream, where there's no live store to read from).
   */
  live?: boolean;
}

/**
 * Constraint pill metadata kept as a stable export for context-menu / test
 * code that wants to reason about verdict aria labels without instantiating
 * the full `<VerdictBadge>`. The visual rendering of each verdict lives in
 * `VerdictBadge` (R2.5) — this table is informational only.
 */
export const CONSTRAINT_PILL: Record<ConstraintVerdict, { label: string; ariaLabel: string }> = {
  pass:         { label: 'P', ariaLabel: 'constraint passing' },
  fail:         { label: 'F', ariaLabel: 'constraint failing' },
  inconclusive: { label: 'I', ariaLabel: 'constraint inconclusive' },
  error:        { label: 'E', ariaLabel: 'constraint evaluator error' },
};

/** Public helper — used by the context menu + external consumers. */
export function pillForVerdict(v: ConstraintVerdict | undefined) {
  return v ? CONSTRAINT_PILL[v] : null;
}

const INDENT_PX = 12; // per R2.2 brief

function VariableRowImpl({
  node,
  collapsed,
  onToggleCollapse,
  selected,
  onSelect,
  pinned,
  showSparkline,
  sparklineSamples,
  onContextMenu,
  onActivate,
  indent,
  live,
}: VariableRowProps) {
  const flashRef = useRef<HTMLSpanElement>(null);
  const [flashKey, setFlashKey] = useState(0);

  // Live value subscription (F15 guardrail): when `live`, read this row's
  // own current value straight from the session-live store via per-key
  // selectors rather than `node.entry.value`. Both hooks are called
  // unconditionally (rules-of-hooks — `live`/`node.isLeaf` can't gate the
  // call itself) but their result is only trusted when live; a variable
  // lives in exactly one of scalar_vars / string_vars, so at most one of
  // these is ever defined.
  const scalarLive = useVar(node.path);
  const stringLive = useStringVar(node.path);
  const value: VariableValue | undefined = live && node.entry
    ? (scalarLive !== undefined ? scalarLive : stringLive !== undefined ? stringLive : node.entry.value)
    : node.entry?.value;

  const prevValueRef = useRef<unknown>(value);

  // Trigger the 300ms pulse whenever the leaf's value changes.
  useEffect(() => {
    if (!node.entry) return;
    if (prevValueRef.current !== value) {
      prevValueRef.current = value;
      setFlashKey((k) => k + 1);
    }
  }, [value, node.entry]);

  const depth = indent ?? node.depth;
  const paddingLeft = 4 + depth * INDENT_PX;
  const isGroup = node.children.length > 0 || !node.isLeaf;
  const verdict = node.entry?.constraint;
  const pill = verdict ? CONSTRAINT_PILL[verdict] : null;

  const handleClick = () => {
    onSelect(node.path);
    if (isGroup) onToggleCollapse(node.path);
    else onActivate(node.path);
  };

  const handleContextMenu = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    if (!node.isLeaf) return; // groups: no context menu today
    onSelect(node.path);
    onContextMenu(node.path, { x: e.clientX, y: e.clientY });
  };

  const rowStyle: CSSProperties = {
    paddingLeft,
    paddingRight: 8,
    minHeight: 22,
    background: selected ? 'color-mix(in srgb, var(--accent) 14%, transparent)' : 'transparent',
    borderLeft: selected ? '2px solid var(--accent)' : '2px solid transparent',
    fontSize: 'var(--text-xs)',
    // Lazy-render off-screen rows: the browser skips layout / paint /
    // style for rows outside the viewport (Chromium + Firefox 125+ +
    // Safari). Gives 14k-row panes ~60 FPS scroll without a JS
    // virtualizer. `auto 28px` = use 28 px as the initial placeholder
    // but remember the actual rendered size after first layout so rows
    // with sparklines / long values don't momentarily overflow their
    // 28 px reservation when scrolled into view.
    contentVisibility: 'auto',
    containIntrinsicSize: 'auto 28px',
  };

  return (
    <div
      role={isGroup ? 'treeitem' : 'row'}
      data-testid={`variable-row-${node.path}`}
      aria-expanded={isGroup ? !collapsed : undefined}
      aria-selected={selected || undefined}
      className="flex items-center gap-1.5 py-[3px] cursor-pointer select-none transition-colors"
      style={rowStyle}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
      onMouseEnter={(e) => {
        if (!selected) {
          (e.currentTarget as HTMLDivElement).style.background =
            'color-mix(in srgb, var(--text-primary) 6%, transparent)';
        }
      }}
      onMouseLeave={(e) => {
        if (!selected) (e.currentTarget as HTMLDivElement).style.background = 'transparent';
      }}
      title={`${node.path}${node.entry?.lastChangedTick != null ? ` · last change @ tick ${node.entry.lastChangedTick}` : ''}`}
    >
      {/* Expand chevron (groups) / pin marker (pinned leaves) / spacer (plain leaves) */}
      <span style={{ width: 12, display: 'inline-flex', justifyContent: 'center', flexShrink: 0 }}>
        {isGroup ? (
          <span
            className="material-symbols-outlined"
            style={{
              fontSize: '12px',
              color: 'var(--text-muted)',
              transform: collapsed ? undefined : 'rotate(90deg)',
              transition: 'transform 0.12s',
            }}
          >
            chevron_right
          </span>
        ) : pinned ? (
          <span
            className="material-symbols-outlined"
            aria-label="pinned"
            style={{ fontSize: '11px', color: 'var(--chart-series-3)' }}
          >
            push_pin
          </span>
        ) : null}
      </span>

      {/* Label */}
      <span
        className="mono-text truncate"
        style={{
          color: isGroup ? 'var(--text-secondary)' : 'var(--text-primary)',
          fontWeight: isGroup ? 600 : 400,
          flex: '0 1 auto',
          minWidth: 0,
        }}
      >
        {node.label}
      </span>

      {/* Group count badge */}
      {isGroup && (
        <span
          className="ml-auto"
          style={{ color: 'var(--text-muted)', fontSize: '9px', flexShrink: 0 }}
        >
          {node.leafCount}
        </span>
      )}

      {/* Leaf content: sparkline + value + pill */}
      {node.isLeaf && node.entry && (
        <div
          className="ml-auto flex items-center gap-2"
          style={{ flexShrink: 0 }}
        >
          {showSparkline && sparklineSamples.length >= 3 && (
            <Sparkline
              samples={sparklineSamples}
              width={60}
              height={16}
              color="color-mix(in srgb, var(--text-secondary) 75%, transparent)"
              ariaLabel={`${node.path} sparkline`}
            />
          )}
          <FlashValue
            ref={flashRef}
            flashKey={flashKey}
            verdict={verdict}
          >
            {formatVariableValue(value ?? null, node.entry.unit)}
          </FlashValue>
          {verdict && (
            <VerdictBadge
              verdict={verdict}
              name={node.path}
              size="compact"
              testId={`variable-pill-${node.path}`}
            />
          )}
        </div>
      )}
    </div>
  );
}

export const VariableRow = memo(VariableRowImpl);

// ── Flash value ──────────────────────────────────────────────────────
// Renders the formatted value; bumps a key on each change to retrigger
// a CSS animation. The animation is defined inline via a <style> tag
// injected once per document, so we don't depend on Tailwind extensions.

const FLASH_STYLE_ID = 'sysml-variables-flash';
const FLASH_CSS = `
@keyframes sysml-variable-flash {
  0% { background-color: color-mix(in srgb, var(--chart-series-2) 45%, transparent); }
  100% { background-color: transparent; }
}
.sysml-variable-flash {
  animation: sysml-variable-flash 300ms ease-out;
  border-radius: 3px;
  padding: 0 3px;
}
`;

function ensureFlashStyle() {
  if (typeof document === 'undefined') return;
  if (document.getElementById(FLASH_STYLE_ID)) return;
  const el = document.createElement('style');
  el.id = FLASH_STYLE_ID;
  el.textContent = FLASH_CSS;
  document.head.appendChild(el);
}

interface FlashValueProps {
  children: ReactNode;
  flashKey: number;
  verdict?: ConstraintVerdict;
  ref?: React.Ref<HTMLSpanElement>;
}

function FlashValue({ children, flashKey, verdict, ref }: FlashValueProps) {
  useEffect(() => { ensureFlashStyle(); }, []);
  const color = verdict === 'pass'
    ? 'var(--verdict-pass)'
    : verdict === 'fail'
      ? 'var(--verdict-fail)'
      : 'var(--text-primary)';
  return (
    <span
      key={flashKey}
      ref={ref}
      className="mono-text font-medium sysml-variable-flash"
      style={{ color, fontSize: 'var(--text-xs)', fontVariantNumeric: 'tabular-nums' }}
    >
      {children}
    </span>
  );
}
