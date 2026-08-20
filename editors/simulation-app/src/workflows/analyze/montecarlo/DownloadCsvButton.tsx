/**
 * DownloadCsvButton — R5.9 glue layer.
 *
 * Thin React wrapper around `exportMonteCarloCsv`. The heavy lifting
 * (row ordering, escaping, column collection) is all done by the pure
 * helper — this component only handles the blob/anchor dance.
 *
 * Pattern borrowed from `workflows/verify/report/DownloadReportButton`
 * (R3.7) so the engineering-atelier styling stays consistent across the
 * app.
 */

import { useCallback } from 'react';
import type { CSSProperties } from 'react';
import { exportMonteCarloCsv, monteCarloCsvFilename } from './exportMonteCarloCsv';
import type { ChildDescriptor } from './passRateHelpers';

export interface DownloadCsvButtonProps {
  /**
   * Deferred so the parent doesn't have to rebuild the descriptor list
   * on every render; cheap re-renders are common on streaming panels.
   */
  getChildren: () => ChildDescriptor[];
  /** Stable batch id embedded in the filename. */
  batchId: string;
  /** Button label override (default: "Export CSV"). */
  label?: string;
  /** Optional className hook for the host's styling. */
  className?: string;
  /** Optional style override; defaults to the engineering-atelier pill. */
  style?: CSSProperties;
  /** Test id passthrough. */
  testId?: string;
  /** Fired after the anchor is clicked — useful for analytics. */
  onDownloaded?: (filename: string) => void;
  /** Disable the button (e.g. while the batch is still pending). */
  disabled?: boolean;
  /** Injectable clock for deterministic filenames in tests. */
  clock?: () => Date;
}

const DEFAULT_STYLE: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 6,
  padding: '6px 12px',
  borderRadius: 6,
  border: '1px solid var(--border-default)',
  background: 'var(--surface-panel)',
  color: 'var(--text-primary)',
  fontSize: 12,
  fontFamily: 'inherit',
  cursor: 'pointer',
};

export function DownloadCsvButton(props: DownloadCsvButtonProps) {
  const {
    getChildren,
    batchId,
    label = 'Export CSV',
    className,
    style,
    testId,
    onDownloaded,
    disabled,
    clock,
  } = props;

  const handleClick = useCallback(() => {
    const children = getChildren();
    const csv = exportMonteCarloCsv(children);
    const filename = monteCarloCsvFilename(batchId, clock ? clock() : new Date());
    // SSR / non-DOM guard — matches DownloadReportButton's treatment.
    if (
      typeof document === 'undefined' ||
      typeof URL === 'undefined' ||
      !URL.createObjectURL
    ) {
      return;
    }
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    // Give the browser a tick before revoking — matches the other
    // download buttons' debounce.
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    onDownloaded?.(filename);
  }, [getChildren, batchId, onDownloaded, clock]);

  const finalStyle: CSSProperties = disabled
    ? { ...(style ?? DEFAULT_STYLE), opacity: 0.5, cursor: 'not-allowed' }
    : style ?? DEFAULT_STYLE;

  return (
    <button
      type="button"
      className={className}
      style={finalStyle}
      onClick={handleClick}
      data-testid={testId ?? 'download-csv-button'}
      disabled={disabled}
    >
      {label}
    </button>
  );
}
