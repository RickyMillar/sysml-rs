/**
 * ViewpointPicker — typeahead search over `ViewpointDefinition` /
 * `ViewpointUsage` elements.
 *
 * Selecting a viewpoint narrows the surrounding ViewsPanel via
 * `Filter::View { viewpoint_id }`. An empty selection means "all
 * authored views".
 *
 * The text input is debounced by 250ms before triggering a
 * `sysml.query` request. The picker itself does not hold the selection
 * — that lives on the parent so URL/state sync stays in one place.
 */

import { useState } from 'react';
import { useDebouncedValue } from '@/hooks/useDebouncedValue';
import { useViewpointSearch, type ViewpointPickerEntry } from './queries';

export const VIEWPOINT_PICKER_DEBOUNCE_MS = 250;

const styles = {
  root: {
    display: 'flex',
    flexDirection: 'column' as const,
    gap: 4,
  },
  bar: {
    display: 'flex',
    alignItems: 'center',
    gap: 6,
  },
  input: {
    flex: 1,
    padding: '4px 8px',
    fontSize: 12,
    borderRadius: 4,
    border: '1px solid var(--border-default)',
    background: 'var(--surface-sunken)',
    color: 'var(--text-primary)',
    outline: 'none',
  },
  clearBtn: {
    padding: '2px 8px',
    fontSize: 11,
    background: 'transparent',
    color: 'var(--text-muted)',
    border: '1px solid var(--border-default)',
    borderRadius: 4,
    cursor: 'pointer',
  },
  selectedChip: {
    fontSize: 11,
    color: 'var(--text-primary)',
    padding: '2px 6px',
    background: 'var(--accent-tint)',
    border: '1px solid var(--accent)',
    borderRadius: 4,
    whiteSpace: 'nowrap' as const,
    overflow: 'hidden' as const,
    textOverflow: 'ellipsis' as const,
    maxWidth: 180,
  },
  list: {
    maxHeight: 200,
    overflowY: 'auto' as const,
    border: '1px solid var(--border-default)',
    borderRadius: 4,
    background: 'var(--surface-panel)',
  },
  row: {
    padding: '4px 8px',
    fontSize: 12,
    color: 'var(--text-primary)',
    cursor: 'pointer',
    borderBottom: '1px solid var(--border-default)',
  },
  rowLast: {
    borderBottom: 'none',
  },
  rowKind: {
    color: 'var(--text-muted)',
    marginRight: 4,
  },
  hint: {
    padding: '4px 8px',
    fontSize: 11,
    color: 'var(--text-muted)',
  },
};

function entryLabel(entry: ViewpointPickerEntry): string {
  return entry.name ?? entry.qualified_name ?? entry.id;
}

function kindLabel(kind: string): string {
  if (kind === 'ViewpointDefinition') return 'viewpoint def';
  if (kind === 'ViewpointUsage') return 'viewpoint';
  return kind;
}

export interface ViewpointPickerProps {
  /** URI to query against. */
  uri: string | null;
  /** Currently selected viewpoint id (controlled). */
  selectedId: string | null;
  /** Currently selected viewpoint label, shown when collapsed. */
  selectedLabel: string | null;
  /** Called when the user picks a viewpoint. */
  onSelect: (entry: ViewpointPickerEntry) => void;
  /** Called when the user clears the current selection. */
  onClear: () => void;
}

export function ViewpointPicker({
  uri,
  selectedId,
  selectedLabel,
  onSelect,
  onClear,
}: ViewpointPickerProps) {
  const [text, setText] = useState('');
  const [open, setOpen] = useState(false);
  const debounced = useDebouncedValue(text, VIEWPOINT_PICKER_DEBOUNCE_MS);
  const search = useViewpointSearch(open ? uri : null, debounced.trim());

  const showList = open && uri;
  const rows = search.data ?? [];

  return (
    <div style={styles.root} data-testid="viewpoint-picker">
      <div style={styles.bar}>
        <input
          type="text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => setOpen(true)}
          placeholder={
            selectedId
              ? 'Search viewpoints…'
              : 'Filter by viewpoint…'
          }
          style={styles.input}
          data-testid="viewpoint-picker-input"
          aria-label="Filter views by viewpoint"
        />
        {selectedId && (
          <span
            style={styles.selectedChip}
            data-testid="viewpoint-picker-selected"
            title={selectedLabel ?? selectedId}
          >
            {selectedLabel ?? selectedId}
          </span>
        )}
        {selectedId && (
          <button
            type="button"
            style={styles.clearBtn}
            onClick={() => {
              setText('');
              onClear();
            }}
            data-testid="viewpoint-picker-clear"
          >
            Clear
          </button>
        )}
      </div>
      {showList && (
        <div style={styles.list} data-testid="viewpoint-picker-list">
          {search.isLoading && (
            <div style={styles.hint}>Loading…</div>
          )}
          {!search.isLoading && rows.length === 0 && (
            <div style={styles.hint}>
              {debounced.trim().length === 0
                ? 'No viewpoints in this scope.'
                : 'No matches.'}
            </div>
          )}
          {rows.map((entry, idx) => (
            <div
              key={entry.id}
              style={{
                ...styles.row,
                ...(idx === rows.length - 1 ? styles.rowLast : {}),
              }}
              role="button"
              tabIndex={0}
              data-testid={`viewpoint-picker-row-${entry.id}`}
              onClick={() => {
                onSelect(entry);
                setText('');
                setOpen(false);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelect(entry);
                  setText('');
                  setOpen(false);
                }
              }}
            >
              <span style={styles.rowKind}>{kindLabel(entry.kind)}</span>
              {entryLabel(entry)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
