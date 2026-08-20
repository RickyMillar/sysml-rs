/**
 * DiagnosticRow — one diagnostic inside `<DiagnosticsPanel />` (R6.1).
 *
 * Row anatomy:
 *   [severity chip] [code]  message…              path · line:col
 *
 * The row itself is a `<button>` so keyboard navigation (Enter / Space)
 * activates the click handler — the panel wires that to a selection +
 * navigate side-effect that reveals the file in the model tree and
 * routes to `/run`.
 *
 * `aria-label` is built via `buildRowAriaLabel` so tests can assert the
 * shape without replicating the logic.
 */

import {
  useCallback,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from 'react';
import { SourcePreviewPopover } from '@/features/editor/SourcePreviewPopover';
import type { DiagnosticEntry } from './types';
import {
  DIAGNOSTIC_SEVERITY_COLORS,
  DIAGNOSTIC_SEVERITY_LABELS,
} from './types';
import { formatSpanLocation } from './filterDiagnostics';

export interface DiagnosticRowProps {
  entry: DiagnosticEntry;
  /** Default action when the row is clicked or Enter/Space pressed. */
  onActivate: (entry: DiagnosticEntry) => void;
  /**
   * Element id this row's diagnostic refers to, if the panel's
   * extractor was able to resolve one. `null` short-circuits the
   * hover-preview popover — no id means `sysml.get_source` can't
   * find a span. The popover still renders the hover styling
   * cleanly; just no Monaco card.
   */
  previewElementId?: string | null;
  /**
   * Called when the user clicks the hover-preview popover (not the
   * row itself). The panel hands a single side-effect that pushes
   * selection + focused URI + opens the Source drawer.
   */
  onPromotePreview?: (entry: DiagnosticEntry) => void;
}

/**
 * Strip everything up to and including the final `/` so rows surface a
 * compact filename. The full URI still lives in the row's `title` so
 * hovering reveals the path. Pure — exported for tests.
 */
export function shortFileName(uri: string): string {
  if (!uri) return '';
  const lastSlash = uri.lastIndexOf('/');
  if (lastSlash < 0) return uri;
  return uri.slice(lastSlash + 1) || uri;
}

/**
 * Build the `aria-label` text for the row button. Includes severity,
 * optional code, trimmed message, and file:line when available — the
 * screen-reader copy is the same copy the eye sees in the row.
 */
export function buildRowAriaLabel(entry: DiagnosticEntry): string {
  const { diagnostic, uri } = entry;
  const parts: string[] = [];
  parts.push(`${DIAGNOSTIC_SEVERITY_LABELS[diagnostic.severity]} diagnostic`);
  if (diagnostic.code) parts.push(`code ${diagnostic.code}`);
  parts.push(diagnostic.message);
  const loc = formatSpanLocation(diagnostic.span);
  const fileLabel = shortFileName(diagnostic.span?.file ?? uri);
  if (loc) {
    parts.push(`in ${fileLabel} at line ${loc}`);
  } else {
    parts.push(`in ${fileLabel}`);
  }
  parts.push('press Enter to open');
  return parts.join(', ');
}

export function DiagnosticRow({
  entry,
  onActivate,
  previewElementId,
  onPromotePreview,
}: DiagnosticRowProps) {
  const [hovered, setHovered] = useState(false);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const { diagnostic, uri } = entry;
  // Prefer the span's file when the diagnostic carries one — semantic
  // diagnostics can reference a file other than the parent URI.
  const previewUri = diagnostic.span?.file ?? uri ?? null;
  const handlePromote = useCallback(() => {
    onPromotePreview?.(entry);
  }, [entry, onPromotePreview]);
  const ariaLabel = useMemo(() => buildRowAriaLabel(entry), [entry]);
  const location = useMemo(
    () => formatSpanLocation(diagnostic.span),
    [diagnostic.span],
  );
  const fileLabel = shortFileName(diagnostic.span?.file ?? uri);

  const handleActivate = useCallback(() => {
    onActivate(entry);
  }, [entry, onActivate]);

  const handleKey = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        handleActivate();
      }
    },
    [handleActivate],
  );

  const severityColor = DIAGNOSTIC_SEVERITY_COLORS[diagnostic.severity];

  const rowStyle: CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    gap: 6,
    width: '100%',
    padding: '6px 10px',
    background: hovered
      ? 'var(--surface-raised)'
      : 'transparent',
    border: 'none',
    borderBottom:
      '1px solid var(--border-default)',
    color: 'var(--text-primary)',
    textAlign: 'left',
    cursor: 'pointer',
    fontFamily: 'inherit',
  };

  return (
    <>
    <button
      ref={buttonRef}
      type="button"
      aria-label={ariaLabel}
      data-testid={`diagnostic-row-${diagnostic.severity}`}
      data-severity={diagnostic.severity}
      onClick={handleActivate}
      onKeyDown={handleKey}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
      title={`${fileLabel}${location ? `:${location}` : ''}`}
      style={rowStyle}
    >
      <SeverityChip severity={diagnostic.severity} color={severityColor} />
      {diagnostic.code ? (
        <span
          data-testid="diagnostic-row-code"
          style={{
            fontSize: 10,
            letterSpacing: 0.3,
            color: 'var(--text-muted)',
            background: 'var(--surface-panel)',
            border: '1px solid var(--border-default)',
            borderRadius: 3,
            padding: '0 5px',
            whiteSpace: 'nowrap',
          }}
        >
          {diagnostic.code}
        </span>
      ) : null}
      <span
        data-testid="diagnostic-row-message"
        style={{
          flex: 1,
          fontSize: 'var(--text-xs, 11px)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {diagnostic.message}
      </span>
      <span
        data-testid="diagnostic-row-location"
        style={{
          fontSize: 10,
          color: 'var(--text-muted)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          maxWidth: 130,
        }}
      >
        {fileLabel}
        {location ? (
          <>
            <span aria-hidden="true" style={{ opacity: 0.5 }}>
              {':'}
            </span>
            {location}
          </>
        ) : null}
      </span>
    </button>
    <SourcePreviewPopover
      triggerRef={buttonRef}
      triggerHovered={hovered}
      uri={previewUri}
      elementId={previewElementId ?? null}
      onPromote={onPromotePreview ? handlePromote : undefined}
      testId={`diagnostic-preview-${diagnostic.severity}`}
    />
    </>
  );
}

function SeverityChip({
  severity,
  color,
}: {
  severity: DiagnosticEntry['diagnostic']['severity'];
  color: string;
}) {
  return (
    <span
      data-testid={`diagnostic-severity-chip-${severity}`}
      aria-hidden="true"
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        minWidth: 46,
        padding: '0 6px',
        height: 16,
        borderRadius: 8,
        background: `color-mix(in srgb, ${color} 22%, transparent)`,
        color,
        border: `1px solid ${color}`,
        fontSize: 9,
        fontWeight: 700,
        letterSpacing: 0.4,
        textTransform: 'uppercase',
      }}
    >
      {severity}
    </span>
  );
}
