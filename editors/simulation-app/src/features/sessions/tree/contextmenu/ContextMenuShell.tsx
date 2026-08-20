/**
 * ContextMenuShell — shared chrome for tree right-click menus.
 *
 * Handles outside-click + Esc dismissal, portal rendering (so the
 * tree's scroll container can't clip the menu), and position
 * clamping against the viewport. Each kind-specific menu
 * (`VariablesPaneContextMenu`, `SmContextMenu`, `ConstraintContextMenu`)
 * declares its own header label + item list and lets the shell do
 * everything else.
 */
import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

export interface ContextMenuItem {
  id: string;
  /** Material-symbols icon name. */
  icon: string;
  label: string;
  onClick: () => void;
  /** Optional accent colour for the icon (used to flag breakpoint
   *  red, override purple, etc.). */
  accent?: string;
  /** Render a thin separator line above this item. */
  separator?: boolean;
  /** Disable the row; renders dimmed and ignores clicks. */
  disabled?: boolean;
  /** Optional secondary label rendered to the right (used for
   *  submenu indicators or value previews). */
  trailing?: ReactNode;
}

export interface ContextMenuShellProps {
  /** Controls visibility — null hides. */
  open: boolean;
  /** Header label rendered at the top. Truncates with ellipsis. */
  header: string;
  /** Screen position (clientX/clientY). */
  position: { x: number; y: number };
  /** Item rows. Auto-closes on click via `onClose` after firing. */
  items: ContextMenuItem[];
  onClose: () => void;
  /** Test id passed through to the menu root. */
  testId?: string;
  /** Width for clamping (px). Defaults to 220. */
  width?: number;
}

const DEFAULT_WIDTH = 220;

export function ContextMenuShell({
  open,
  header,
  position,
  items,
  onClose,
  testId = 'context-menu',
  width = DEFAULT_WIDTH,
}: ContextMenuShellProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKey);
    };
  }, [open, onClose]);

  if (!open) return null;

  // Clamp against viewport so the menu can't fall off the right /
  // bottom edge. Item count drives a rough height estimate (header
  // + ~28px per row + padding).
  const heightEstimate = 32 + items.length * 30 + 8;
  const vw = typeof window !== 'undefined' ? window.innerWidth : 1024;
  const vh = typeof window !== 'undefined' ? window.innerHeight : 768;
  const x = Math.min(position.x, vw - width - 4);
  const y = Math.min(position.y, vh - heightEstimate - 4);

  const node = (
    <div
      ref={ref}
      role="menu"
      data-testid={testId}
      className="fixed z-50 flex flex-col py-1 rounded-lg shadow-lg"
      style={{
        left: x,
        top: y,
        minWidth: width,
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
        backdropFilter: 'blur(12px)',
      }}
    >
      <div
        className="px-3 py-1.5 mono-text truncate"
        style={{
          fontSize: 'var(--text-xs)',
          color: 'var(--outline)',
          borderBottom: '1px solid var(--outline-variant)',
          maxWidth: width,
        }}
      >
        {header}
      </div>
      {items.map((item) => (
        <div key={item.id}>
          {item.separator && (
            <div
              style={{
                height: 1,
                background: 'var(--outline-variant)',
                opacity: 0.3,
                margin: '2px 0',
              }}
            />
          )}
          <button
            role="menuitem"
            data-testid={`${testId}-item-${item.id}`}
            disabled={item.disabled}
            onClick={item.onClick}
            className="flex items-center gap-2.5 px-3 py-1.5 text-left transition-colors w-full"
            style={{
              background: 'transparent',
              border: 'none',
              cursor: item.disabled ? 'not-allowed' : 'pointer',
              color: item.disabled ? 'var(--outline)' : 'var(--on-surface)',
              fontSize: 'var(--text-sm)',
              opacity: item.disabled ? 0.5 : 1,
            }}
            onMouseEnter={(e) => {
              if (item.disabled) return;
              e.currentTarget.style.background = 'var(--surface-container-high)';
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.background = 'transparent';
            }}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: '16px', color: item.accent ?? 'var(--outline)' }}
            >
              {item.icon}
            </span>
            <span style={{ flex: '1 1 auto' }}>{item.label}</span>
            {item.trailing && (
              <span
                className="mono-text"
                style={{ fontSize: 'var(--text-xs)', color: 'var(--outline)' }}
              >
                {item.trailing}
              </span>
            )}
          </button>
        </div>
      ))}
    </div>
  );

  if (typeof document === 'undefined') return node;
  return createPortal(node, document.body);
}
