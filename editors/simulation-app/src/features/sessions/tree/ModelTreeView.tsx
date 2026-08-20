/**
 * ModelTreeView — the recursive tree container Phase B1a's
 * `ModelTreeNodeRow` plugs into.
 *
 * Pure presentation: owns no data, no stores, no selection. The
 * consumer (the to-be-built SessionTreeV2 panel, plus any future
 * re-uses like the diagram hover picker) passes in:
 *   - `tree` — the polymorphic ModelTreeNode[] from
 *     `useSessionModelTree`.
 *   - `expandedSet` + `onToggleExpand` — the expand/collapse state.
 *   - `focusedId` + `onSelectNode` — which node the detail region is
 *     tracking.
 *   - per-attribute decorations (pinned set, editable flag,
 *     sparklineSamples resolver).
 *
 * Keeping the state above the component lets the parent persist
 * expansion to localStorage and drive selection from `focusPath` in
 * the session store — the two features Phase B3 (breadcrumb) + Phase
 * B5 (inline editor) need to compose cleanly.
 *
 * UX closeout #4 / #17 (big-model freeze): below `ROW_VIRTUALIZATION_THRESHOLD`
 * rendered rows, this renders exactly as before — every existing
 * fixture (and hybrid-scale trees generally) sit comfortably under that
 * threshold, so small/medium models are byte-for-byte unaffected.
 * Above it (espresso-production-cell-scale), the flattened row list — the
 * expanded tree already comes out of `renderNodes` as one flat array,
 * respecting `expandedSet` — is windowed through `@tanstack/react-virtual`
 * so only the on-screen rows actually mount into the DOM, instead of
 * every part/attribute/sm/constraint row in the workspace at once.
 * Mirrors the `NODE_CAP=250` precedent already shipped for the diagram
 * (`SvgCanvas.tsx`, Bucket 3.11) for the same class of problem.
 */
import { memo, useRef, type ReactElement, type RefObject } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { ModelTreeNodeRow } from './ModelTreeNodeRow';
import type { ModelTreeNode, ModelTreeNodeKind } from './types';

/**
 * Row count above which `ModelTreeView` switches from mounting every
 * row to windowed rendering. Deliberately far above any existing test
 * fixture / hybrid-scale tree and far below espresso-production-cell-scale.
 */
export const ROW_VIRTUALIZATION_THRESHOLD = 300;

/** Flat-row estimate — mirrors the `--row-dense` token (16px; ninebar
 *  Phase 1 density tier, crib-sheet reconciliation #1) that
 *  `ModelTreeNodeRow`'s row/section styles now consume. `estimateSize`
 *  takes a plain number (react-virtual has no CSS-var awareness), so
 *  this constant can drift from the token — the virtualizer
 *  self-corrects visually via `estimateSize`, not exact per-row
 *  measurement, which keeps this simple and jsdom-safe. */
const ESTIMATED_ROW_HEIGHT_PX = 16;

/** A view-summary row injected as a synthetic tree child. */
export interface InlineViewChild {
  id: string;
  name: string | null;
  kind: string;
}

