/**
 * Pure filtering helpers for the Phase B session tree.
 *
 * Split from `buildModelTree` so structural vs display-filter concerns
 * stay separate — the structural tree is authoritative and cached
 * across ticks; these filters run at render-time when the user
 * changes a chip or types in the search box.
 *
 * Filter behaviour:
 *   - ALL: show every surviving node. Still drops anonymous
 *     AttributeUsage (machine-generated helper features from nested
 *     expression results — always noise for humans).
 *   - LIVE: keep only attributes whose value has been observed this
 *     session. Ancestor chain is preserved so the user can still see
 *     "where" the attribute lives.
 *   - PINNED: keep only attributes the user has starred. Same
 *     ancestor preservation.
 *
 * Search: orthogonal to the chip filter. Matches (case-insensitive)
 * against the node's name and ownerPath. Ancestor chain preserved.
 *
 * Definition / Usage de-duplication: at runtime we care about
 * instances (PartUsage, AttributeUsage, …) not their class
 * definitions. `definitionMode` controls whether a *Definition* row
 * that has a corresponding *Usage* is shown, hidden, or styled
 * differently. Matching is driven exclusively by `TreeNode.typedAs`,
 * which the backend populates from
 * `sysml_core::resolution::scoping::chaining::find_feature_type`.
 * Definitions that no usage types against stay visible — surfacing
 * those is how typing-resolution gaps become apparent.
 */
import type { ModelTreeNode, AttributeTreeNode } from './types';

export type TreeFilterMode = 'all' | 'live' | 'pinned';

export type DefinitionMode = 'usages' | 'definitions' | 'both';

export interface FilterOptions {
  mode: TreeFilterMode;
  pinnedIds: ReadonlySet<string>;
  searchQuery?: string;
  /** Drop attributes whose name is empty / "(unnamed)" / uuid-shaped. */
  dropUnnamedAttributes?: boolean;
  /**
   * Which of the Definition / Usage duality to show.
   * - `usages` (default): drop *Definition* rows whose name has a
   *   matching *Usage* somewhere else in the tree. Lone definitions
   *   (entry-point classes with no usage) stay visible.
   * - `definitions`: drop every *Usage* row — spec-first view.
   * - `both`: keep everything; definitions still render (callers can
   *   style them distinctly via `rawKind`).
   */
  definitionMode?: DefinitionMode;
}

/** Returns a new tree with the filter applied. */
export function filterTree(
  tree: readonly ModelTreeNode[],
  options: FilterOptions,
): ModelTreeNode[] {
  const {
    mode,
    pinnedIds,
    searchQuery = '',
    dropUnnamedAttributes = true,
    definitionMode = 'usages',
  } = options;
  const q = searchQuery.trim().toLowerCase();

  // Pre-compute the set of definition ids that any usage is typed
  // by. Drives the Usages-mode drop check below — authoritative
  // from the backend's sysml-core `find_feature_type` index via
  // `TreeNode.typedAs`.
  const typedDefinitionIds =
    definitionMode === 'usages' ? collectTypedDefinitionIds(tree) : null;

  // The walk distinguishes three reasons a node may not be self-kept,
  // because they imply different subtree handling:
  //   - 'keep'          — node itself survives all checks
  //   - 'drop-subtree'  — node + all descendants are removed wholesale
  //                       (the Defs/Usages toggle: when we hide a def
  //                       because its usage shows elsewhere, the def's
  //                       OWN attributes / sub-parts must also vanish —
  //                       they belong to the def, not to the workspace
  //                       root, and ancestor-preservation would
  //                       resurrect the whole def via its kept attrs)
  //   - 'drop-self'     — node fails, but children may still surface
  //                       under the parent (chip filter, search,
  //                       machine-generated names — the existing
  //                       ancestor-preservation behaviour)
  type KeepDecision = 'keep' | 'drop-subtree' | 'drop-self';

  const keepSelf = (node: ModelTreeNode): KeepDecision => {
    // Machine-generated name drop — catches every archetype (not just
    // attributes). A part / sm / constraint / calc whose name is a
    // UUID is as noisy as an attribute. Ancestor preservation means
    // their named descendants still surface.
    if (
      dropUnnamedAttributes &&
      node.kind !== 'section' &&
      isMachineGeneratedName(node.name)
    ) {
      return 'drop-self';
    }

    // Definition / Usage toggle. Wholesale subtree drop — the
    // children of a hidden def belong to that def, not the workspace
    // root; resurrecting them via ancestor preservation is the bug
    // R2.4-era reviewers hit (a HallEffectSensor PartDefinition kept
    // bobbing back up because its `sensitivity` / `range_*`
    // AttributeUsage children passed every other filter).
    if (isDefinitionKind(node.rawKind)) {
      if (definitionMode === 'usages') {
        if (typedDefinitionIds && typedDefinitionIds.has(node.id)) {
          return 'drop-subtree';
        }
      }
      // `definitions` mode keeps defs; `both` keeps them.
    } else if (isUsageKind(node.rawKind)) {
      if (definitionMode === 'definitions') return 'drop-subtree';
    }

    // Filter-chip check: only gates the attribute archetype. Other
    // archetypes (part / sm / constraint / ode) stay visible so the
    // user still sees their structural context.
    if (node.kind === 'attribute') {
      if (mode === 'live') {
        const attr = node as AttributeTreeNode;
        if (attr.value === undefined) return 'drop-self';
      }
      if (mode === 'pinned') {
        if (!pinnedIds.has(node.id)) return 'drop-self';
      }
    }

    // Search check: any node whose name or ownerPath matches wins.
    // Non-matching nodes are still kept if ANY descendant matches
    // (ancestor preservation is handled below).
    if (q && !nodeMatchesQuery(node, q)) {
      return 'drop-self';
    }

    return 'keep';
  };

  // Ancestor preservation: a parent is kept when ANY surviving child
  // survives, even if the parent itself didn't match — that's what
  // makes chip filtering + search readable (user sees the live
  // attribute AND the path to it).
  //
  // EXCEPTION 1: when the node itself is machine-generated (uuid /
  // anon), show it as transparent — promote its surviving children
  // to the parent's level instead of keeping a row labelled with
  // gibberish. Same promotion rule PRUNE_KINDS follows in the
  // structural builder.
  //
  // EXCEPTION 2: when the node was dropped via the Defs/Usages
  // toggle (decision === 'drop-subtree'), the entire subtree goes
  // away — children belong to the hidden def, not to the surrounding
  // tree.
  const walk = (nodes: readonly ModelTreeNode[]): ModelTreeNode[] => {
    const out: ModelTreeNode[] = [];
    for (const node of nodes) {
      const decision = keepSelf(node);
      if (decision === 'drop-subtree') {
        // Hide the def + everything beneath it.
        continue;
      }
      const filteredChildren = walk(node.children);
      if (decision === 'drop-self') {
        if (
          dropUnnamedAttributes &&
          node.kind !== 'section' &&
          isMachineGeneratedName(node.name) &&
          filteredChildren.length > 0
        ) {
          // Transparent container: promote children.
          out.push(...filteredChildren);
          continue;
        }
        if (filteredChildren.length === 0) continue;
        // Ancestor preservation: keep the failing node so its
        // surviving descendants render in context.
        out.push({ ...node, children: filteredChildren } as ModelTreeNode);
        continue;
      }
      // decision === 'keep'
      out.push({ ...node, children: filteredChildren } as ModelTreeNode);
    }
    return out;
  };

  return walk(tree);
}

