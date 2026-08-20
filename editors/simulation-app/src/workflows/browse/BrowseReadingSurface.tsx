/**
 * BrowseReadingSurface — the primary reading surface for Browse
 * (ninebar Phase 1.5), the "Source" side of the segmented control in
 * `BrowseWorkflow`.
 *
 * Loads the FULL file for the selected element and mounts a read-only
 * `MonacoSysmlEditor` with the `/lsp` client attached (`lspUri`), then
 * reveals the element's line/col. Highlighting in this editor comes
 * exclusively from the LSP semantic-tokens provider — there is no
 * Monarch fallback (`sysmlLanguage.ts` removed it on purpose so backend
 * gaps surface as the magenta hard-fail colour). So the reading floor
 * has to attach the LSP to get any colour at all — a slice with no LSP
 * renders 100% magenta.
 *
 * Why the full file, not `sysml.get_source`'s slice: the LSP `didOpen`
 * sends the editor buffer as the document body. A bare span slice
 * (`in V_net : Real;`) doesn't parse standalone AND would overwrite the
 * server's shared copy of the file. We reveal the element inside the
 * whole file instead — same primitive `SourcePanel` uses for editing,
 * minus the dirty-buffer / cursor-link machinery (this is a reading
 * floor, not an editor, so it stays `readOnly` and drops `onChange`).
 *
 * `get_source` is still called — but only for the element's `line`/`col`
 * so we can scroll the full buffer to it (and to detect the no-span
 * synthesised/stdlib case). The full text comes from `loadFile`.
 *
 * Quiet centered empty state when nothing is selected. Reads only
 * `useSelectionStore` + `sysml.get_source` + `loadFile` (`/files`) —
 * none touches a session, so this surface works with zero sessions. The
 * Monaco `key` folds in `reloadEpoch` so a same-root reload remounts the
 * editor (LSP didClose/didOpen) against fresh backend text.
 */
import type { ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useSelectionStore } from '@/features/selection/store';
import { useGetSource } from '@/features/editor/useGetSource';
import { MonacoSysmlEditor } from '@/features/editor/MonacoSysmlEditor';
import { loadFile } from '@/shared/api/model';
import { useWorkspaceStore } from '@/store/workspace';

function CenteredHint({ testId, icon, children }: { testId: string; icon: string; children: ReactNode }) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center h-full w-full gap-2"
      style={{ color: 'var(--text-secondary)' }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 28, opacity: 0.6 }}>
        {icon}
      </span>
      <span style={{ fontSize: 'var(--text-sm)' }}>{children}</span>
    </div>
  );
}

export function BrowseReadingSurface() {
  const selectedUri = useSelectionStore((s) => s.selectedUri);
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const reloadEpoch = useWorkspaceStore((s) => s.reloadEpoch);

  // `get_source` gives us the element's line/col (for reveal) and, when
  // it returns null, tells us the element has no source span (synthesised
  // or stdlib) — the one case where we don't load a file at all.
  const spanQuery = useGetSource(selectedUri, selectedElementId);
  const hasSpan = spanQuery.data != null;

  // Full file text — the document the LSP tokenises. Gated on a real span
  // so we never try to `loadFile` a synthesised/stdlib element. Keyed on
  // reloadEpoch so a same-root reload re-fetches fresh backend text.
  const fileQuery = useQuery({
    queryKey: ['browse-reading-file', selectedUri, reloadEpoch],
    queryFn: () => (selectedUri ? loadFile(selectedUri) : Promise.resolve(null)),
    enabled: !!selectedUri && !!selectedElementId && hasSpan,
    staleTime: 5_000,
  });

  if (!selectedUri || !selectedElementId) {
    return (
      <CenteredHint testId="browse-reading-empty" icon="menu_book">
        Select an element to read its source.
      </CenteredHint>
    );
  }

  if (spanQuery.isLoading) {
    return (
      <CenteredHint testId="browse-reading-loading" icon="hourglass_empty">
        Loading source…
      </CenteredHint>
    );
  }

  if (spanQuery.isError) {
    return (
      <CenteredHint testId="browse-reading-error" icon="error">
        Failed to load source.
      </CenteredHint>
    );
  }

  const span = spanQuery.data;
  if (!span) {
    return (
      <CenteredHint testId="browse-reading-no-span" icon="visibility_off">
        No source span — synthesised or stdlib element.
      </CenteredHint>
    );
  }

  if (fileQuery.isError) {
    return (
      <CenteredHint testId="browse-reading-error" icon="error">
        Failed to load source.
      </CenteredHint>
    );
  }

  const fileResult = fileQuery.data;
  if (fileQuery.isLoading || !fileResult) {
    return (
      <CenteredHint testId="browse-reading-loading" icon="hourglass_empty">
        Loading source…
      </CenteredHint>
    );
  }

  return (
    <div data-testid="browse-reading-surface" className="h-full w-full">
      <MonacoSysmlEditor
        key={`${selectedUri}#${reloadEpoch}`}
        value={fileResult.source}
        readOnly
        lspUri={selectedUri}
        height="100%"
        revealLineCol={typeof span.line === 'number' ? { line: span.line, col: span.col } : undefined}
        testId="browse-reading-editor"
      />
    </div>
  );
}
