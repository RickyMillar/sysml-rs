/**
 * useVariablesState — local Zustand store for Variables pane UI state.
 *
 * Persists pin, expand, filter-chip, and "show sparklines" preferences to
 * localStorage so the pane resumes as the user left it. Deliberately
 * small: this store owns ONLY per-pane client state — variable data flows
 * through `VariableInspection` + `SessionEvents` (Round 1 primitives), not
 * through here.
 *
 * Storage shape (version 1):
 *   key = `sysml.variables.v1`
 *   {
 *     pinned: string[];        // variable names user pinned
 *     collapsed: string[];     // tree paths user collapsed
 *     showSparklines: boolean; // per-pane toggle
 *     filter: VariableFilter;  // currently-selected chip
 *   }
 *
 * `search` is NOT persisted — it's transient query state.
 */

import { useSyncExternalStore } from 'react';
import type { VariableFilter } from './VariableTree';

// ── Persisted shape ──────────────────────────────────────────────────

export interface VariablesPaneState {
  pinned: Set<string>;
  collapsed: Set<string>;
  showSparklines: boolean;
  filter: VariableFilter;
  /** Transient, not persisted. */
  search: string;
  /** Transient selection; drives keyboard nav + drill-down. */
  selectedPath: string | null;
}

export interface VariablesPaneActions {
  togglePinned: (name: string) => void;
  setPinned: (names: Iterable<string>) => void;
  toggleCollapsed: (path: string) => void;
  setCollapsed: (paths: Iterable<string>) => void;
  expandAll: () => void;
  setShowSparklines: (show: boolean) => void;
  setFilter: (filter: VariableFilter) => void;
  setSearch: (query: string) => void;
  setSelectedPath: (path: string | null) => void;
  reset: () => void;
}

export type VariablesPaneStore = VariablesPaneState & VariablesPaneActions;

// ── Storage contract (swappable for tests) ───────────────────────────

export interface VariablesStorage {
  load(): Partial<PersistedState> | null;
  save(state: PersistedState): void;
}

interface PersistedState {
  pinned: string[];
  collapsed: string[];
  showSparklines: boolean;
  filter: VariableFilter;
}

const STORAGE_KEY = 'sysml.variables.v1';

/**
 * Trailing-edge debounce window for localStorage writes. High-frequency
 * sources (rapid pin/collapse toggling, filter-chip scrubbing) would
 * otherwise trigger a full `JSON.stringify + setItem` on every mutation.
 * 500 ms is small enough that a page refresh after an idle moment sees
 * the user's last action, large enough that interactive rapid-fire
 * edits collapse into a single write.
 */
const LOCALSTORAGE_PERSIST_DEBOUNCE_MS = 500;

/** Production storage backed by window.localStorage (guarded for SSR/tests). */
function localStorageBacking(): VariablesStorage {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: PersistedState | null = null;

  const flush = (): void => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (pending === null) return;
    const snapshot = pending;
    pending = null;
    try {
      if (typeof localStorage === 'undefined') return;
      localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
    } catch {
      // silent — quota / private-mode errors shouldn't break the UI
    }
  };

  // Flush any pending write when the page is about to unload so a
  // mid-scrub close doesn't lose the user's last action. `beforeunload`
  // isn't guaranteed on mobile background-tab-kill, but it catches the
  // common case.
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    window.addEventListener('beforeunload', flush);
  }

  return {
    load() {
      try {
        if (typeof localStorage === 'undefined') return null;
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return null;
        const parsed = JSON.parse(raw) as Partial<PersistedState>;
        return parsed ?? null;
      } catch {
        return null;
      }
    },
    save(state) {
      pending = state;
      if (timer !== null) return;
      timer = setTimeout(() => {
        timer = null;
        flush();
      }, LOCALSTORAGE_PERSIST_DEBOUNCE_MS);
    },
  };
}

/** In-memory backing used by tests + SSR. */
export function createMemoryStorage(initial?: Partial<PersistedState>): VariablesStorage {
  let state: Partial<PersistedState> | null = initial ?? null;
  return {
    load: () => state,
    save: (s) => { state = s; },
  };
}

// ── Store implementation ─────────────────────────────────────────────

/**
 * Vanilla external store — plain function, no React dependency. The React
 * hook below subscribes via useSyncExternalStore so components re-render
 * when pin/collapse/filter/search changes.
 */
export function createVariablesStore(
  storage: VariablesStorage = localStorageBacking(),
): {
  getState: () => VariablesPaneState;
  subscribe: (listener: () => void) => () => void;
  actions: VariablesPaneActions;
} {
  const persisted = storage.load() ?? {};

  let state: VariablesPaneState = {
    pinned: new Set<string>(persisted.pinned ?? []),
    collapsed: new Set<string>(persisted.collapsed ?? []),
    showSparklines: persisted.showSparklines ?? true,
    filter: (persisted.filter as VariableFilter | undefined) ?? 'all',
    search: '',
    selectedPath: null,
  };

  const listeners = new Set<() => void>();
  const notify = () => { for (const l of listeners) l(); };

  const persist = (): void => {
    storage.save({
      pinned: Array.from(state.pinned),
      collapsed: Array.from(state.collapsed),
      showSparklines: state.showSparklines,
      filter: state.filter,
    });
  };

  const update = (patch: Partial<VariablesPaneState>, shouldPersist = true): void => {
    state = { ...state, ...patch };
    if (shouldPersist) persist();
    notify();
  };

  const actions: VariablesPaneActions = {
    togglePinned(name) {
      const next = new Set(state.pinned);
      if (next.has(name)) next.delete(name); else next.add(name);
      update({ pinned: next });
    },
    setPinned(names) {
      update({ pinned: new Set(names) });
    },
    toggleCollapsed(path) {
      const next = new Set(state.collapsed);
      if (next.has(path)) next.delete(path); else next.add(path);
      update({ collapsed: next });
    },
    setCollapsed(paths) {
      update({ collapsed: new Set(paths) });
    },
    expandAll() {
      update({ collapsed: new Set() });
    },
    setShowSparklines(show) {
      update({ showSparklines: show });
    },
    setFilter(filter) {
      update({ filter });
    },
    setSearch(query) {
      // search is transient — update state but don't re-persist
      update({ search: query }, false);
    },
    setSelectedPath(path) {
      update({ selectedPath: path }, false);
    },
    reset() {
      update({
        pinned: new Set(),
        collapsed: new Set(),
        showSparklines: true,
        filter: 'all',
        search: '',
        selectedPath: null,
      });
    },
  };

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    actions,
  };
}

// ── Singleton + React hook ───────────────────────────────────────────

let singleton: ReturnType<typeof createVariablesStore> | null = null;

/**
 * Lazy singleton — we defer until first access so tests can reset via
 * `__resetVariablesStoreForTests` without paying the localStorage load at
 * module evaluation.
 */
function getSingleton() {
  if (!singleton) singleton = createVariablesStore();
  return singleton;
}

/** Public hook consumed by VariablesPane + VariableRow. */
export function useVariablesState(): VariablesPaneStore {
  const store = getSingleton();
  const state = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  return { ...state, ...store.actions };
}

/** Test helper — reset the singleton between specs. */
export function __resetVariablesStoreForTests(
  storage?: VariablesStorage,
): ReturnType<typeof createVariablesStore> {
  singleton = createVariablesStore(storage);
  return singleton;
}
