/**
 * CardShell — shared chrome for result workbench panels.
 *
 * The old Run results surface used dismissible inactive cards. The Run
 * surface is now tabbed, so this shell is only responsible for the concrete panel
 * frame, header, and optional export actions.
 */

import type { ReactNode } from 'react';

export interface ExportAction {
  label: string;
  icon: string;
  onClick: () => void;
}

interface CardShellProps {
  title: string;
  icon: string;
  accentColor?: string;
  /** When true, card expands to fill the workbench panel. */
  expanded?: boolean;
  /** Optional header click, retained for non-workbench hosts. */
  onHeaderClick?: () => void;
  /** Export actions rendered as small icon buttons in the header when expanded. */
  exportActions?: ExportAction[];
  children: ReactNode;
}

export function CardShell({
  title,
  icon,
  accentColor = 'var(--text-secondary)',
  expanded = false,
  onHeaderClick,
  exportActions,
  children,
}: CardShellProps) {
  return (
    <div
      className={`${expanded ? '' : 'shrink-0'} flex flex-col rounded-lg overflow-hidden`}
      style={{
        width: expanded ? '100%' : 400,
        minWidth: expanded ? undefined : 320,
        height: expanded ? '100%' : undefined,
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
      }}
    >
      {/* Header */}
      <div
        className="flex items-center gap-2 px-3 py-1.5 shrink-0"
        style={{
          borderBottom: '1px solid var(--outline-variant)',
          background: 'var(--surface-container-low)',
          cursor: onHeaderClick ? 'pointer' : undefined,
        }}
        onClick={onHeaderClick}
      >
        <span className="material-symbols-outlined" style={{ fontSize: '14px', color: accentColor }}>
          {icon}
        </span>
        <span
          className="flex-1 mono-text"
          style={{
            fontSize: 'var(--text-xs)',
            color: 'var(--on-surface)',
            fontWeight: 600,
          }}
        >
          {title}
        </span>
        {/* Export buttons — visible when card is expanded */}
        {expanded && exportActions && exportActions.length > 0 && (
          <div className="flex items-center gap-0.5">
            {exportActions.map((action) => (
              <button
                key={action.label}
                onClick={(e) => { e.stopPropagation(); action.onClick(); }}
                className="shrink-0 rounded transition-colors"
                style={{
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  color: 'var(--outline)',
                  padding: '2px 4px',
                }}
                title={action.label}
              >
                <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>
                  {action.icon}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto" style={{ padding: '8px' }}>
        {children}
      </div>
    </div>
  );
}
