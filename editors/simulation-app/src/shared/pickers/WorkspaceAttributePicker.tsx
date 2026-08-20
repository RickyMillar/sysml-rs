/**
 * WorkspaceAttributePicker — shared multi-select list for Sweep /
 * Monte Carlo / Trade Study / Sensitivity.
 *
 * analyze workflows need the same "pick attributes from the workspace"
 * surface — today they each render a slightly different checkbox list,
 * click-to-add pill bar, or `<select>`. This component is the canonical
 * checkbox list extracted from `MonteCarloConfig.tsx`, generalised over
 * `MetricDescriptor` and given an optional `renderExpanded` slot so each
 * workflow can drop in its own per-row editor (distribution,
 * min/max/grid, weight, sampling-range).
 *
 * Deliberately dumb: owns only the filter text. Selection state lives
 * with the consumer.
 *
 * Empty / loading / no-workspace states are first-class so each consumer
 * doesn't reinvent them. Labels are customisable via `messages`.
 */

import { useMemo, useState, type ReactNode } from 'react';
import type { MetricDescriptor } from '../metrics/types';

export interface WorkspaceAttributePickerMessages {
  /** Placeholder in the filter input. */
  searchPlaceholder?: string;
  /** Title when `hasWorkspace === false`. */
  noWorkspaceTitle?: string;
  /** Hint when `hasWorkspace === false`. */
  noWorkspaceHint?: string;
  /** Title when `isLoading === true`. */
  loadingTitle?: string;
  /** Hint shown while loading. */
  loadingHint?: string;
  /** Title when `candidates.length === 0`. */
  emptyTitle?: string;
  /** Hint when candidate list is empty. */
  emptyHint?: string;
  /** Title when the filter excludes every candidate. */
  filteredOutTitle?: string;
  /** Hint shown when the filter excludes every candidate. */
  filteredOutHint?: string;
}

export interface WorkspaceAttributePickerProps {
  /**
   * Candidates surfaced from the workspace (AttributeUsage, expressions,
   * constraints, …). Order is preserved; consumers should pre-sort.
   */
  candidates: readonly MetricDescriptor[];
  /** Ids of candidates currently selected. */
  selected: readonly string[];
  /** Called with the candidate's id when its checkbox is toggled. */
  onToggle: (id: string) => void;
  /**
   * Render a per-row editor below a checked candidate (distribution,
   * min/max, weight, sampling range). Returning `null` keeps the row
   * checkbox-only.
   */
  renderExpanded?: (id: string) => ReactNode;
  /** Is the workspace itself loaded? Gates the empty/loading states. */
  hasWorkspace?: boolean;
  /** Are candidates still being discovered? Shows a loading row. */
  isLoading?: boolean;
  /** Max pixel height of the scrollable candidate list. */
  maxListHeight?: number;
  /**
   * Override any of the copy used in the filter placeholder or empty
   * states. Individual fields are optional.
   */
  messages?: WorkspaceAttributePickerMessages;
  /**
   * data-testid prefix used by the component. Defaults to
   * `workspace-attribute-picker`; each consumer passes a per-tool prefix
   * so test ids read e.g. `montecarlo-picker-row-<id>`.
   */
  testIdPrefix?: string;
}

const DEFAULT_MESSAGES: Required<WorkspaceAttributePickerMessages> = {
  searchPlaceholder: 'Filter attributes…',
  noWorkspaceTitle: 'No workspace loaded',
  noWorkspaceHint: 'Load a workspace to list attributes.',
  loadingTitle: 'Scanning attributes…',
  loadingHint: 'Reading the model for attribute usages.',
  emptyTitle: 'No attributes found',
  emptyHint: 'Add an AttributeUsage (e.g. `attribute voltage = 12;`) to the model.',
  filteredOutTitle: 'No matches',
  filteredOutHint: 'Clear the filter to see all attributes.',
};

