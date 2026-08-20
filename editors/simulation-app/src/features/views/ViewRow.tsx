/**
 * ViewRow — one row inside `<ViewsPanel />` (Phase 3 extraction).
 *
 * The row was inlined in ViewsPanel until Phase 3 needed per-row
 * hover state and a popover trigger ref. Splitting it out keeps the
 * panel's render straight and gives `<SourcePreviewPopover>` a stable
 * anchor element per row.
 *
 * Click semantics:
 *   - Row click          → `onPick(id)` (the panel's existing
 *     setSelectedViewId behaviour — renders the view).
 *   - Hover-popover click → `onPromote(id)` (Phase 3 — promotes into
 *     the Source utility drawer).
 */
import { useCallback, useRef, useState } from 'react';
import { SourcePreviewPopover } from '@/features/editor/SourcePreviewPopover';
import type { ViewSummary } from './queries';

export interface ViewRowProps {
  view: ViewSummary;
  selected: boolean;
  onPick: (id: string) => void;
  onPromote?: (id: string) => void;
  /** URI passed to the preview popover. */
  previewUri: string | null;
  styles: Record<string, React.CSSProperties>;
  kindLabel: string;
  exposedSummary: string;
}

export function ViewRow({
  view,
  selected,
  onPick,
  onPromote,
  previewUri,
  styles,
  kindLabel,
  exposedSummary,
}: ViewRowProps) {
  const [hovered, setHovered] = useState(false);
  const rowRef = useRef<HTMLDivElement | null>(null);
  const rowStyle = selected
    ? { ...styles.row, ...styles.rowSelected }
    : styles.row;

  const handlePromote = useCallback(() => {
    onPromote?.(view.id);
  }, [onPromote, view.id]);

  return (
    <>
      <div
        ref={rowRef}
        key={view.id}
        style={rowStyle}
        role="button"
        tabIndex={0}
        data-testid={`views-panel-row-${view.id}`}
        onClick={() => onPick(view.id)}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => setHovered(true)}
        onBlur={() => setHovered(false)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onPick(view.id);
          }
        }}
      >
        <div style={styles.rowName}>
          <span style={{ color: 'var(--text-muted)' }}>
            {kindLabel}{' '}
          </span>
          {view.name ?? <em>(unnamed)</em>}
        </div>
        <div style={styles.rowMeta}>{exposedSummary}</div>
        <div style={styles.trace} data-testid={`views-panel-trace-${view.id}`}>
          viewpoint trace pending
        </div>
      </div>
      <SourcePreviewPopover
        triggerRef={rowRef}
        triggerHovered={hovered}
        uri={previewUri}
        elementId={view.id}
        onPromote={onPromote ? handlePromote : undefined}
        testId={`views-panel-preview-${view.id}`}
      />
    </>
  );
}