/**
 * Does this name look machine-generated / placeholder? Matches:
 *  - empty / "(unnamed)" — the explicit null-name fallback
 *  - full UUID (8-4-4-4-12) — backend id leaking as name
 *  - "anon_<hex>" / "anon-<hex>" — serialiser-generated anonymous
 *    feature names
 *  - names that *contain* a UUID substring anywhere (e.g.
 *    "lighting_5abc123f-def4-..." — authored prefix with a uuid
 *    tail some tools emit when disambiguating overrides)
 *  - names with 20+ consecutive hex-only chars, or two hex runs
 *    separated by a hyphen totalling 12+ chars — a catch-all for
 *    id-as-name fallbacks we haven't seen yet
 */
export function isMachineGeneratedName(name: string): boolean {
  if (!name) return true;
  if (name === '(unnamed)') return true;
  // Full UUID at end or anywhere — catches both bare IDs and
  // "lighting-<uuid>" style prefixed names.
  if (/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.test(name)) {
    return true;
  }
  // anon_<hex> / anon-<hex>.
  if (/^anon[_-][0-9a-f]{6,}$/i.test(name)) return true;
  // Two long hex runs with a dash separator — catches partial uuid
  // shapes like "abc12345-def67890" without the full 4 groups.
  if (/[0-9a-f]{6,}-[0-9a-f]{6,}/i.test(name)) return true;
  // Long unbroken hex run (≥16 chars) in the name — unlikely in
  // authored identifiers, common in id fallbacks.
  if (/[0-9a-f]{16,}/i.test(name)) return true;
  return false;
}

/** @deprecated — kept for backwards compatibility with existing tests. */
export const isUnnamedAttributeName = isMachineGeneratedName;

function nodeMatchesQuery(node: ModelTreeNode, qLower: string): boolean {
  if (node.name.toLowerCase().includes(qLower)) return true;
  if (node.ownerPath.toLowerCase().includes(qLower)) return true;
  return false;
}

// ── Definition / Usage helpers ────────────────────────────────────

/** `X` is a Definition-class raw kind (PartDefinition etc.). */
export function isDefinitionKind(rawKind: string): boolean {
  return rawKind.endsWith('Definition');
}

/** `X` is a Usage-class raw kind (PartUsage etc.). */
export function isUsageKind(rawKind: string): boolean {
  return rawKind.endsWith('Usage');
}

/**
 * Walk the tree, collect every `typedAs` id — the set of definition
 * element ids that any usage is typed by. The backend fills
 * `typedAs` via `sysml_core::resolution::scoping::chaining::find_feature_type`,
 * so this set is the authoritative answer to "which definitions are
 * instantiated somewhere in this workspace?" independent of name
 * overlap between usage and definition.
 */
export function collectTypedDefinitionIds(
  tree: readonly ModelTreeNode[],
): Set<string> {
  const out = new Set<string>();
  const walk = (nodes: readonly ModelTreeNode[]) => {
    for (const n of nodes) {
      if (n.typedAs) out.add(n.typedAs);
      walk(n.children);
    }
  };
  walk(tree);
  return out;
}
