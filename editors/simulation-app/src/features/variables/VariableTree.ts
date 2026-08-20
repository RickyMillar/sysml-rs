/**
 * VariableTree — pure helpers for the Variables pane (R2.2).
 *
 * Takes the flat `name → value` map exposed by `VariableInspection.current`
 * plus metadata (constraint participation, pin state, recent-change tick)
 * and produces a structured, filtered, searchable tree ready for render.
 *
 * Pure functions only — zero React, zero Zustand, zero storage. Every
 * hierarchy / search / filter rule lives here so it can be tested without a
 * DOM and reused from any workflow that needs a variable browser
 * (compare view, trade-study inspector, future scripting console).
 */

// ── Public types ─────────────────────────────────────────────────────

/**
 * A raw variable value carried through the pane.
 * Matches the loose shape of backend snapshot values — numbers, strings,
 * booleans, nulls, and structured payloads (Quantity, Complex, ...).
 */
export type VariableValue = string | number | boolean | null | Record<string, unknown>;

/** One variable's live state, assembled from engine surfaces by VariablesPane. */
export interface VariableEntry {
  /** Fully-qualified dotted name, e.g. `circuit1.breaker.phaseIn.flow`. */
  name: string;
  /** Latest value. Pane formats via `formatVariableValue`. */
  value: VariableValue;
  /** Optional unit from MetricRegistry (e.g. "K", "A"). */
  unit?: string;
  /** Physics domain classification (for future colouring hooks). */
  domain?: string;
  /** If this variable participates in a constraint, the current verdict. */
  constraint?: ConstraintVerdict;
  /** Tick number where the value last changed; null if never observed. */
  lastChangedTick?: number | null;
}

/**
 * Constraint verdict — P/F/I/E mirrors R2.5 (Agent L).
 * 'pass'        — constraint satisfied
 * 'fail'        — constraint violated
 * 'inconclusive'— evaluator returned undefined (e.g. missing operand)
 * 'error'       — evaluator threw (unit mismatch, division by zero)
 */
export type ConstraintVerdict = 'pass' | 'fail' | 'inconclusive' | 'error';

/** One node in the render tree. Leaves represent variables; groups do not. */
export interface VariableTreeNode {
  /** Segment label (e.g. "breaker" in circuit1.breaker.phaseIn). */
  label: string;
  /** Full dotted path from root to this node. */
  path: string;
  /** Depth from root (0 = first segment). Drives indent. */
  depth: number;
  /** True when no further children — this is a leaf variable row. */
  isLeaf: boolean;
  /** Leaf entry (undefined on groups). */
  entry?: VariableEntry;
  /** Number of leaves in the subtree (1 on leaves). */
  leafCount: number;
  /** Children in stable alphabetical order; groups first then leaves. */
  children: VariableTreeNode[];
}

/** Filter chip identity — mirrors the header chips from the R2.2 brief. */
export type VariableFilter =
  | 'all'
  | 'passing'
  | 'failing'
  | 'inconclusive'
  | 'error'
  | 'changed'
  | 'pinned';

/** Options passed to buildTree. All fields optional with safe defaults. */
export interface BuildTreeOptions {
  /** Set of fully-qualified names marked as pinned by the user. */
  pinned?: Set<string>;
  /** Free-text search — case-insensitive substring match on the FQ name. */
  search?: string;
  /** Active filter chip. */
  filter?: VariableFilter;
  /** Current tick; used by the `changed` filter together with `recentWindow`. */
  currentTick?: number;
  /** How many ticks back counts as "recent" for the `changed` filter. Default 10. */
  recentWindow?: number;
  /**
   * Names to hide by default — the backend emits book-keeping variables
   * (`__t_ms`, `tick`) that pollute the user-facing tree. Callers can
   * override to show them (e.g. a power-user debug toggle).
   */
  hidden?: (name: string) => boolean;
}

// ── Defaults ─────────────────────────────────────────────────────────

const DEFAULT_HIDDEN = (name: string): boolean =>
  name.startsWith('__') || name === 't_ms' || name === 'tick' || name === 'clock_time';

const DEFAULT_RECENT_WINDOW = 10;

// ── Filtering ────────────────────────────────────────────────────────

