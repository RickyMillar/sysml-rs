/**
 * MonteCarloResultsPanel — wrapper consumed by Agent EE's shell.
 *
 * EE owns `MonteCarloResultsShell` (the configure/run/analyze frame).
 * This panel is the analyze-phase payload: histogram viewer +
 * pass-rate dashboard + CSV export button. Keeping it as a separate
 * component means EE can drop it into the shell verbatim without
 * touching FF's code, and FF can verify the analyze surface in
 * isolation without depending on EE's shell merge.
 */

import { useCallback } from 'react';
import type { CSSProperties } from 'react';
import {
  MonteCarloHistogramViewer,
  type MonteCarloOutcome,
} from '../../../shared/viewers/MonteCarloHistogramViewer';
import { PassRateDashboard } from './PassRateDashboard';
import { DownloadCsvButton } from './DownloadCsvButton';
import type { ChildDescriptor } from './passRateHelpers';

export interface MonteCarloResultsPanelProps {
  /** Stable id used in the CSV filename and test ids. */
  batchId: string;
  /** Child iteration records. */
  children: ChildDescriptor[];
  /** Outcome metrics to histogram. */
  outcomes: MonteCarloOutcome[];
  /** Constraint ids whose pass rate should be tracked. */
  constraintIds: string[];
  /** Optional human label map for constraints. */
  constraintLabels?: Record<string, string>;
  /** Optional default bin count (clamped 5..100). */
  defaultBinCount?: number;
  /** Fired after the CSV anchor click. */
  onCsvDownloaded?: (filename: string) => void;
  /** Optional test id override. */
  testId?: string;
  /** Optional outer style override. */
  style?: CSSProperties;
}

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 20,
  color: 'var(--on-surface)',
};

const HEADER_STYLE: CSSProperties = {
  display: 'flex',
  justifyContent: 'space-between',
  alignItems: 'center',
  gap: 12,
};

const SECTION_LABEL_STYLE: CSSProperties = {
  fontSize: 11,
  opacity: 0.8,
  letterSpacing: 0.4,
  textTransform: 'uppercase',
  marginBottom: 8,
};

export function MonteCarloResultsPanel(props: MonteCarloResultsPanelProps) {
  const {
    batchId,
    children: childList,
    outcomes,
    constraintIds,
    constraintLabels,
    defaultBinCount,
    onCsvDownloaded,
    testId,
    style,
  } = props;

  const getChildren = useCallback(() => childList, [childList]);

  return (
    <div
      style={{ ...ROOT_STYLE, ...style }}
      data-testid={testId ?? 'monte-carlo-results-panel'}
    >
      <header style={HEADER_STYLE}>
        <div>
          <div style={SECTION_LABEL_STYLE}>Batch</div>
          <div style={{ fontSize: 13, fontFamily: 'ui-monospace, "JetBrains Mono", monospace' }}>
            {batchId}
          </div>
        </div>
        <DownloadCsvButton
          batchId={batchId}
          getChildren={getChildren}
          onDownloaded={onCsvDownloaded}
          testId="monte-carlo-download-csv"
        />
      </header>
      <section>
        <div style={SECTION_LABEL_STYLE}>Pass rate</div>
        <PassRateDashboard
          children={childList}
          constraints={constraintIds}
          labels={constraintLabels}
          testId="monte-carlo-pass-rate"
        />
      </section>
      <section>
        <div style={SECTION_LABEL_STYLE}>Outcomes</div>
        <MonteCarloHistogramViewer
          children={childList}
          outcomes={outcomes}
          defaultBinCount={defaultBinCount}
          testId="monte-carlo-histograms"
        />
      </section>
    </div>
  );
}
