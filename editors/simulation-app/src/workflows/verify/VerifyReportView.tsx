/**
 * VerifyReportView — the on-screen verification report document (Phase 4).
 *
 * The report is a real document surface, not a panel: it renders the
 * EXACT output of the pure `generateHtmlReport` generator inside a
 * sandboxed `<iframe srcDoc>`, so the on-screen view, the print output,
 * and the downloaded file are byte-identical (one source of truth). The
 * generated document carries its own print stylesheet, so Print produces
 * the calm paginated report the plan's DoD asks for.
 *
 * Provenance (§6.2) is assembled by `buildReportInput` and rendered by
 * the generator; fields the backend billet hasn't filled yet show `—`.
 */

import { useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import type { Verdict, VerifyRunResult } from '@/engine/types';
import { DownloadReportButton } from './report/DownloadReportButton';
import { generateHtmlReport } from './report/generateHtmlReport';
import { buildReportInput } from './report/buildReportInput';
import type { ReportInput } from './report/types';
import type { SessionProvenance } from '@/features/sessions/types';

export interface VerifyReportViewProps {
  result: VerifyRunResult | null;
  verdicts: Verdict[];
  suiteLabel: string;
  selectedCaseNames: string[];
  sessionId: string | null;
  /** Active session's provenance block (B6) — fills the report's model row. */
  sessionProvenance?: SessionProvenance | null;
  workspaceName: string;
}

export function VerifyReportView(props: VerifyReportViewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const getInput = (): ReportInput =>
    buildReportInput({
      workspaceName: props.workspaceName,
      result: props.result,
      verdicts: props.verdicts,
      suiteLabel: props.suiteLabel,
      selectedCaseNames: props.selectedCaseNames,
      sessionId: props.sessionId,
      sessionProvenance: props.sessionProvenance,
      runTimestamp: new Date(),
    });

  // Memoise the rendered document on the verdict inputs — a fresh
  // timestamp on every keystroke would thrash the iframe.
  const html = useMemo(
    () => generateHtmlReport(getInput()).html,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [props.result, props.verdicts, props.suiteLabel, props.sessionId, props.workspaceName],
  );

  const hasContent = props.verdicts.length > 0 || (props.result?.verdicts.length ?? 0) > 0;

  if (!hasContent) {
    return (
      <div data-testid="verify-report-empty" className="flex h-full items-center justify-center" style={{ color: 'var(--text-muted)', fontSize: 12 }}>
        Run verification to produce a report.
      </div>
    );
  }

  return (
    <div data-testid="verify-report" className="flex flex-col h-full min-h-0">
      <div className="flex items-center gap-2 px-3 shrink-0" style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-muted)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>
          Report
        </span>
        <div className="flex items-center gap-2" style={{ marginLeft: 'auto' }}>
          <button
            type="button"
            data-testid="verify-report-print"
            onClick={() => iframeRef.current?.contentWindow?.print()}
            style={actionStyle}
          >
            <span className="material-symbols-outlined" style={{ fontSize: 14 }}>print</span>
            Print
          </button>
          <DownloadReportButton getInput={getInput} testId="verify-report-download" style={actionStyle} />
        </div>
      </div>
      <iframe
        ref={iframeRef}
        data-testid="verify-report-frame"
        title="Verification report"
        srcDoc={html}
        style={{ flex: 1, width: '100%', border: 'none', background: 'var(--surface-canvas)' }}
      />
    </div>
  );
}

const actionStyle: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
  padding: '4px 10px',
  borderRadius: 4,
  border: '1px solid var(--border-hairline)',
  background: 'var(--surface-raised)',
  color: 'var(--text-primary)',
  fontSize: 11,
  fontWeight: 600,
  cursor: 'pointer',
};
