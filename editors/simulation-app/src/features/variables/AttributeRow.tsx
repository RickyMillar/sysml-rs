/**
 * AttributeRow — one attribute with its live value.
 *
 * Phase A3 scaffolding: the Phase B session tree will show one of these
 * rows for every `AttributeUsage` under a focused part (see
 * layout). It's a cleaner, tree-free version of `VariableRow` — no
 * chevron, no group/leaf branching, no `VariableTreeNode` coupling — so
 * the same markup can be reused in the detail region, in any inline
 * attribute picker, or in future hover previews.
 *
 * Design notes:
 *   - Dumb. Consumer owns pin state and click behaviour.
 *   - Flash-on-change is retained because the detail region wants the
 *     same "value just moved" pulse that Plots does today.
 *   - Sparkline is opt-in via `sparklineSamples.length >= 3` — same
 *     threshold VariableRow uses. The tree shows sparklines on HOVER
 *     (plan Part I "Sparklines in the tree"), but the component itself
 *     is stateless about that — the consumer passes an empty array
 *     when the row isn't hovered.
 *   - Edit pencil is gated behind `editable` + `onEdit`. The override
 *     flow itself lives in the consumer (Phase B); the row just fires
 *     the callback.
 *   - Uses the same `sysml-variable-flash` CSS keyframe that
 *     VariableRow injects, so both render identically when mixed on
 *     the page.
 */

import { memo, useEffect, useRef, useState } from 'react';
import type { CSSProperties, MouseEvent, ReactNode } from 'react';
import { VerdictBadge } from '@/components/VerdictBadge';
import { Sparkline } from './Sparkline';
import type { ConstraintVerdict, VariableValue } from './VariableTree';
import { formatVariableValue } from './VariableTree';

export interface AttributeRowProps {
  /** Stable identifier used as the React key + testid suffix. */
  id: string;
  /** Display label — a leaf segment like `bimetalTemp`. */
  name: string;
  /** Current live value. Formatted via `formatVariableValue`. */
  value: VariableValue;
  /** Optional unit (e.g. "K", "A", "V") appended to the formatted value. */
  unit?: string;
  /** Optional constraint verdict pill. */
  verdict?: ConstraintVerdict;
  /** Sample ring buffer for the sparkline; empty / short arrays hide it. */
  sparklineSamples?: readonly number[];
  /** Gate the sparkline entirely (pane-level toggle). */
  showSparkline?: boolean;
  /** Is this row pinned? Drives the pin icon state. */
  pinned?: boolean;
  /** Called when the pin affordance is clicked. */
  onTogglePin?: () => void;
  /** Is this row editable? Shows the pencil icon when true. */
  editable?: boolean;
  /** Called when the edit pencil is clicked. */
  onEdit?: () => void;
  /** Primary-click handler (e.g. focus the detail region on this row). */
  onClick?: () => void;
  /** Right-click handler. */
  onContextMenu?: (position: { x: number; y: number }) => void;
  /** Keyboard-selected visual state. */
  selected?: boolean;
  /** Left indent in pixels — 0 by default. The detail region uses 0; a
   *  future embedded-in-tree consumer can pass a multiple of 12. */
  indent?: number;
  /** Tooltip suffix (e.g. "last changed @ tick 471"). */
  lastChangedTick?: number;
  /** Click-sparkline handler (e.g. add to Plots). When provided,
   *  the sparkline becomes a button with its own hit target and does
   *  NOT trigger the row's onClick. */
  onSparklineClick?: () => void;
  /** data-testid prefix (default `attribute-row`). */
  testIdPrefix?: string;
}

const DEFAULT_TEST_ID_PREFIX = 'attribute-row';

