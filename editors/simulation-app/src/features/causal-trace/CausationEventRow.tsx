/**
 * CausationEventRow — single row in the CausalTracePanel timeline (R7.1).
 *
 * Render-only; takes the event, a `depth` index for vertical alignment,
 * and a click handler that the panel wires to a playhead scrub.
 */

import type { CSSProperties, MouseEvent } from 'react';
import type { CausationEvent } from '@/engine/types';
import {
  CAUSATION_KIND_ICON,
  CAUSATION_KIND_LABEL,
  formatCausationEvent,
  formatCausationEventPrefix,
} from './formatCausationEvent';

export interface CausationEventRowProps {
  event: CausationEvent;
  /** Row index in the chain (0 = root). Used only for test accessibility keys. */
  index: number;
  /** True when this row is the root event (shown at the top, highlighted). */
  isRoot: boolean;
  /** Callback when the user clicks the row. Panel maps this to a playhead scrub. */
  onClick?: (event: CausationEvent) => void;
}

// Root vs branch previously differed by the (now-neutralized) panel accent hue;
// weight now carries the distinction instead, per tokens.css's "selection owns
// border weight" convention.
const ROOT_BORDER = '1px solid var(--border-strong)';
const BRANCH_BORDER = '1px solid var(--border-default)';

export function CausationEventRow({
  event,
  index,
  isRoot,
  onClick,
}: CausationEventRowProps) {
  const icon = CAUSATION_KIND_ICON[event.kind];
  const kindLabel = CAUSATION_KIND_LABEL[event.kind];
  const summary = formatCausationEvent(event);
  const prefix = formatCausationEventPrefix(event);

  const style: CSSProperties = {
    display: 'flex',
    alignItems: 'flex-start',
    gap: 'var(--space-2, 8px)',
    padding: 'var(--space-2, 8px) var(--space-3, 12px)',
    border: isRoot ? ROOT_BORDER : BRANCH_BORDER,
    borderRadius: 'var(--radius-sm, 4px)',
    background: isRoot
      ? 'color-mix(in oklch, var(--border-default) 14%, transparent)'
      : 'var(--surface-sunken)',
    cursor: onClick ? 'pointer' : 'default',
    marginBottom: 'var(--space-2, 8px)',
    fontSize: 'var(--text-sm, 13px)',
    lineHeight: 1.3,
  };

  const handleClick = (e: MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    onClick?.(event);
  };

  return (
    <button
      type="button"
      data-testid={`causation-row-${index}`}
      data-event-id={event.id}
      data-event-kind={event.kind}
      aria-label={`${kindLabel}: ${summary}`}
      onClick={handleClick}
      style={{
        ...style,
        // Override <button> defaults so the row looks like a list item.
        textAlign: 'left',
        width: '100%',
        font: 'inherit',
      }}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{
          fontSize: '18px',
          lineHeight: 1.2,
          color: isRoot
            ? 'var(--text-secondary)'
            : 'var(--text-muted)',
          flex: '0 0 auto',
        }}
      >
        {icon}
      </span>
      <span style={{ flex: '1 1 auto', minWidth: 0 }}>
        <span
          style={{
            display: 'block',
            color: 'var(--text-secondary)',
            fontSize: 'var(--text-xs, 11px)',
            marginBottom: '2px',
          }}
        >
          {prefix} · {kindLabel}
        </span>
        <span
          style={{
            display: 'block',
            color: 'var(--text-primary)',
            wordBreak: 'break-word',
            fontWeight: isRoot ? 600 : 400,
          }}
        >
          {summary}
        </span>
      </span>
    </button>
  );
}
