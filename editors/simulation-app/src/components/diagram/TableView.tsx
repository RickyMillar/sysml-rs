import { useMemo, useState } from 'react';
import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type ColumnDef,
  type SortingState,
} from '@tanstack/react-table';
import { useWorkspaceStore } from '@/store/workspace';
import type { TableModel, TableRow as TableRowModel } from '@/shared/api/model';

/**
 * Legend entry the backend attaches to a `TableModel` (`legend` on the wire,
 * mirroring `tmodel::TableLegendEntry`). Typed locally until the shared
 * `TableModel` mirror in `@/shared/api/model` picks the field up.
 */
interface TableLegendEntry {
  symbol: string;
  label: string;
}

type TableModelWithLegend = TableModel & { legend?: TableLegendEntry[] };

/**
 * Tabular view of the model. Rendered when the store holds a `tableModel`
 * payload (DiagramHost dispatches by payload shape).
 *
 * Reads the typed `TableModel` payload that the backend produces for
 * `view=grid` (a traceability matrix today: requirements × satisfy/verify
 * targets, with S/V/A/D/T cell symbols). Future table flavours (element
 * catalogs, attribute tables, …) will land in the same slot once the
 * backend grows additional generators.
 */
export function TableView() {
  const tableModel = useWorkspaceStore((s) => s.tableModel) as TableModelWithLegend | null;
  const legend = tableModel?.legend ?? [];

  const data = useMemo<TableRowModel[]>(() => tableModel?.rows ?? [], [tableModel]);

  const columns = useMemo<ColumnDef<TableRowModel>[]>(() => {
    const defs = tableModel?.columns ?? [];
    return defs.map((col, index) => ({
      id: col.id,
      header: col.label,
      // Sort by displayed text so the user sees rows ordered by what they read.
      accessorFn: (row) => row.cells[index]?.display ?? '',
      cell: ({ row }) => {
        const cell = row.original.cells[index];
        if (!cell) return null;
        return (
          <span
            className={cell.cssClasses?.join(' ')}
            title={cell.elementId ?? undefined}
          >
            {cell.display}
          </span>
        );
      },
    }));
  }, [tableModel]);

  const [sorting, setSorting] = useState<SortingState>([]);

  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const isEmpty = !tableModel || data.length === 0;
  const isMatrix =
    tableModel?.kind === 'traceability_matrix' ||
    (tableModel?.title ?? '').toLowerCase().includes('matrix');
  // A traceability matrix with only the leading "Requirement" column carries no
  // satisfy/verify/derive/allocate targets — it's a requirement list, not a 2D
  // matrix. Rather than present a lone column as if it were a matrix, say so.
  const isCoverageless = !isEmpty && isMatrix && (tableModel?.columns?.length ?? 0) <= 1;

  return (
    <div
      data-testid="table-view-root"
      style={{
        width: '100%',
        height: '100%',
        overflow: 'auto',
        background: 'var(--surface-dim)',
      }}
    >
      {tableModel?.title && (
        <div
          data-testid="table-view-title"
          style={{
            padding: '12px 16px 8px',
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--on-surface)',
            borderBottom: '1px solid var(--outline-variant)',
          }}
        >
          {tableModel.title}
        </div>
      )}

      {legend.length > 0 && !isEmpty && (
        <div
          data-testid="table-view-legend"
          style={{
            padding: '6px 16px',
            fontSize: 11,
            color: 'var(--outline)',
            borderBottom: '1px solid var(--outline-variant)',
            display: 'flex',
            flexWrap: 'wrap',
            gap: '4px 16px',
          }}
        >
          {legend.map((entry) => (
            <span key={entry.symbol}>
              <span
                className={`cell-${entry.symbol.toLowerCase()}`}
                style={{ fontWeight: 600, color: 'var(--on-surface)' }}
              >
                {entry.symbol}
              </span>
              {' = '}
              {entry.label}
            </span>
          ))}
        </div>
      )}

      {isCoverageless && (
        <div
          data-testid="table-view-no-coverage"
          style={{
            padding: '8px 16px',
            fontSize: 11,
            color: 'var(--outline)',
            borderBottom: '1px solid var(--outline-variant)',
          }}
        >
          No traceability relationships (satisfy / verify / derive / allocate) connect these{' '}
          {data.length} requirement{data.length === 1 ? '' : 's'} yet — showing them with no
          coverage. Add such relationships to populate the matrix columns.
        </div>
      )}

      {isEmpty ? (
        <div
          data-testid="table-view-empty"
          style={{
            padding: 24,
            color: 'var(--outline)',
            fontSize: 12,
          }}
        >
          {!tableModel ? (
            'No tabular data for this view yet.'
          ) : isMatrix ? (
            <>
              <div style={{ fontWeight: 600, color: 'var(--on-surface)', marginBottom: 6 }}>
                No requirements in the exposed scope.
              </div>
              <div style={{ maxWidth: 520, lineHeight: 1.5 }}>
                A traceability matrix plots requirements as rows and the elements that
                satisfy / verify / derive from them as columns. Rows are drawn only from
                requirement definitions and usages inside this view&rsquo;s{' '}
                <code>expose</code> subtree (the standard library is always excluded), and
                that subtree contains none — so there is nothing to plot. Expose a package
                or element that owns requirements to populate the rows.
              </div>
            </>
          ) : (
            'This table generator produced no rows for the exposed scope.'
          )}
        </div>
      ) : (
        <table
          style={{
            // Size to content: a near-empty matrix must not stretch its lone
            // columns across the viewport. The auto table layout grows with
            // content; the scroll container above handles genuinely wide
            // tables (cells cap at maxWidth below).
            width: 'auto',
            borderCollapse: 'collapse',
            fontSize: 12,
            color: 'var(--on-surface)',
          }}
        >
          <thead>
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id} style={{ background: 'var(--surface-container-high)' }}>
                {hg.headers.map((h) => {
                  const sortDir = h.column.getIsSorted();
                  return (
                    <th
                      key={h.id}
                      onClick={h.column.getToggleSortingHandler()}
                      style={{
                        textAlign: 'left',
                        padding: '8px 12px',
                        borderBottom: '1px solid var(--outline-variant)',
                        cursor: 'pointer',
                        userSelect: 'none',
                        fontWeight: 600,
                        whiteSpace: 'nowrap',
                        maxWidth: 280,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      }}
                    >
                      {flexRender(h.column.columnDef.header, h.getContext())}
                      {sortDir === 'asc' ? ' ▲' : sortDir === 'desc' ? ' ▼' : ''}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td
                    key={cell.id}
                    style={{
                      padding: '6px 12px',
                      borderBottom: '1px solid var(--outline-variant)',
                      maxWidth: 280,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
