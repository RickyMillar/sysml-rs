/**
 * Selection store — tracks the currently selected model element.
 *
 * This is a fresh implementation that uses the shared `httpGet` wrapper
 * instead of raw fetch. The inspector reads from this store.
 *
 * Selection is orthogonal to activities — you can inspect element X
 * while running a session on element Y.
 */

import { create } from 'zustand';
import { httpGet } from '@/shared/api/http';
import type { ElementChild, ElementDetail, ElementSpan } from '@/types/element';

/**
 * Where a selection came from. Lets reveal-on-select logic avoid feedback loops:
 * an `'editor'`-origin selection (driven by the Monaco cursor) must NOT snap the
 * cursor back via `revealLineCol`, or cursor↔selection would oscillate.
 */
export type SelectionOrigin = 'ui' | 'diagram' | 'editor';

interface SelectionState {
  selectedElementId: string | null;
  selectedUri: string | null;
  selectionOrigin: SelectionOrigin;
  elementDetail: ElementDetail | null;
  loading: boolean;

  /** Select an element (from tree click, diagram click, or editor cursor). */
  select: (uri: string | null, elementId: string | null, origin?: SelectionOrigin) => void;

  /** Clear selection. */
  clear: () => void;
}

export const useSelectionStore = create<SelectionState>((set) => ({
  selectedElementId: null,
  selectedUri: null,
  selectionOrigin: 'ui',
  elementDetail: null,
  loading: false,

  select: (uri, elementId, origin = 'ui') => {
    set({
      selectedElementId: elementId,
      selectedUri: uri,
      selectionOrigin: origin,
      elementDetail: null,
      loading: !!elementId,
    });

    // Auto-fetch detail
    if (uri && elementId) {
      fetchDetail(uri, elementId);
    }
  },

  clear: () =>
    set({
      selectedElementId: null,
      selectedUri: null,
      selectionOrigin: 'ui',
      elementDetail: null,
      loading: false,
    }),
}));

// ── Detail fetch (outside store to avoid circular getState) ──────────

/** Backend Element shape (from /models/:uri/elements/:id). */
interface BackendElement {
  id?: unknown;
  kind?: unknown;
  name?: unknown;
  owner?: unknown;
  owning_membership?: unknown;
  qname?: unknown;
  props?: Record<string, unknown> | null;
  spans?: unknown;
}

function stringifyValue(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

function normalizeChild(raw: BackendElement): ElementChild {
  return {
    id: typeof raw.id === 'string' ? raw.id : stringifyValue(raw.id),
    name: typeof raw.name === 'string' ? raw.name : null,
    kind: typeof raw.kind === 'string' ? raw.kind : stringifyValue(raw.kind),
  };
}

async function fetchDetail(uri: string, elementId: string) {
  const elementPath = `/models/${encodeURIComponent(uri)}/elements/${encodeURIComponent(elementId)}`;
  const childrenPath = `${elementPath}/children`;

  try {
    // Fetch element and children in parallel — backend returns the element
    // record for the inspector header/properties and a separate list of
    // owned children. The two endpoints are independent so we await them
    // together rather than serially.
    const [data, childrenRaw] = await Promise.all([
      httpGet<BackendElement>(elementPath),
      httpGet<BackendElement[]>(childrenPath).catch(() => [] as BackendElement[]),
    ]);

    const props: Record<string, string> = {};
    if (data.props && typeof data.props === 'object') {
      for (const [k, v] of Object.entries(data.props)) {
        props[k] = stringifyValue(v);
      }
    }

    const children: ElementChild[] = Array.isArray(childrenRaw)
      ? childrenRaw.map((c) => normalizeChild(c))
      : [];

    const spans: ElementSpan[] = Array.isArray(data.spans)
      ? (data.spans as ElementSpan[])
      : [];

    const qualifiedName =
      typeof data.qname === 'string'
        ? data.qname
        : data.qname && typeof data.qname === 'object'
          ? stringifyValue(data.qname)
          : null;

    const detail: ElementDetail = {
      id: typeof data.id === 'string' ? data.id : elementId,
      name: typeof data.name === 'string' ? data.name : null,
      kind: typeof data.kind === 'string' ? data.kind : 'Unknown',
      owner: typeof data.owner === 'string' ? data.owner : null,
      owningMembership:
        typeof data.owning_membership === 'string' ? data.owning_membership : null,
      qualifiedName,
      props,
      children,
      spans,
    };

    // Only update if the element is still selected
    const current = useSelectionStore.getState();
    if (current.selectedElementId === elementId && current.selectedUri === uri) {
      useSelectionStore.setState({ elementDetail: detail, loading: false });
    }
  } catch {
    useSelectionStore.setState({ elementDetail: null, loading: false });
  }
}