export interface ModelTreeViewProps {
  /** The polymorphic tree — already merged with live state. */
  tree: readonly ModelTreeNode[];
  /** Set of currently-expanded node ids. */
  expandedSet: ReadonlySet<string>;
  /** Called when a row's chevron is clicked. */
  onToggleExpand: (id: string) => void;
  /** Currently focused node id (drives the selection highlight). */
  focusedId?: string | null;
  /** Called when a row body is clicked. Consumer flips focusPath. */
  onSelectNode?: (node: ModelTreeNode) => void;
  /** Lookup of which attribute ids are pinned. */
  pinnedIds?: ReadonlySet<string>;
  /** Called when an attribute's pin icon is toggled. */
  onTogglePin?: (node: ModelTreeNode) => void;
  /** Whether to show the edit pencil on attributes. */
  editable?: boolean;
  /** Called when the edit pencil fires on an attribute row. */
  onEditAttribute?: (node: ModelTreeNode) => void;
  /** Returns the sparkline samples for an attribute — consumer owns
   *  the ring-buffer source (useTimeSeriesStore in practice). */
  getSparklineSamples?: (node: ModelTreeNode) => readonly number[];
  /** Called when a sparkline in an attribute / calc row is clicked.
   *  Consumer usually binds to `promoteToPlots` with activeSessionId. */
  onSparklineClick?: (fullName: string, node: ModelTreeNode) => void;
  /** Right-click handler on any row. */
  onContextMenu?: (node: ModelTreeNode, position: { x: number; y: number }) => void;
  /** Inline launch action for runnable model rows. */
  onLaunchRunnable?: (node: ModelTreeNode) => void;
  /**
   * Resolved views-by-element-id map. Built once per sysml.query view-list refresh
   * by the consumer. The row affordance only renders when this map has
   * a non-empty entry for `node.elementId`. Bucket 5-followup-2.
   */
  viewsByElementId?: ReadonlyMap<string, ReadonlyArray<InlineViewChild>>;
  /** Currently-selected ViewDefinition / ViewUsage id, for popover highlight. */
  selectedViewId?: string | null;
  /** Called when the user picks a view from a row's chip. */
  onPickView?: (viewId: string) => void;
  /**
   * Phase 3 — URI passed into each row's source-preview popover.
   * Pure pass-through. Container resolves this from the focused file
   * (or workspace URI for cross-file trees).
   */
  previewUri?: string | null;
  /** Phase 3 — called when a row's hover popover is clicked. */
  onPromotePreview?: (node: ModelTreeNode) => void;
  /** data-testid prefix for the whole tree. */
  testIdPrefix?: string;
  /**
   * Where archetype section headers ("Parts", "State machines", …) get
   * injected.
   *
   * - `'all'` (default): label every mixed-kind sibling group at any
   *   depth — the original behaviour, kept for the Browse structural
   *   browser where nesting-by-type is the point.
   * - `'root'`: emit headers ONLY for the top-level (depth 0) group.
   *   Nested container contents are then organised by containment +
   *   the per-row type icon, not by repeating the same category header
   *   at every depth.
   * - `'none'`: never emit archetype headers. Used by the package +
   *   ownership view, where the root level is packages (containers),
   *   not archetype groups — a "Parts"/"State machines" header over a
   *   list of packages would be nonsense.
   *
   * The Run session tree uses `'root'`. It merges every file's roots
   * into one flat, kind-sorted list, so depth 0 already surfaces every
   * archetype exactly once — a single header set replaces the previous
   * duplicate "State machines" / "Calculations" labels that appeared
   * once for a model's owned children (depth ≥ 1) and again for the
   * loose top-level usages from sibling files (depth 0). It also fixes
   * the mislabel where expanding a container spliced its sub-headers
   * into a same-kind run and stranded later siblings (e.g. analysis
   * parts rendered under a "Views" header).
   */
  sectionHeaderScope?: 'root' | 'all' | 'none';
}

