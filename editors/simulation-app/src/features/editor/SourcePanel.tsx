/**
 * Source utility panel — live Monaco editor over the focused file.
 *
 * Phase 1 (commits 9aef3948 + Phase 1b): the LSP `/lsp` WebSocket now
 * shares the host's `SysmlService`, so every `did_change` lands in the
 * salsa store the REST `sysml.*` commands read from. That means the FE
 * doesn't need to dual-write — typing here drives diagnostics, hover,
 * completion, and goto-def via LSP while the REST-driven panels
 * (tree, find, capabilities, …) see the same edits without separate
 * `sysml.load_source` POSTs.
 *
 * Selection (tree or diagram click) drives a reveal cursor via the
 * cached `sysml.get_source(uri, id)` query — we only need the
 * `line` / `col` so the same query the sneak-peek pre-warms gets
 * reused. The full buffer content stays whatever's loaded in the
 * workspace store, not the slice the legacy panel used to render.
 *
 * Sneak-peek (`SneakPeek.tsx`) stays slice-based + read-only; no LSP
 * is attached there.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { MonacoSysmlEditor } from './MonacoSysmlEditor';
import { useGetSource } from './useGetSource';
import { useSelectionStore } from '@/features/selection/store';
import { useWorkspaceStore } from '@/store/workspace';
import { loadFile } from '@/shared/api/model';
import { elementAtOffset, useDiagramLinkStore } from '@/features/diagram-link/store';

export function SourcePanel() {
  const focusedUri = useWorkspaceStore((s) => s.focusedUri);
  const file = useWorkspaceStore((s) => (focusedUri ? s.loadedFiles.get(focusedUri) : undefined));
  const updateSource = useWorkspaceStore((s) => s.updateSource);
  const seedSource = useWorkspaceStore((s) => s.seedSource);
  // Same-root reload bumps this (see the store doc): the buffer, the
  // file-source cache, and the Monaco instance below all key on it so
  // a reload re-seeds from fresh backend text instead of the
  // pre-reload cache — and the Monaco remount cycles the LSP client
  // (didClose/didOpen), re-syncing the server-side editor overlay.
  const reloadEpoch = useWorkspaceStore((s) => s.reloadEpoch);

  // Bug A fix: workspace hydrate only pulls tree + stats, leaving
  // `file.source` empty. Lazy-fetch the actual file text the first time
  // a buffer is needed, then write it back into the store via
  // `seedSource` (no `dirty: true`, no LSP didChange). We use react-
  // query so re-opens the Source drawer don't re-hit /files.
  const needsSeed = !!file && file.source === '';
  const sourceQuery = useQuery({
    queryKey: ['file-source', focusedUri, reloadEpoch],
    queryFn: () => {
      if (!focusedUri) return Promise.resolve(null);
      return loadFile(focusedUri);
    },
    enabled: !!focusedUri && needsSeed,
    staleTime: 60_000,
  });

  useEffect(() => {
    if (focusedUri && sourceQuery.data && needsSeed) {
      seedSource(focusedUri, sourceQuery.data.source);
    }
  }, [focusedUri, needsSeed, seedSource, sourceQuery.data]);

  const selectedUri = useSelectionStore((s) => s.selectedUri);
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const selectionOrigin = useSelectionStore((s) => s.selectionOrigin);
  const select = useSelectionStore((s) => s.select);

  // Only reveal when the selection's URI matches the focused file —
  // otherwise we'd jump the cursor on every cross-file click.
  const revealQuery = useGetSource(
    selectedUri === focusedUri ? selectedUri : null,
    selectedUri === focusedUri ? selectedElementId : null,
  );

  // Memo guards against React re-running the reveal effect when the
  // query result is referentially identical but the wrapper object is
  // recreated by tanstack-query. We SUPPRESS reveal when the selection
  // originated from the editor cursor — otherwise cursor→select→reveal
  // would snap the cursor and oscillate (Bucket 2.3 loop guard).
  const revealLineCol = useMemo(() => {
    if (selectionOrigin === 'editor') return undefined;
    const r = revealQuery.data;
    if (!r || typeof r.line !== 'number') return undefined;
    return { line: r.line, col: r.col };
  }, [revealQuery.data, selectionOrigin]);

  // text→diagram link: cursor offset → innermost element via the ViewModel
  // text-map → select (origin 'editor'). Debounced; only fires when the
  // resolved element actually changes, so it won't spam fetchDetail.
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onCursorOffset = useCallback(
    (offset: number) => {
      if (!focusedUri) return;
      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        const { textMap, textMapUri } = useDiagramLinkStore.getState();
        if (textMapUri !== focusedUri) return;
        const id = elementAtOffset(textMap, focusedUri, offset);
        if (!id) return;
        const cur = useSelectionStore.getState();
        if (cur.selectedElementId === id) return;
        select(focusedUri, id, 'editor');
      }, 120);
    },
    [focusedUri, select],
  );

  useEffect(
    () => () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    },
    [],
  );

  // Local mirror of buffer text so Monaco stays an uncontrolled editor
  // (controlled mode causes cursor jumps on every keystroke). We seed
  // from the store on focus change AND when the store transitions from
  // empty (just-hydrated) to populated (lazy-fetched above), then let
  // the change handler push back into the store. The LSP didChange
  // path is independent and happens inside attachLspClient.
  const [buffer, setBuffer] = useState<string>(file?.source ?? '');
  // Focus change OR workspace reload re-seeds the buffer. On reload the
  // store source was reset to '' (hydrateWorkspaceStore), so this drops
  // the pre-reload text, the loading branch below unmounts Monaco
  // (LSP didClose), and the lazy fetch re-seeds fresh from the backend.
  const activeSeedKey = focusedUri ? `${focusedUri}#${reloadEpoch}` : null;
  const [seedKey, setSeedKey] = useState<string | null>(activeSeedKey);

  useEffect(() => {
    if (activeSeedKey && activeSeedKey !== seedKey) {
      setBuffer(file?.source ?? '');
      setSeedKey(activeSeedKey);
    } else if (buffer === '' && file?.source && file.source.length > 0) {
      // Same focused URI, but the lazy seed just landed. Pull it in.
      setBuffer(file.source);
    }
  }, [activeSeedKey, file?.source, seedKey, buffer]);

  if (!focusedUri) {
    return (
      <div
        data-testid="source-panel-empty"
        style={{
          padding: 12,
          fontSize: 11,
          color: 'var(--outline)',
        }}
      >
        Load a file or workspace to start editing source.
      </div>
    );
  }

  if (!file || (needsSeed && buffer === '')) {
    return (
      <div data-testid="source-panel-loading" style={{ padding: 12, fontSize: 11 }}>
        {sourceQuery.isError
          ? `Failed to load source: ${
              sourceQuery.error instanceof Error
                ? sourceQuery.error.message
                : String(sourceQuery.error)
            }`
          : 'Loading source…'}
      </div>
    );
  }

  return (
    <MonacoSysmlEditor
      key={`${focusedUri}#${reloadEpoch}`}
      value={buffer}
      lspUri={focusedUri}
      revealLineCol={revealLineCol}
      onChange={(next) => {
        setBuffer(next);
        updateSource(focusedUri, next);
      }}
      onCursorOffset={onCursorOffset}
      testId="source-panel-editor"
    />
  );
}
