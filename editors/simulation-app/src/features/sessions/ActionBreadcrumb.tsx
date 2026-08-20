/**
 * ActionBreadcrumb — renders the subaction drill-down breadcrumb trail.
 *
 * Shows "Root > ParentAction > CurrentAction". Click any crumb to
 * navigate back to that scope level (truncating deeper entries).
 *
 * Only rendered when focusedActionPath is non-empty.
 */

import { useSessionStore } from './store';

export function ActionBreadcrumb() {
  const focusedActionPath = useSessionStore((s) => s.focusedActionPath);
  const navigateToActionDepth = useSessionStore((s) => s.navigateToActionDepth);

  if (focusedActionPath.length === 0) return null;

  return (
    <div
      className="flex items-center gap-1 px-3 shrink-0"
      style={{
        height: 28,
        borderBottom: '1px solid var(--outline-variant)',
        fontSize: '11px',
        color: 'var(--on-surface-variant)',
        background: 'var(--surface-container)',
        overflow: 'hidden',
      }}
    >
      {/* Root crumb */}
      <button
        onClick={() => navigateToActionDepth(0)}
        style={{
          background: 'none',
          border: 'none',
          padding: '2px 4px',
          cursor: 'pointer',
          color: 'var(--primary)',
          fontSize: '11px',
          fontWeight: 500,
          borderRadius: 4,
        }}
      >
        Root
      </button>

      {/* Path crumbs */}
      {focusedActionPath.map((name, i) => {
        const isLast = i === focusedActionPath.length - 1;
        return (
          <span key={i} className="flex items-center gap-1">
            <span
              className="material-symbols-outlined"
              style={{ fontSize: '12px', color: 'var(--outline)' }}
            >
              chevron_right
            </span>
            {isLast ? (
              <span
                style={{
                  fontWeight: 600,
                  color: 'var(--on-surface)',
                  padding: '2px 4px',
                }}
              >
                {name}
              </span>
            ) : (
              <button
                onClick={() => navigateToActionDepth(i + 1)}
                style={{
                  background: 'none',
                  border: 'none',
                  padding: '2px 4px',
                  cursor: 'pointer',
                  color: 'var(--primary)',
                  fontSize: '11px',
                  fontWeight: 500,
                  borderRadius: 4,
                }}
              >
                {name}
              </button>
            )}
          </span>
        );
      })}
    </div>
  );
}