function ModelTreeViewImpl({
  tree,
  expandedSet,
  onToggleExpand,
  focusedId = null,
  onSelectNode,
  pinnedIds,
  onTogglePin,
  editable,
  onEditAttribute,
  getSparklineSamples,
  onSparklineClick,
  onContextMenu,
  onLaunchRunnable,
  viewsByElementId,
  selectedViewId,
  onPickView,
  previewUri,
  onPromotePreview,
  testIdPrefix = 'model-tree',
  sectionHeaderScope = 'all',
}: ModelTreeViewProps) {
  const renderNodes = (
    nodes: readonly ModelTreeNode[],
    depth: number,
  ): ReactElement[] => {
    const rows: ReactElement[] = [];
    // Decide whether to inject section headers for this sibling
    // group. Only groups that mix ≥2 archetype buckets get labels —
    // a homogeneous container (e.g. all attributes on a leaf) stays
    // quiet. Virtual `section` children (the Outputs / Parameters
    // split under AttributeUsages) already carry their own labels
    // so we never wrap those in another layer.
    const distinctKinds = distinctArchetypeKinds(nodes);
    // `'root'` scope confines headers to the top-level (depth 0) group;
    // nested container contents rely on indentation + the per-row type
    // icon. `'all'` (default) labels every mixed-kind group at any depth.
    // `'none'` suppresses them entirely (package + ownership view).
    const scopeAllowsHeaders =
      sectionHeaderScope === 'all' ||
      (sectionHeaderScope === 'root' && depth === 0);
    const labelsOn =
      scopeAllowsHeaders &&
      distinctKinds.size >= 2 &&
      !distinctKinds.has('section');
    let lastKind: ModelTreeNodeKind | null = null;

    for (const node of nodes) {
      if (labelsOn && node.kind !== lastKind) {
        const label = ARCHETYPE_LABELS[node.kind];
        if (label) {
          rows.push(
            <SectionHeader
              key={`${testIdPrefix}-section-${node.id}`}
              label={label}
              depth={depth}
              testId={`${testIdPrefix}-section-${node.kind}-${node.id}`}
            />,
          );
        }
      }
      lastKind = node.kind;

      const expanded = expandedSet.has(node.id);
      const samples = getSparklineSamples?.(node);
      const availableViews = viewsByElementId?.get(node.elementId);
      const hasVirtualChildren = !!availableViews && availableViews.length > 0;
      rows.push(
        <ModelTreeNodeRow
          key={node.id}
          node={node}
          expanded={expanded}
          onToggleExpand={() => onToggleExpand(node.id)}
          selected={focusedId === node.id}
          onSelect={onSelectNode ? () => onSelectNode(node) : undefined}
          pinned={pinnedIds?.has(node.id) ?? false}
          onTogglePin={onTogglePin ? () => onTogglePin(node) : undefined}
          editable={editable}
          onEdit={
            onEditAttribute ? () => onEditAttribute(node) : undefined
          }
          sparklineSamples={samples}
          onSparklineClick={
            onSparklineClick
              ? (fullName) => onSparklineClick(fullName, node)
              : undefined
          }
          onContextMenu={
            onContextMenu
              ? (pos) => onContextMenu(node, pos)
              : undefined
          }
          onLaunchRunnable={onLaunchRunnable}
          hasVirtualChildren={hasVirtualChildren}
          viewsCount={availableViews?.length ?? 0}
          previewUri={previewUri ?? null}
          onPromotePreview={onPromotePreview}
          testIdPrefix={`${testIdPrefix}-node`}
        />,
      );
      if (expanded && node.children.length > 0) {
        rows.push(...renderNodes(node.children, depth + 1));
      }
      // Bucket 5-followup-3 (2026-05-05): inline view children. When
      // the node is expanded and authored views expose this element,
      // render them as virtual children at depth+1. Click selects the
      // view via `onPickView` (drives the diagram). The "Views"
      // section header only appears when the node also has real
      // children — otherwise the view rows are unambiguous.
      if (expanded && hasVirtualChildren && availableViews) {
        if (node.children.length > 0) {
          rows.push(
            <SectionHeader
              key={`${testIdPrefix}-views-section-${node.id}`}
              label="Views"
              depth={depth + 1}
              testId={`${testIdPrefix}-views-section-${node.id}`}
            />,
          );
        }
        for (const v of availableViews) {
          const viewRowId = `${node.id}__view__${v.id}`;
          rows.push(
            <ViewChildRow
              key={viewRowId}
              viewId={v.id}
              name={v.name}
              kind={v.kind}
              depth={depth + 1}
              selected={selectedViewId === v.id}
              onClick={onPickView ? () => onPickView(v.id) : undefined}
              testId={`${testIdPrefix}-node-${viewRowId}`}
            />,
          );
        }
      }
    }
    return rows;
  };

  // Built fresh every render (matches prior behaviour exactly) — the
  // choice of HOW to mount this flat array (plain vs. windowed) is
  // decided below, purely by its length.
  const allRows = tree.length === 0 ? [] : renderNodes(tree, 0);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const shouldVirtualize = allRows.length > ROW_VIRTUALIZATION_THRESHOLD;

  return (
    <div
      ref={scrollRef}
      role="tree"
      /* The bare prefix belongs HERE: this scroller IS the tree, and it is
         what every consumer means by `model-tree` / `session-tree-v2` (the
         per-row testids are `${prefix}-node-…` children of it). The duplicate
         of finding 16 was resolved at the other end — `SessionTreeV2`'s outer
         panel, which wraps a header + filters + this tree, no longer claims
         the tree's id and is `session-tree-v2-panel`. */
      data-testid={testIdPrefix}
      data-empty={tree.length === 0 || undefined}
      data-virtualized={shouldVirtualize || undefined}
      className="flex flex-col h-full overflow-y-auto"
    >
      {tree.length === 0 ? (
        <div
          data-testid={`${testIdPrefix}-empty`}
          className="flex flex-col items-center justify-center gap-2 p-4"
          /* `--outline` is a BORDER token (n-400); used as text it measured
             2.88:1 here. This is live empty-state copy telling you what to do
             next, not disabled chrome, so it takes the muted TEXT tier. */
          style={{ color: 'var(--text-muted)' }}
        >
          <span
            className="material-symbols-outlined"
            style={{ fontSize: 24, opacity: 0.8 }}
          >
            account_tree
          </span>
          <span style={{ fontSize: 11, textAlign: 'center' }}>
            No model loaded. Load a workspace to see its tree here.
          </span>
        </div>
      ) : shouldVirtualize ? (
        <VirtualizedRows rows={allRows} scrollRef={scrollRef} testIdPrefix={testIdPrefix} />
      ) : (
        allRows
      )}
    </div>
  );
}

/**
 * Windowed row mount for big trees (UX closeout #4 / #17). Only the
 * rows in (or near) the scroll viewport actually mount — everything
 * else is represented purely by the track's total height so the
 * scrollbar still reflects the true row count.
 */
