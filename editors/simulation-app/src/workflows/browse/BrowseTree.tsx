/**
 * BrowseTree — left-rail package + element tree for the Browse floor
 * (ninebar Phase 1.5).
 *
 * Reuses the exact data pipeline the Run page's session tree already
 * runs on: `useSessionModelTree` composes `useWorkspaceUris` +
 * `useWorkspaceTree` (workspace-level `GET /models/{uri}/tree`, no
 * session) into a structural `ModelTreeNode[]`, then optionally
 * overlays live session state. `expectedSessionId: null` makes that
 * overlay a guaranteed no-op here — Browse must render identically
 * whether or not some OTHER workflow currently has a session running
 * (plan: "this surface must work with zero sessions"), so the tree
 * never merges live values regardless of what's live elsewhere.
 *
 * Rendering reuses the pure presentational `ModelTreeView` (built on
 * `ModelTreeNodeRow`, already `--row-dense` — ninebar Phase 1 density
 * tier) verbatim, unforked. `groupByPackage: true` gives the "package +
 * element tree" shape the plan asks for (mirrors the demo's per-package
 * section headers).
 *
 * Selection drives the shared `useSelectionStore` — the same store the
 * reading surface and the trace matrix read from — so a click here, a
 * click on a trace-matrix row, and this tree's own highlight all stay
 * in lock-step regardless of which surface originated the selection
 * (`focusedNodeId` below is derived FROM the store, not written only
 * by this component's own clicks).
 */
import { useEffect, useMemo, useState } from 'react';
import { useSessionModelTree } from '@/features/sessions/tree/useSessionModelTree';
import { ModelTreeView } from '@/features/sessions/tree/ModelTreeView';
import { findPathToNode } from '@/features/sessions/tree/buildModelTree';
import type { ModelTreeNode } from '@/features/sessions/tree/types';
import { useSelectionStore } from '@/features/selection/store';

function countNodes(tree: readonly ModelTreeNode[]): number {
  let n = 0;
  const walk = (nodes: readonly ModelTreeNode[]) => {
    for (const node of nodes) {
      n++;
      walk(node.children);
    }
  };
  walk(tree);
  return n;
}

/** Find the tree-position id of the node whose underlying element id
 *  matches `elementId` — the tree's own `id` can differ from
 *  `elementId` when the backend's dedupe pass rewrote it (typed-def
 *  inlining), so highlighting has to match on `elementId`. */
function findNodeIdByElementId(
  tree: readonly ModelTreeNode[],
  elementId: string,
): string | null {
  for (const node of tree) {
    if (node.elementId === elementId) return node.id;
    const inner = findNodeIdByElementId(node.children, elementId);
    if (inner) return inner;
  }
  return null;
}

export function BrowseTree() {
  const { tree, isLoading } = useSessionModelTree({
    groupByPackage: true,
    // Browse never mixes in live session state — see file doc comment.
    expectedSessionId: null,
  });

  const [expandedSet, setExpandedSet] = useState<Set<string>>(() => new Set());
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    if (seeded || tree.length === 0) return;
    // First-arrival seed: open the root (package) level only. Deeper
    // levels open on click — Browse's tree can span the whole
    // workspace, so auto-expanding further risks a wall of rows.
    setExpandedSet(new Set(tree.filter((n) => n.children.length > 0).map((n) => n.id)));
    setSeeded(true);
  }, [tree, seeded]);

  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const select = useSelectionStore((s) => s.select);

  const focusedNodeId = useMemo(
    () => (selectedElementId ? findNodeIdByElementId(tree, selectedElementId) : null),
    [tree, selectedElementId],
  );

  // A selection that originated elsewhere (e.g. a trace-matrix row
  // click) should still land visibly in the tree — expand its ancestor
  // chain so the highlighted row isn't hidden under a collapsed parent.
  useEffect(() => {
    if (!focusedNodeId) return;
    const chain = findPathToNode(tree, focusedNodeId);
    if (!chain) return;
    setExpandedSet((prev) => {
      let changed = false;
      const next = new Set(prev);
      for (const ancestor of chain.slice(0, -1)) {
        if (!next.has(ancestor.id)) {
          next.add(ancestor.id);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [focusedNodeId, tree]);

  return (
    <div data-testid="browse-tree" className="flex flex-col h-full overflow-hidden">
      <div
        className="flex items-center shrink-0 px-3"
        style={{
          height: 'var(--row-default)',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-secondary)',
          textTransform: 'uppercase',
          letterSpacing: '0.03em',
          borderBottom: '1px solid var(--border-default)',
        }}
      >
        <span className="flex-1">Model</span>
        <span
          data-testid="browse-tree-count"
          style={{ fontFamily: 'var(--font-mono)', textTransform: 'none', letterSpacing: 0 }}
        >
          {isLoading ? '…' : countNodes(tree)}
        </span>
      </div>
      <div className="flex-1 min-h-0">
        <ModelTreeView
          tree={tree}
          expandedSet={expandedSet}
          onToggleExpand={(id) =>
            setExpandedSet((prev) => {
              const next = new Set(prev);
              if (next.has(id)) next.delete(id);
              else next.add(id);
              return next;
            })
          }
          focusedId={focusedNodeId}
          onSelectNode={(node) => select(node.uri ?? null, node.elementId ?? node.id)}
          testIdPrefix="browse-tree-nodes"
        />
      </div>
    </div>
  );
}
