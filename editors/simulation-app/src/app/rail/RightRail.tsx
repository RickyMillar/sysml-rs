/**
 * RightRail — the slide-in right-rail host (ninebar Phase 1, task #9).
 *
 * Replaces the closed placeholder `<aside data-testid="right-rail">` in
 * `AppShell.tsx`. Always mounted (so the five-slot testid contract holds
 * regardless of state) — slides in/out via `width` between 0 and
 * `var(--rail-right-width)`, animated over `var(--motion-panel)`. Off by
 * default: with no pinned and no transient context, `data-state="closed"`
 * and width 0, matching the `tests/ninebar-shell.spec.ts` contract.
 *
 * Renders up to two stacked contexts (pinned first, transient second —
 * audit F3, "max two"), each with its own header row (title, pin toggle,
 * close button) at `--row-default` height; when both are present they
 * split the rail vertically via equal `flex: 1`.
 */
import { getRailContext } from './railRegistry';
import { useRightRailStore } from './railStore';
// Side-effect import — registers the built-in rail contexts (currently
// just 'stream-status', the Phase 1 proof context).
import './contexts';

export function RightRail() {
  const pinned = useRightRailStore((s) => s.pinned);
  const transient = useRightRailStore((s) => s.transient);
  const pin = useRightRailStore((s) => s.pin);
  const unpin = useRightRailStore((s) => s.unpin);
  const close = useRightRailStore((s) => s.close);

  // Pinned first, transient second — stable stacking order regardless of
  // which slot was populated more recently.
  const stack: Array<{ id: string; isPinned: boolean }> = [
    ...(pinned ? [{ id: pinned, isPinned: true }] : []),
    ...(transient ? [{ id: transient, isPinned: false }] : []),
  ];
  const isOpen = stack.length > 0;

  return (
    <aside
      data-testid="right-rail"
      data-state={isOpen ? 'open' : 'closed'}
      className="shrink-0 flex flex-col overflow-hidden"
      style={{
        width: isOpen ? 'var(--rail-right-width)' : 0,
        background: 'var(--surface-panel)',
        borderLeft: isOpen ? '1px solid var(--border-default)' : 'none',
        transition: 'width var(--motion-panel) ease',
      }}
    >
      {stack.map(({ id, isPinned }, i) => {
        const descriptor = getRailContext(id);
        if (!descriptor) return null;
        return (
          <div
            key={id}
            data-testid={`right-rail-context-${id}`}
            className="flex flex-col overflow-hidden"
            style={{
              flex: '1 1 0',
              minHeight: 0,
              borderBottom: i < stack.length - 1 ? '1px solid var(--border-default)' : 'none',
            }}
          >
            <div
              className="flex items-center gap-2 shrink-0 px-3"
              style={{
                height: 'var(--row-default)',
                borderBottom: '1px solid var(--border-default)',
              }}
            >
              {descriptor.icon && (
                <span
                  aria-hidden="true"
                  className="material-symbols-outlined"
                  style={{ fontSize: 16, color: 'var(--text-secondary)' }}
                >
                  {descriptor.icon}
                </span>
              )}
              <span
                className="flex-1"
                style={{
                  fontSize: 'var(--text-sm)',
                  fontWeight: 500,
                  color: 'var(--text-primary)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {descriptor.title}
              </span>
              <button
                type="button"
                data-testid={`right-rail-pin-${id}`}
                onClick={() => (isPinned ? unpin() : pin(id))}
                aria-pressed={isPinned}
                aria-label={isPinned ? `Unpin ${descriptor.title}` : `Pin ${descriptor.title}`}
                className="material-symbols-outlined"
                style={{
                  fontSize: 16,
                  color: isPinned ? 'var(--accent-fg)' : 'var(--text-muted)',
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  padding: 2,
                }}
              >
                keep
              </button>
              <button
                type="button"
                data-testid={`right-rail-close-${id}`}
                onClick={() => (isPinned ? unpin() : close())}
                aria-label={`Close ${descriptor.title}`}
                className="material-symbols-outlined"
                style={{
                  fontSize: 16,
                  color: 'var(--text-muted)',
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  padding: 2,
                }}
              >
                close
              </button>
            </div>
            <div className="flex-1 overflow-auto" style={{ minHeight: 0 }}>
              {descriptor.render()}
            </div>
          </div>
        );
      })}
    </aside>
  );
}
