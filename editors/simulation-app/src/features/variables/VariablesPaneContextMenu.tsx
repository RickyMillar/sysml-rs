/**
 * VariablesPaneContextMenu — portal right-click menu for variable rows.
 *
 * Actions from the R2.2 brief plus chart toggling (P1 of the
 * tree-as-central-place rework):
 *   1. Add to chart / Remove from chart (toggle by current selection)
 *   2. Override value          (prompts → SessionControl-adjacent mutation)
 *   3. Add threshold breakpoint(prompts → SessionControl.setBreakpoint)
 *   4. Pin / Unpin             (toggles local pane state)
 *   5. Copy name               (writes FQ name to clipboard)
 *
 * Kept deliberately self-contained: rendered via `createPortal` so the
 * tree scroll container doesn't clip the menu. The menu consumes
 * callbacks from VariablesPane rather than reading stores directly —
 * this keeps this component easy to reuse in tests.
 */

import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

export interface VariablesPaneContextMenuProps {
  /** The variable the user right-clicked. Null hides the menu. */
  variableName: string | null;
  /** Whether the variable is currently pinned (drives label). */
  isPinned: boolean;
  /** Whether the variable is already in the active session's chart
   *  selection. Drives the Add/Remove from chart label. */
  isInChart: boolean;
  /** Screen position (clientX/clientY). */
  position: { x: number; y: number };
  /** Called for every item; the menu closes automatically after. */
  onToggleChart: (name: string) => void;
  onOverride: (name: string) => void;
  onAddBreakpoint: (name: string) => void;
  onTogglePin: (name: string) => void;
  onCopyName: (name: string) => void;
  /** Dismiss the menu (Esc, outside click, action fired). */
  onClose: () => void;
}

interface MenuItem {
  id: string;
  icon: string;
  label: string;
  onClick: () => void;
  accent?: string;
  /** Separator rendered *above* this item. */
  separator?: boolean;
}

export function VariablesPaneContextMenu({
  variableName,
  isPinned,
  isInChart,
  position,
  onToggleChart,
  onOverride,
  onAddBreakpoint,
  onTogglePin,
  onCopyName,
  onClose,
}: VariablesPaneContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);

  // Outside-click + Esc dismiss.
  useEffect(() => {
    if (!variableName) return;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const handleKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKey);
    };
  }, [variableName, onClose]);

  if (!variableName) return null;

  const items: MenuItem[] = [
    {
      id: 'chart',
      icon: isInChart ? 'remove_circle' : 'show_chart',
      label: isInChart ? 'Remove from chart' : 'Add to chart',
      onClick: () => { onToggleChart(variableName); onClose(); },
      accent: 'var(--chart-series-8)',
    },
    {
      id: 'override',
      icon: 'edit',
      label: 'Override value',
      onClick: () => { onOverride(variableName); onClose(); },
      accent: 'var(--text-secondary)',
      separator: true,
    },
    {
      id: 'breakpoint',
      icon: 'flag',
      label: 'Add threshold breakpoint',
      onClick: () => { onAddBreakpoint(variableName); onClose(); },
      accent: 'var(--sim-breakpoint-mark)',
    },
    {
      id: 'pin',
      icon: isPinned ? 'keep_off' : 'keep',
      label: isPinned ? 'Unpin' : 'Pin',
      onClick: () => { onTogglePin(variableName); onClose(); },
      separator: true,
    },
    {
      id: 'copy',
      icon: 'content_copy',
      label: 'Copy name',
      onClick: () => { onCopyName(variableName); onClose(); },
    },
  ];

  const MENU_WIDTH = 220;
  const MENU_HEIGHT = 200;
  const x = Math.min(position.x, (typeof window !== 'undefined' ? window.innerWidth : 1024) - MENU_WIDTH - 4);
  const y = Math.min(position.y, (typeof window !== 'undefined' ? window.innerHeight : 768) - MENU_HEIGHT - 4);

  const menu = (
    <div
      ref={ref}
      role="menu"
      aria-label="Variable actions"
      data-testid="variables-pane-context-menu"
      className="fixed z-50 flex flex-col py-1 rounded-lg shadow-lg"
      style={{
        left: x,
        top: y,
        minWidth: MENU_WIDTH,
        background: 'var(--surface-panel)',
        border: '1px solid var(--border-default)',
        backdropFilter: 'blur(12px)',
      }}
    >
      <div
        className="px-3 py-1.5 mono-text truncate"
        style={{
          fontSize: 'var(--text-xs)',
          color: 'var(--text-muted)',
          borderBottom: '1px solid var(--border-default)',
          maxWidth: MENU_WIDTH,
        }}
      >
        {variableName}
      </div>
      {items.map((item) => (
        <div key={item.id}>
          {item.separator && (
            <div style={{ height: 1, background: 'var(--border-default)', opacity: 0.3, margin: '2px 0' }} />
          )}
          <button
            role="menuitem"
            data-testid={`variables-pane-action-${item.id}`}
            onClick={item.onClick}
            className="flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors w-full"
            style={{
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              color: 'var(--text-primary)',
              fontSize: 'var(--text-sm)',
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--surface-raised)'; }}
            onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: '16px', color: item.accent ?? 'var(--text-muted)' }}
            >
              {item.icon}
            </span>
            <span>{item.label}</span>
          </button>
        </div>
      ))}
    </div>
  );

  if (typeof document === 'undefined') return menu;
  return createPortal(menu, document.body);
}
