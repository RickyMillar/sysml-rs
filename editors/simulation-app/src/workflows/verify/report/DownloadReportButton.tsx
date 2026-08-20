/**
 * DownloadReportButton — React trigger that runs the pure-function
 * report generator and offers the resulting HTML as a download.
 *
 * Intentionally thin; all logic lives in `generateHtmlReport`. The
 * button is purely a UI glue layer (`URL.createObjectURL` +
 * programmatic `<a download>` click).
 *
 * Tests for the component itself are intentionally light because the
 * heavy lifting is tested at the generator level. This file has no
 * snapshot tests — the surface area is a `<button>` that fires a
 * one-shot download side-effect.
 */

import { useCallback } from 'react';
import type { CSSProperties } from 'react';
import { generateHtmlReport } from './generateHtmlReport';
import type { ReportInput } from './types';

export interface DownloadReportButtonProps {
  /** Callback producing the input when the user clicks — deferred so
   * upstream callers don't have to compose a fresh `Verdict[]` on every
   * render (the list can be large). */
  getInput: () => ReportInput;
  /** Optional override for the button label. */
  label?: string;
  /** Optional className hook for the host's styling. */
  className?: string;
  /** Optional style override; defaults to the engineering-atelier pill. */
  style?: CSSProperties;
  /** Test id passthrough. */
  testId?: string;
  /** Called after the download anchor has been clicked (useful for
   *  analytics, telemetry, or disabling the button briefly). */
  onDownloaded?: (filename: string) => void;
}

const DEFAULT_STYLE: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 6,
  padding: '6px 12px',
  borderRadius: 6,
  border: '1px solid var(--outline-variant, #454653)',
  background: 'var(--surface-container, #19202b)',
  color: 'var(--on-surface, #dde2f2)',
  fontSize: 12,
  fontFamily: 'inherit',
  cursor: 'pointer',
};

export function DownloadReportButton(props: DownloadReportButtonProps) {
  const { getInput, label = 'Download report', className, style, testId, onDownloaded } = props;

  const handleClick = useCallback(() => {
    const input = getInput();
    const { html, filename } = generateHtmlReport(input);
    // Guard non-DOM contexts (SSR, tests without jsdom).
    if (typeof document === 'undefined' || typeof URL === 'undefined' || !URL.createObjectURL) {
      return;
    }
    const blob = new Blob([html], { type: 'text/html;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    // Give the browser a tick to start the download before we revoke.
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    onDownloaded?.(filename);
  }, [getInput, onDownloaded]);

  return (
    <button
      type="button"
      className={className}
      style={style ?? DEFAULT_STYLE}
      onClick={handleClick}
      data-testid={testId ?? 'download-report-button'}
    >
      {label}
    </button>
  );
}
