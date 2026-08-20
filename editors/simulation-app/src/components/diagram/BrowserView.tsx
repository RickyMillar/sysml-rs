import { useState } from 'react';
import { useWorkspaceStore } from '@/store/workspace';
import type { TreeNode } from '@/shared/api/model';

/**
 * Containment-tree "Browser" view. Rendered when the store holds a `treeModel`
 * payload (DiagramHost dispatches by payload shape).
 *
 * Reads the typed `TreeModel` payload (`roots: TreeNode[]`) from the
 * workspace store. Tree shape is authoritative on the backend (strict
 * `element.owner.is_none()` for roots, recursive `children_of` for nesting,
 * memberships / imports filtered out) — this component only renders.
 *
 * Sprotty's tree layout is intentionally bypassed: a native React tree
 * gives us virtualization, keyboard nav, and accessible disclosure
 * semantics for free.
 */
function BrowserNodeRow({ node, depth }: { node: TreeNode; depth: number }) {
  const childCount = node.children?.length ?? 0;
  const hasChildren = childCount > 0;
  const [expanded, setExpanded] = useState(depth < 2);

  return (
    <div data-testid={`browser-node-${node.id}`}>
      <div
        onClick={() => hasChildren && setExpanded((e) => !e)}
        className={node.cssClasses?.join(' ')}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          padding: '4px 8px',
          paddingLeft: 8 + depth * 16,
          fontSize: 12,
          color: 'var(--on-surface)',
          cursor: hasChildren ? 'pointer' : 'default',
          borderBottom: '1px solid var(--outline-variant)',
        }}
      >
        <span
          className="material-symbols-outlined"
          style={{
            fontSize: 14,
            visibility: hasChildren ? 'visible' : 'hidden',
            color: 'var(--outline)',
          }}
        >
          {expanded ? 'expand_more' : 'chevron_right'}
        </span>
        <span style={{ fontWeight: 600 }}>{node.label}</span>
        {node.kindLabel && (
          <span style={{ color: 'var(--outline)', fontSize: 11 }}>{node.kindLabel}</span>
        )}
      </div>
      {expanded && hasChildren && (
        <>
          {node.children!.map((c) => (
            <BrowserNodeRow key={c.id} node={c} depth={depth + 1} />
          ))}
        </>
      )}
    </div>
  );
}

export function BrowserView() {
  const treeModel = useWorkspaceStore((s) => s.treeModel);
  const roots = treeModel?.roots ?? [];

  return (
    <div
      data-testid="browser-view-root"
      style={{
        width: '100%',
        height: '100%',
        overflow: 'auto',
        background: 'var(--surface-dim)',
      }}
    >
      {treeModel?.title && roots.length > 0 && (
        <div
          data-testid="browser-view-title"
          style={{
            padding: '12px 16px 8px',
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--on-surface)',
            borderBottom: '1px solid var(--outline-variant)',
          }}
        >
          {treeModel.title}
        </div>
      )}

      {roots.length === 0 ? (
        <div
          data-testid="browser-view-empty"
          style={{ padding: 24, color: 'var(--outline)', fontSize: 12 }}
        >
          {treeModel ? 'No top-level elements in this model.' : 'No model loaded.'}
        </div>
      ) : (
        roots.map((r) => <BrowserNodeRow key={r.id} node={r} depth={0} />)
      )}
    </div>
  );
}