/**
 * Apply search + filter chip + default-hidden rules to a flat entry list.
 *
 * Pure — callers should memoize upstream. The `search` term supports
 * substring match (case-insensitive) and `*` as a wildcard, so users can
 * type `circuit*.temp` to pre-filter across instances. This matches
 * the VariableBrowser glob-filter convention that already shipped.
 */
export function filterEntries(
  entries: VariableEntry[],
  opts: BuildTreeOptions = {},
): VariableEntry[] {
  const {
    pinned = new Set<string>(),
    search = '',
    filter = 'all',
    currentTick,
    recentWindow = DEFAULT_RECENT_WINDOW,
    hidden = DEFAULT_HIDDEN,
  } = opts;

  let result = entries.filter((e) => !hidden(e.name));

  // Chip filter
  if (filter !== 'all') {
    result = result.filter((e) => {
      switch (filter) {
        case 'passing':      return e.constraint === 'pass';
        case 'failing':      return e.constraint === 'fail';
        case 'inconclusive': return e.constraint === 'inconclusive';
        case 'error':        return e.constraint === 'error';
        case 'pinned':       return pinned.has(e.name);
        case 'changed': {
          if (currentTick === undefined || e.lastChangedTick == null) return false;
          return currentTick - e.lastChangedTick <= recentWindow;
        }
        default: return true;
      }
    });
  }

  // Search (glob or substring)
  const trimmed = search.trim();
  if (trimmed.length > 0) {
    const matcher = trimmed.includes('*')
      ? buildGlobMatcher(trimmed)
      : buildSubstringMatcher(trimmed);
    result = result.filter((e) => matcher(e.name));
  }

  return result;
}

/** Build a case-insensitive glob matcher. `*` -> `.*`. */
export function buildGlobMatcher(pattern: string): (name: string) => boolean {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
  const re = new RegExp('^' + escaped + '$', 'i');
  return (name) => re.test(name);
}

/** Case-insensitive substring matcher. */
export function buildSubstringMatcher(term: string): (name: string) => boolean {
  const lower = term.toLowerCase();
  return (name) => name.toLowerCase().includes(lower);
}

// ── Tree construction ────────────────────────────────────────────────

/**
 * Build a hierarchical render tree from filtered entries.
 *
 * Grouping: dotted names are split on `.`; each segment becomes a group
 * node. Single-segment names (no dot) become top-level leaves, same as
 * the existing VariableBrowser. Children are sorted groups-first, then
 * alphabetically within each class.
 */
export function buildTree(
  entries: VariableEntry[],
  opts: BuildTreeOptions = {},
): VariableTreeNode[] {
  const filtered = filterEntries(entries, opts);
  const root: VariableTreeNode = makeNode('', '', -1, false);

  for (const entry of filtered) {
    const parts = entry.name.split('.');
    let parent = root;
    for (let i = 0; i < parts.length; i++) {
      const label = parts[i];
      const path = parts.slice(0, i + 1).join('.');
      const isLeaf = i === parts.length - 1;
      let child = parent.children.find((c) => c.label === label);
      if (!child) {
        child = makeNode(label, path, parent.depth + 1, isLeaf);
        parent.children.push(child);
      }
      if (isLeaf) {
        child.isLeaf = true;
        child.entry = entry;
      }
      parent = child;
    }
  }

  sortTree(root);
  computeLeafCounts(root);
  return root.children;
}

/**
 * Split entries into the "pinned" virtual group + everything else.
 * Consumers render the pinned block at the top of the pane.
 */
export function partitionPinned(
  entries: VariableEntry[],
  pinned: Set<string>,
): { pinned: VariableEntry[]; rest: VariableEntry[] } {
  if (pinned.size === 0) return { pinned: [], rest: entries };
  const pin: VariableEntry[] = [];
  const rest: VariableEntry[] = [];
  for (const e of entries) {
    (pinned.has(e.name) ? pin : rest).push(e);
  }
  pin.sort((a, b) => a.name.localeCompare(b.name));
  return { pinned: pin, rest };
}

/**
 * Flatten a tree to a linear row list (respecting per-path expand state).
 *
 * Virtualised callers (IntersectionObserver or windowed lists) consume
 * the flat array directly; hierarchical callers can still walk via
 * `children`. `collapsed` is a Set of paths whose subtrees are hidden.
 */
