/**
 * Diagram-link store — shared lookup data for the bidirectional text↔diagram
 * link (Bucket 2.3).
 *
 * The single source of truth for *selection* stays `features/selection/store.ts`
 * (`selectedElementId`). This store holds only the lookup DATA the cursor→diagram
 * direction needs: the ViewModel `text_map` for the active view, written by
 * `SvgCanvas` when it fetches a ViewModel, and read by `SourcePanel`'s cursor
 * handler to resolve an editor offset → ElementId via `elementAtOffset`.
 */

import { create } from 'zustand';
import type { TextMap } from '@/diagram-svg/viewmodel-types';

/**
 * Reverse lookup mirroring Rust `TextMap::element_at` — the innermost element
 * whose byte span contains `offset`, ties broken toward the smallest span.
 *
 * `offset` is a UTF-8 **byte** offset (the Monaco adapter converts its UTF-16
 * cursor offset via `utf16OffsetToByteOffset` before calling here), so it matches
 * the Rust spans directly — correct for non-ASCII source, not just ASCII.
 */
export function elementAtOffset(
  textMap: TextMap | null,
  file: string,
  offset: number,
): string | null {
  if (!textMap) return null;
  let best: string | null = null;
  let bestLen = Infinity;
  for (const [id, span] of Object.entries(textMap.spans)) {
    if (span.file !== file) continue;
    if (offset < span.start || offset >= span.end) continue;
    const len = span.end - span.start;
    if (len < bestLen) {
      bestLen = len;
      best = id;
    }
  }
  return best;
}

interface DiagramLinkState {
  textMap: TextMap | null;
  /** URI the current text_map belongs to (guards cross-file cursor lookups). */
  textMapUri: string | null;
  setTextMap: (uri: string | null, textMap: TextMap | null) => void;
}

export const useDiagramLinkStore = create<DiagramLinkState>((set) => ({
  textMap: null,
  textMapUri: null,
  setTextMap: (uri, textMap) => set({ textMapUri: uri, textMap }),
}));