function VirtualizedRows({
  rows,
  scrollRef,
  testIdPrefix,
}: {
  rows: ReactElement[];
  scrollRef: RefObject<HTMLDivElement | null>;
  testIdPrefix: string;
}) {
  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_HEIGHT_PX,
    overscan: 12,
    // A deterministic starting guess so this renders sensibly both
    // before the real browser layout settles AND in jsdom (which has
    // no ResizeObserver — ResizeObserver-driven remeasurement no-ops
    // there, leaving this value authoritative for the whole test).
    initialRect: { width: 0, height: 480 },
    getItemKey: (index) => rows[index]?.key ?? index,
  });
  const virtualItems = rowVirtualizer.getVirtualItems();

  return (
    <div
      data-testid={`${testIdPrefix}-virtual-track`}
      style={{ position: 'relative', width: '100%', height: rowVirtualizer.getTotalSize() }}
    >
      {virtualItems.map((virtualItem) => (
        <div
          key={virtualItem.key}
          data-index={virtualItem.index}
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            transform: `translateY(${virtualItem.start}px)`,
          }}
        >
          {rows[virtualItem.index]}
        </div>
      ))}
    </div>
  );
}

// ─── Section headers ──────────────────────────────────────────────────

/**
 * Uppercase-small labels painted between archetype buckets under a
 * single parent. Intentionally not `role="treeitem"` — these are
 * visual grouping only, they don't participate in keyboard nav or
 * selection and `collectByKind` / `walkModelTree` don't see them.
 * `section` (the Outputs/Parameters split) is already handled by
 * its own node type and never emits a header from here.
 */
const ARCHETYPE_LABELS: Partial<Record<ModelTreeNodeKind, string>> = {
  part: 'Parts',
  port: 'Ports',
  sm: 'State machines',
  action: 'Actions',
  case: 'Cases',
  constraint: 'Constraints',
  ode: 'ODEs',
  calc: 'Calculations',
  attribute: 'Attributes',
  connection: 'Connections',
  other: 'Other',
};

function distinctArchetypeKinds(
  nodes: readonly ModelTreeNode[],
): Set<ModelTreeNodeKind> {
  const set = new Set<ModelTreeNodeKind>();
  for (const n of nodes) set.add(n.kind);
  return set;
}

function SectionHeader({
  label,
  depth,
  testId,
}: {
  label: string;
  depth: number;
  testId: string;
}) {
  return (
    <div
      aria-hidden
      data-testid={testId}
      className="flex items-center select-none"
      style={{
        paddingLeft: 4 + depth * 12 + 14,
        paddingRight: 8,
        minHeight: 16,
        fontSize: 9,
        fontWeight: 700,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: 'var(--outline)',
        marginTop: 2,
      }}
    >
      {label}
    </div>
  );
}

/**
 * Synthetic tree row for an authored view exposing a model element.
 * Bucket 5-followup-3 — replaces the per-row "views (N)" chip + popover
 * with inline children, which is how users naturally read the tree.
 */
function ViewChildRow({
  viewId,
  name,
  kind,
  depth,
  selected,
  onClick,
  testId,
}: {
  viewId: string;
  name: string | null;
  kind: string;
  depth: number;
  selected: boolean;
  onClick?: () => void;
  testId: string;
}) {
  const stereotype = kind.endsWith('Definition') ? 'view def' : 'view';
  return (
    <div
      role="treeitem"
      aria-selected={selected || undefined}
      data-testid={testId}
      data-view-id={viewId}
      data-selected={selected || undefined}
      className="flex items-center gap-1.5 py-[3px] cursor-pointer select-none"
      style={{
        paddingLeft: 4 + depth * 12 + 14,
        paddingRight: 8,
        minHeight: 22,
        background: selected
          ? 'color-mix(in srgb, var(--primary) 14%, transparent)'
          : 'transparent',
        borderLeft: selected
          ? '2px solid var(--primary)'
          : '2px solid transparent',
        fontSize: 'var(--text-xs)',
      }}
      onClick={(e) => {
        e.stopPropagation();
        onClick?.();
      }}
      title={`Render ${stereotype} ${name ?? '(unnamed)'}`}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{
          fontSize: 12,
          color: selected ? 'var(--primary)' : 'var(--outline)',
          flexShrink: 0,
        }}
      >
        visibility
      </span>
      <span
        style={{
          fontSize: 9,
          color: 'var(--outline)',
          fontStyle: 'italic',
          marginRight: 2,
          flexShrink: 0,
        }}
      >
        {stereotype}
      </span>
      <span
        className="mono-text truncate"
        style={{
          color: selected ? 'var(--primary)' : 'var(--on-surface)',
          fontWeight: selected ? 600 : 400,
        }}
      >
        {name ?? <em>(unnamed)</em>}
      </span>
    </div>
  );
}

export const ModelTreeView = memo(ModelTreeViewImpl);