function AttributeRowImpl({
  id,
  name,
  value,
  unit,
  verdict,
  sparklineSamples,
  showSparkline = true,
  pinned = false,
  onTogglePin,
  editable = false,
  onEdit,
  onClick,
  onContextMenu,
  selected = false,
  indent = 0,
  lastChangedTick,
  onSparklineClick,
  testIdPrefix = DEFAULT_TEST_ID_PREFIX,
}: AttributeRowProps) {
  const flashRef = useRef<HTMLSpanElement>(null);
  const [flashKey, setFlashKey] = useState(0);
  const prevValueRef = useRef<VariableValue>(value);

  useEffect(() => {
    if (prevValueRef.current !== value) {
      prevValueRef.current = value;
      setFlashKey((k) => k + 1);
    }
  }, [value]);

  const samples = sparklineSamples ?? EMPTY_SAMPLES;
  const renderSparkline = showSparkline && samples.length >= 3;

  const rowStyle: CSSProperties = {
    paddingLeft: 4 + indent,
    paddingRight: 8,
    minHeight: 22,
    background: selected
      ? 'color-mix(in srgb, var(--accent) 14%, transparent)'
      : 'transparent',
    borderLeft: selected ? '2px solid var(--accent)' : '2px solid transparent',
    fontSize: 'var(--text-xs)',
    // Same virtualisation as VariableRow — keeps 1k-row detail regions smooth.
    contentVisibility: 'auto',
    containIntrinsicSize: 'auto 28px',
  };

  const titleSuffix =
    lastChangedTick != null ? ` · last change @ tick ${lastChangedTick}` : '';

  return (
    <div
      role="row"
      data-testid={`${testIdPrefix}-${id}`}
      data-id={id}
      data-pinned={pinned}
      data-selected={selected || undefined}
      className="flex items-center gap-1.5 py-[3px] cursor-pointer select-none transition-colors"
      style={rowStyle}
      onClick={onClick}
      onContextMenu={(e: MouseEvent<HTMLDivElement>) => {
        if (!onContextMenu) return;
        e.preventDefault();
        onContextMenu({ x: e.clientX, y: e.clientY });
      }}
      onMouseEnter={(e) => {
        if (!selected) {
          (e.currentTarget as HTMLDivElement).style.background =
            'color-mix(in srgb, var(--text-primary) 6%, transparent)';
        }
      }}
      onMouseLeave={(e) => {
        if (!selected)
          (e.currentTarget as HTMLDivElement).style.background = 'transparent';
      }}
      title={`${name}${titleSuffix}`}
    >
      {/* Pin toggle / placeholder */}
      <span
        style={{
          width: 14,
          display: 'inline-flex',
          justifyContent: 'center',
          flexShrink: 0,
        }}
      >
        {onTogglePin ? (
          <button
            type="button"
            data-testid={`${testIdPrefix}-${id}-pin`}
            aria-label={pinned ? 'unpin' : 'pin'}
            aria-pressed={pinned}
            onClick={(e) => {
              e.stopPropagation();
              onTogglePin();
            }}
            style={{
              border: 'none',
              background: 'transparent',
              padding: 0,
              cursor: 'pointer',
              color: pinned ? 'var(--chart-series-3)' : 'var(--text-muted)',
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: 12 }}
            >
              push_pin
            </span>
          </button>
        ) : pinned ? (
          <span
            className="material-symbols-outlined"
            aria-label="pinned"
            style={{ fontSize: 11, color: 'var(--chart-series-3)' }}
          >
            push_pin
          </span>
        ) : null}
      </span>

      {/* Label */}
      <span
        className="mono-text truncate"
        style={{
          color: 'var(--text-primary)',
          fontWeight: 400,
          flex: '0 1 auto',
          minWidth: 0,
        }}
        data-testid={`${testIdPrefix}-${id}-label`}
      >
        {name}
      </span>

      {/* Right cluster: sparkline + value + pill + edit */}
      <div
        className="ml-auto flex items-center gap-2"
        style={{ flexShrink: 0 }}
      >
        {renderSparkline && (
          onSparklineClick ? (
            <button
              type="button"
              data-testid={`${testIdPrefix}-${id}-spark-btn`}
              title="Click to add to Plots"
              onClick={(e) => {
                e.stopPropagation();
                onSparklineClick();
              }}
              style={{
                border: 'none',
                background: 'transparent',
                padding: 0,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
              }}
            >
              <Sparkline
                samples={samples as number[]}
                width={60}
                height={16}
                color="color-mix(in srgb, var(--text-secondary) 75%, transparent)"
                ariaLabel={`${name} sparkline — click to add to Plots`}
              />
            </button>
          ) : (
            <Sparkline
              samples={samples as number[]}
              width={60}
              height={16}
              color="color-mix(in srgb, var(--text-secondary) 75%, transparent)"
              ariaLabel={`${name} sparkline`}
            />
          )
        )}
        <FlashValue ref={flashRef} flashKey={flashKey} verdict={verdict}>
          {formatVariableValue(value, unit)}
        </FlashValue>
        {verdict && (
          <VerdictBadge
            verdict={verdict}
            name={name}
            size="compact"
            testId={`${testIdPrefix}-${id}-pill`}
          />
        )}
        {editable && onEdit && (
          <button
            type="button"
            data-testid={`${testIdPrefix}-${id}-edit`}
            aria-label={`edit ${name}`}
            onClick={(e) => {
              e.stopPropagation();
              onEdit();
            }}
            style={{
              border: 'none',
              background: 'transparent',
              padding: 0,
              cursor: 'pointer',
              color: 'var(--text-muted)',
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: 14 }}
            >
              edit
            </span>
          </button>
        )}
      </div>
    </div>
  );
}

const EMPTY_SAMPLES: readonly number[] = [];

export const AttributeRow = memo(AttributeRowImpl);

// ── Flash value (shared CSS with VariableRow) ───────────────────────
// Reuses the same `sysml-variable-flash` keyframe — only injected once
// per document (see `FLASH_STYLE_ID` guard).

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
  useEffect(() => {
    ensureFlashStyle();
  }, []);
  const color =
    verdict === 'pass'
      ? 'var(--verdict-pass)'
      : verdict === 'fail'
        ? 'var(--verdict-fail)'
        : 'var(--text-primary)';
  return (
    <span
      key={flashKey}
      ref={ref}
      className="mono-text font-medium sysml-variable-flash"
      style={{
        color,
        fontSize: 'var(--text-xs)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {children}
    </span>
  );
}