export function WorkspaceAttributePicker({
  candidates,
  selected,
  onToggle,
  renderExpanded,
  hasWorkspace = true,
  isLoading = false,
  maxListHeight = 240,
  messages,
  testIdPrefix = 'workspace-attribute-picker',
}: WorkspaceAttributePickerProps) {
  const msg = { ...DEFAULT_MESSAGES, ...messages };
  const [filter, setFilter] = useState('');

  const selectedSet = useMemo(() => new Set(selected), [selected]);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return candidates;
    return candidates.filter((c) => {
      if (c.name.toLowerCase().includes(q)) return true;
      if (c.id.toLowerCase().includes(q)) return true;
      if (c.domain && c.domain.toLowerCase().includes(q)) return true;
      return false;
    });
  }, [candidates, filter]);

  return (
    <div
      data-testid={testIdPrefix}
      data-has-workspace={hasWorkspace}
      data-candidate-count={candidates.length}
      data-selected-count={selected.length}
      className="flex flex-col"
    >
      {/* Filter input. Hidden when the list would be empty anyway. */}
      {hasWorkspace && !isLoading && candidates.length > 0 && (
        <div className="px-3 pb-2">
          <input
            type="text"
            data-testid={`${testIdPrefix}-filter`}
            placeholder={msg.searchPlaceholder}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{
              width: '100%',
              height: 26,
              padding: '0 8px',
              background: 'var(--surface-container)',
              color: 'var(--on-surface)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 4,
              fontSize: 11,
            }}
          />
        </div>
      )}

      <div
        data-testid={`${testIdPrefix}-list`}
        style={{
          maxHeight: maxListHeight,
          overflowY: 'auto',
          borderTop: '1px solid var(--outline-variant)',
        }}
      >
        {!hasWorkspace ? (
          <EmptyRow
            icon="folder_open"
            title={msg.noWorkspaceTitle}
            hint={msg.noWorkspaceHint}
            testId={`${testIdPrefix}-no-workspace`}
          />
        ) : isLoading ? (
          <EmptyRow
            icon="progress_activity"
            title={msg.loadingTitle}
            hint={msg.loadingHint}
            spinning
            testId={`${testIdPrefix}-loading`}
          />
        ) : candidates.length === 0 ? (
          <EmptyRow
            icon="search_off"
            title={msg.emptyTitle}
            hint={msg.emptyHint}
            testId={`${testIdPrefix}-empty`}
          />
        ) : visible.length === 0 ? (
          <EmptyRow
            icon="filter_alt_off"
            title={msg.filteredOutTitle}
            hint={msg.filteredOutHint}
            testId={`${testIdPrefix}-filtered-empty`}
          />
        ) : (
          <ul style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
            {visible.map((c) => {
              const checked = selectedSet.has(c.id);
              const expanded = checked && renderExpanded ? renderExpanded(c.id) : null;
              return (
                <li key={c.id}>
                  <CandidateRow
                    candidate={c}
                    checked={checked}
                    onToggle={() => onToggle(c.id)}
                    testIdPrefix={testIdPrefix}
                  />
                  {expanded != null && (
                    <div
                      data-testid={`${testIdPrefix}-expanded-${c.id}`}
                      style={{
                        padding: '4px 12px 8px 32px',
                        borderBottom: '1px dashed var(--outline-variant)',
                      }}
                    >
                      {expanded}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

function CandidateRow({
  candidate,
  checked,
  onToggle,
  testIdPrefix,
}: {
  candidate: MetricDescriptor;
  checked: boolean;
  onToggle: () => void;
  testIdPrefix: string;
}) {
  const subtitle = formatSubtitle(candidate);
  return (
    <label
      data-testid={`${testIdPrefix}-row-${candidate.id}`}
      data-checked={checked}
      data-source={candidate.source}
      className="flex items-center gap-2 px-3 py-1.5"
      style={{
        cursor: 'pointer',
        fontSize: 12,
        color: 'var(--on-surface)',
      }}
    >
      <input
        type="checkbox"
        checked={checked}
        onChange={onToggle}
        data-testid={`${testIdPrefix}-checkbox-${candidate.id}`}
        style={{ cursor: 'pointer' }}
      />
      <div className="flex-1 min-w-0">
        <div className="truncate mono-text" style={{ fontSize: 12 }}>
          {candidate.name}
        </div>
        {subtitle && (
          <div
            className="truncate"
            style={{ fontSize: 10, color: 'var(--outline)' }}
            data-testid={`${testIdPrefix}-subtitle-${candidate.id}`}
          >
            {subtitle}
          </div>
        )}
      </div>
      {candidate.unit && (
        <span
          className="mono-text"
          style={{
            fontSize: 10,
            color: 'var(--outline)',
            background: 'var(--surface-container)',
            padding: '1px 6px',
            borderRadius: 3,
          }}
          data-testid={`${testIdPrefix}-unit-${candidate.id}`}
        >
          {candidate.unit}
        </span>
      )}
    </label>
  );
}

function formatSubtitle(candidate: MetricDescriptor): string | null {
  const parts: string[] = [];
  if (candidate.domain) parts.push(candidate.domain);
  if (candidate.source !== 'variable') parts.push(candidate.source);
  return parts.length ? parts.join(' · ') : null;
}

function EmptyRow({
  icon,
  title,
  hint,
  testId,
  spinning = false,
}: {
  icon: string;
  title: string;
  hint: string;
  testId: string;
  spinning?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center gap-2 px-4 py-6"
      style={{ color: 'var(--outline)' }}
    >
      <span
        className="material-symbols-outlined"
        style={{
          fontSize: 24,
          opacity: 0.8,
          animation: spinning ? 'spin 1s linear infinite' : undefined,
        }}
      >
        {icon}
      </span>
      <span style={{ fontSize: 12, fontWeight: 500 }}>{title}</span>
      <span style={{ fontSize: 11, maxWidth: 260, textAlign: 'center' }}>{hint}</span>
    </div>
  );
}