export function flattenTree(
  nodes: VariableTreeNode[],
  collapsed: Set<string>,
): VariableTreeNode[] {
  const out: VariableTreeNode[] = [];
  const walk = (list: VariableTreeNode[]) => {
    for (const n of list) {
      out.push(n);
      if (n.children.length > 0 && !collapsed.has(n.path)) walk(n.children);
    }
  };
  walk(nodes);
  return out;
}

// ── Counter helpers (filter-chip badges) ─────────────────────────────

/** Total counts per filter chip, computed from the unfiltered entry list. */
export interface FilterCounts {
  all: number;
  passing: number;
  failing: number;
  inconclusive: number;
  error: number;
  changed: number;
  pinned: number;
}

/**
 * Compute filter-chip counts. Matches the semantics of `filterEntries`
 * so chips mirror what the user will see when they click them.
 */
export function computeFilterCounts(
  entries: VariableEntry[],
  opts: Omit<BuildTreeOptions, 'search' | 'filter'> = {},
): FilterCounts {
  const {
    pinned = new Set<string>(),
    currentTick,
    recentWindow = DEFAULT_RECENT_WINDOW,
    hidden = DEFAULT_HIDDEN,
  } = opts;

  const visible = entries.filter((e) => !hidden(e.name));
  const counts: FilterCounts = {
    all: visible.length,
    passing: 0,
    failing: 0,
    inconclusive: 0,
    error: 0,
    changed: 0,
    pinned: 0,
  };

  for (const e of visible) {
    if (e.constraint === 'pass') counts.passing++;
    else if (e.constraint === 'fail') counts.failing++;
    else if (e.constraint === 'inconclusive') counts.inconclusive++;
    else if (e.constraint === 'error') counts.error++;
    if (pinned.has(e.name)) counts.pinned++;
    if (currentTick !== undefined && e.lastChangedTick != null &&
        currentTick - e.lastChangedTick <= recentWindow) {
      counts.changed++;
    }
  }
  return counts;
}

// ── Value formatting ─────────────────────────────────────────────────

/**
 * Format a raw value for the right-aligned value column.
 *
 * Rules:
 *   - null / undefined → "—"
 *   - number with unit → "123.45 K" (up to 4 sig digits, trimmed zeros)
 *   - number w/o unit  → same, no unit suffix
 *   - booleans         → "true" / "false"
 *   - strings          → as-is (trimmed of surrounding whitespace)
 *   - structured values (Quantity, Complex) → JSON-compact
 */
export function formatVariableValue(
  value: VariableValue,
  unit?: string,
): string {
  if (value == null) return '\u2014';
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'number') {
    const pretty = formatNumber(value);
    return unit ? `${pretty} ${unit}` : pretty;
  }
  if (typeof value === 'string') {
    const trimmed = value.trim();
    return unit && trimmed !== '' ? `${trimmed} ${unit}` : trimmed;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return String(n);
  if (n === 0) return '0';
  const abs = Math.abs(n);
  if (abs >= 1e6 || abs < 1e-3) return n.toExponential(3);
  const formatted = n.toPrecision(5);
  // Trim trailing zeros after a decimal point.
  return formatted.includes('.') ? formatted.replace(/\.?0+$/, '') : formatted;
}

// ── Internals ────────────────────────────────────────────────────────

function makeNode(
  label: string,
  path: string,
  depth: number,
  isLeaf: boolean,
): VariableTreeNode {
  return { label, path, depth, isLeaf, leafCount: 0, children: [] };
}

function sortTree(node: VariableTreeNode): void {
  node.children.sort((a, b) => {
    const aGroup = a.children.length > 0 || !a.isLeaf;
    const bGroup = b.children.length > 0 || !b.isLeaf;
    if (aGroup !== bGroup) return aGroup ? -1 : 1;
    return a.label.localeCompare(b.label);
  });
  node.children.forEach(sortTree);
}

function computeLeafCounts(node: VariableTreeNode): number {
  if (node.isLeaf && node.children.length === 0) {
    node.leafCount = 1;
    return 1;
  }
  let count = 0;
  for (const c of node.children) count += computeLeafCounts(c);
  node.leafCount = count;
  return count;
}
