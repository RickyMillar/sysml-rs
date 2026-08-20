/**
 * ModelTreeNodeRow — one-row renderer for a `ModelTreeNode`.
 *
 * Phase B1a: polymorphic per-archetype row used by the (not-yet-built)
 * ModelTreeView container. Children rendering + expand/collapse state
 * + selection state live on the container; this component only paints
 * a single row given what archetype it is.
 *
 * Per archetype:
 *  - part: chevron (leaf/expanded/collapsed), folder-like icon, name
 *    + optional "one-liner" aggregate.
 *  - attribute: delegates to the A3 `<AttributeRow>` so tree rows and
 *    the (future) detail region render identically when they show the
 *    same attribute.
 *  - sm: name + state badge (armed / tripped / …).
 *  - constraint: name + verdict dot.
 *  - ode: name + coarse integrator status (stable / stiff / diverged).
 *  - other: plain name with raw kind suffix.
 */
import { useCallback, useRef, useState } from 'react';
import type { CSSProperties, MouseEvent } from 'react';
import { AttributeRow } from '@/features/variables/AttributeRow';
import type { VariableValue } from '@/features/variables/VariableTree';
import { SourcePreviewPopover } from '@/features/editor/SourcePreviewPopover';
import type {
  AttributeTreeNode,
  CalcTreeNode,
  ConstraintTreeNode,
  ModelTreeNode,
  OdeTreeNode,
  PartTreeNode,
  SectionTreeNode,
  SmTreeNode,
} from './types';

const INDENT_PX = 12;

export interface ModelTreeNodeRowProps {
  node: ModelTreeNode;
  /** Is this node expanded? (Ignored on leaves.) */
  expanded: boolean;
  /** Called when the expansion chevron is clicked. Should be a no-op
   *  for leaves — the container decides. */
  onToggleExpand?: () => void;
  /** Keyboard / click selection highlight. */
  selected?: boolean;
  /** Called when the row body (not the affordances) is clicked. */
  onSelect?: () => void;
  /** Whether this row is pinned (attributes only). */
  pinned?: boolean;
  onTogglePin?: () => void;
  /** Whether the attribute is editable (applies to `attribute` only). */
  editable?: boolean;
  onEdit?: () => void;
  /** Right-click handler with absolute coords. */
  onContextMenu?: (position: { x: number; y: number }) => void;
  /** Optional per-archetype decoration lookups. Container passes these
   *  in pre-resolved so the row doesn't re-query stores. */
  sparklineSamples?: readonly number[];
  /** Click-sparkline handler — called with the full variable name for
   *  attribute / calc rows. Usually bound to `promoteToPlots`. */
  onSparklineClick?: (fullName: string) => void;
  /** Inline launch action for runnable model rows (simulation/cases). */
  onLaunchRunnable?: (node: ModelTreeNode) => void;
  /**
   * The node has children that aren't carried on `node.children` —
   * e.g. authored views exposing this element, rendered as synthetic
   * children by the container. When true, the chevron renders even
   * if `node.children.length === 0`.
   */
  hasVirtualChildren?: boolean;
  /**
   * Number of authored views exposing this node's element. When > 0,
   * the row paints a small `👁 N` badge next to the name so users can
   * scan the tree for view-bearing rows without expanding everything.
   */
  viewsCount?: number;
  /**
   * Phase 3 — URI to preview against on row hover. When set together
   * with a node that carries an `elementId`, the row gets a
   * source-preview popover. Pass `null` to disable.
   */
  previewUri?: string | null;
  /**
   * Phase 3 — called when the user clicks the source-preview popover
   * (not the row itself). Container wires this to push selection +
   * focusedUri + open Source utility.
   */
  onPromotePreview?: (node: ModelTreeNode) => void;
  /** data-testid prefix — default `model-tree-node`. */
  testIdPrefix?: string;
}

export function ModelTreeNodeRow(props: ModelTreeNodeRowProps) {
  const { node, testIdPrefix = 'model-tree-node' } = props;

  // Attributes are special: they reuse the A3 AttributeRow component.
  if (node.kind === 'attribute') {
    return <AttributeRowCase {...props} node={node} />;
  }

  // Plain calculations render identically to attributes — value,
  // unit, sparkline, pin, edit. The only difference vs 'ode' is
  // that plain calcs don't show an integrator-status chip.
  if (node.kind === 'calc') {
    return <CalcRowCase {...props} node={node} />;
  }

  // Section headers — virtual Outputs / Parameters grouping rows.
  if (node.kind === 'section') {
    return <SectionRowCase {...props} node={node} />;
  }

  const depth = node.depth;
  const indent = depth * INDENT_PX;
  const { selected = false, onSelect, onContextMenu, previewUri, onPromotePreview } = props;
  const [hovered, setHovered] = useState(false);
  const rowRef = useRef<HTMLDivElement | null>(null);
  const handlePromote = useCallback(() => {
    onPromotePreview?.(node);
  }, [node, onPromotePreview]);

  const rowStyle: CSSProperties = {
    paddingLeft: 4 + indent,
    paddingRight: 8,
    minHeight: 'var(--row-dense)',
    background: selected
      ? 'color-mix(in srgb, var(--accent) 14%, transparent)'
      : 'transparent',
    borderLeft: selected ? '2px solid var(--accent)' : '2px solid transparent',
    fontSize: 'var(--text-xs)',
    contentVisibility: 'auto',
    containIntrinsicSize: 'auto var(--row-dense)',
  };

  const isDefinition = node.rawKind.endsWith('Definition');
  const hasAnyChildren = node.children.length > 0 || !!props.hasVirtualChildren;

  return (
    <>
    <div
      ref={rowRef}
      role="treeitem"
      aria-expanded={hasAnyChildren ? props.expanded : undefined}
      aria-selected={selected || undefined}
      data-testid={`${testIdPrefix}-${node.id}`}
      data-kind={node.kind}
      data-raw-kind={node.rawKind}
      data-sysml-variant={isDefinition ? 'definition' : 'usage'}
      data-depth={depth}
      data-selected={selected || undefined}
      className="flex items-center gap-1.5 py-[3px] cursor-pointer select-none transition-colors"
      style={rowStyle}
      onClick={onSelect}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
      onContextMenu={(e: MouseEvent<HTMLDivElement>) => {
        if (!onContextMenu) return;
        e.preventDefault();
        onContextMenu({ x: e.clientX, y: e.clientY });
      }}
      title={`${node.name} (${node.rawKind})`}
    >
      <ChevronOrSpacer
        hasChildren={hasAnyChildren}
        expanded={props.expanded}
        onToggle={props.onToggleExpand}
        testId={`${testIdPrefix}-${node.id}-chevron`}
      />
      <KindIcon kind={node.kind} rawKind={node.rawKind} />
      <span
        className="mono-text truncate"
        style={{
          // Definitions render italic + slightly muted so users can
          // visually distinguish class rows from instance rows in
          // `both` mode. `usages` mode won't paint any def rows.
          color: isDefinition ? 'var(--text-secondary)' : 'var(--text-primary)',
          fontStyle: isDefinition ? 'italic' : 'normal',
          fontWeight: node.kind === 'part' ? 600 : 400,
          flex: '0 1 auto',
          minWidth: 0,
        }}
        data-testid={`${testIdPrefix}-${node.id}-label`}
      >
        {node.name}
      </span>
      <ViewsCountBadge
        count={props.viewsCount ?? 0}
        testId={`${testIdPrefix}-${node.id}-views-count`}
      />
      <RunnableActionButton
        node={node}
        onLaunch={props.onLaunchRunnable}
        testId={`${testIdPrefix}-${node.id}-launch`}
      />
      <DecorationForKind
        node={node}
        testIdPrefix={`${testIdPrefix}-${node.id}`}
      />
    </div>
    <SourcePreviewPopover
      triggerRef={rowRef}
      triggerHovered={hovered}
      uri={previewUri ?? null}
      elementId={node.elementId ?? null}
      onPromote={onPromotePreview ? handlePromote : undefined}
      testId={`${testIdPrefix}-${node.id}-preview`}
    />
    </>
  );
}

// ── Per-kind decorations ────────────────────────────────────────────

type RunnableLaunchKind = 'simulation' | 'analysis' | 'verification';

function runnableKind(node: ModelTreeNode): RunnableLaunchKind | null {
  switch (node.rawKind) {
    case 'StateDefinition':
    case 'StateUsage':
    case 'ExhibitStateUsage':
      return 'simulation';
    case 'AnalysisCaseUsage':
    case 'AnalysisCaseDefinition':
      return 'analysis';
    case 'VerificationCaseUsage':
    case 'VerificationCaseDefinition':
      return 'verification';
    default:
      return null;
  }
}

function RunnableActionButton({
  node,
  onLaunch,
  testId,
}: {
  node: ModelTreeNode;
  onLaunch?: (node: ModelTreeNode) => void;
  testId: string;
}) {
  const kind = runnableKind(node);
  if (!kind || !onLaunch) return null;
  const label = kind === 'simulation' ? 'Run' : kind === 'analysis' ? 'Analyze' : 'Verify';
  const icon = kind === 'simulation' ? 'play_arrow' : kind === 'analysis' ? 'analytics' : 'verified';
  return (
    <button
      type="button"
      data-testid={testId}
      className="inline-flex items-center gap-0.5 ml-auto rounded mono-text"
      style={{
        border: '1px solid var(--border-default)',
        background: 'var(--surface-panel)',
        color: 'var(--accent-fg)',
        fontSize: 9,
        padding: '1px 5px',
        cursor: 'pointer',
      }}
      title={`${label} ${node.name}`}
      onClick={(event) => {
        event.stopPropagation();
        onLaunch(node);
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 11 }}>{icon}</span>
      {label}
    </button>
  );
}

function DecorationForKind({
  node,
  testIdPrefix,
}: {
  // Attribute / calc / section rows take early-return paths in
  // ModelTreeNodeRow, so the decoration helper never receives them.
  node: Exclude<
    ModelTreeNode,
    AttributeTreeNode | CalcTreeNode | SectionTreeNode
  >;
  testIdPrefix: string;
}) {
  switch (node.kind) {
    case 'part':
      return <PartDecoration node={node} testIdPrefix={testIdPrefix} />;
    case 'sm':
      return <SmDecoration node={node} testIdPrefix={testIdPrefix} />;
    case 'constraint':
      return (
        <ConstraintDecoration node={node} testIdPrefix={testIdPrefix} />
      );
    case 'ode':
      return <OdeDecoration node={node} testIdPrefix={testIdPrefix} />;
    case 'other':
      return (
        <span
          data-testid={`${testIdPrefix}-rawkind`}
          className="mono-text ml-auto"
          style={{ fontSize: 9, color: 'var(--text-muted)' }}
        >
          {node.rawKind}
        </span>
      );
  }
}

/**
 * Inline indicator: "this row's element is exposed by N authored views".
 * Bucket 5-followup-3 (2026-05-05) — lets users scan the tree for
 * view-bearing rows without expanding everything. Hidden when count = 0.
 */
function ViewsCountBadge({ count, testId }: { count: number; testId: string }) {
  if (count <= 0) return null;
  return (
    <span
      data-testid={testId}
      className="inline-flex items-center gap-0.5 mono-text"
      style={{
        fontSize: 9,
        color: 'var(--text-secondary)',
        background: 'color-mix(in srgb, var(--text-secondary) 10%, transparent)',
        border: '1px solid color-mix(in srgb, var(--text-secondary) 25%, transparent)',
        borderRadius: 3,
        padding: '0 4px',
        flexShrink: 0,
        letterSpacing: 0.2,
      }}
      title={`${count} authored view${count === 1 ? '' : 's'} expose this element`}
      aria-label={`${count} views`}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{ fontSize: 10 }}
      >
        visibility
      </span>
      {count}
    </span>
  );
}

function PartDecoration({
  node,
  testIdPrefix,
}: {
  node: PartTreeNode;
  testIdPrefix: string;
}) {
  if (!node.oneLiner) return null;
  return (
    <span
      data-testid={`${testIdPrefix}-oneliner`}
      className="mono-text ml-auto"
      style={{ fontSize: 10, color: 'var(--text-muted)' }}
    >
      {node.oneLiner}
    </span>
  );
}

function SmDecoration({
  node,
  testIdPrefix,
}: {
  node: SmTreeNode;
  testIdPrefix: string;
}) {
  if (!node.currentState) {
    return (
      <span
        data-testid={`${testIdPrefix}-state`}
        className="mono-text ml-auto"
        style={{ fontSize: 10, color: 'var(--text-muted)' }}
      >
        —
      </span>
    );
  }
  return (
    <span
      data-testid={`${testIdPrefix}-state`}
      className="mono-text ml-auto"
      style={{
        fontSize: 10,
        color: 'var(--text-primary)',
        background: 'var(--surface-panel)',
        padding: '1px 6px',
        borderRadius: 3,
      }}
    >
      {node.currentState}
    </span>
  );
}

function ConstraintDecoration({
  node,
  testIdPrefix,
}: {
  node: ConstraintTreeNode;
  testIdPrefix: string;
}) {
  const color =
    node.verdict === 'pass'
      ? 'var(--verdict-pass)'
      : node.verdict === 'fail'
        ? 'var(--verdict-fail)'
        : node.verdict === 'inconclusive'
          ? 'var(--verdict-inconclusive)'
          : node.verdict === 'error'
            ? 'var(--verdict-error)'
            : 'var(--border-default)';
  return (
    <span
      data-testid={`${testIdPrefix}-verdict`}
      data-verdict={node.verdict ?? 'none'}
      className="ml-auto"
      style={{
        width: 8,
        height: 8,
        borderRadius: '50%',
        background: color,
        display: 'inline-block',
        flexShrink: 0,
      }}
    />
  );
}

function OdeDecoration({
  node,
  testIdPrefix,
}: {
  node: OdeTreeNode;
  testIdPrefix: string;
}) {
  const status = node.status ?? 'unknown';
  const color =
    status === 'stable'
      ? 'var(--health-nominal)'
      : status === 'stiff'
        ? 'var(--health-warning)'
        : status === 'diverged'
          ? 'var(--health-critical)'
          : 'var(--text-muted)';
  return (
    <span
      data-testid={`${testIdPrefix}-status`}
      data-status={status}
      className="mono-text ml-auto"
      style={{ fontSize: 10, color }}
    >
      {status}
    </span>
  );
}

// ── Shared bits ──────────────────────────────────────────────────────

function ChevronOrSpacer({
  hasChildren,
  expanded,
  onToggle,
  testId,
}: {
  hasChildren: boolean;
  expanded: boolean;
  onToggle?: () => void;
  testId: string;
}) {
  if (!hasChildren) {
    return (
      <span style={{ width: 12, display: 'inline-flex', flexShrink: 0 }} />
    );
  }
  return (
    <button
      type="button"
      aria-label={expanded ? 'collapse' : 'expand'}
      data-testid={testId}
      onClick={(e) => {
        e.stopPropagation();
        onToggle?.();
      }}
      style={{
        border: 'none',
        background: 'transparent',
        padding: 0,
        width: 12,
        cursor: 'pointer',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
      }}
    >
      <span
        className="material-symbols-outlined"
        style={{
          fontSize: 12,
          color: 'var(--text-muted)',
          transform: expanded ? 'rotate(90deg)' : undefined,
          transition: 'transform 0.12s',
        }}
      >
        chevron_right
      </span>
    </button>
  );
}

function KindIcon({
  kind,
  rawKind,
}: {
  kind: ModelTreeNode['kind'];
  rawKind?: string;
}) {
  // Virtual package containers (rawKind='Package' set by
  // `useSessionModelTree` when grouping is on) render with a folder
  // icon so they read as containers rather than as authored parts.
  if (rawKind === 'Package') {
    return (
      <span
        className="material-symbols-outlined"
        style={{ fontSize: 13, color: 'var(--text-muted)', flexShrink: 0 }}
      >
        folder
      </span>
    );
  }
  const icon =
    kind === 'part'
      ? 'category'
      : kind === 'sm'
        ? 'swap_horiz'
        : kind === 'constraint'
          ? 'rule'
          : kind === 'ode'
            ? 'show_chart'
            : 'circle';
  // Glyph tokens, not chart-series: a data-series fill and an icon colour are
  // different jobs, and only the icon has a contrast floor to meet. On the
  // series ramp these measured 2.86:1 against the tree panel (finding 44).
  const color =
    kind === 'part'
      ? 'var(--glyph-part)'
      : kind === 'sm'
        ? 'var(--glyph-state)'
        : kind === 'constraint'
          ? 'var(--glyph-constraint)'
          : kind === 'ode'
            ? 'var(--glyph-ode)'
            : 'var(--text-muted)';
  return (
    <span
      className="material-symbols-outlined"
      style={{ fontSize: 13, color, flexShrink: 0 }}
    >
      {icon}
    </span>
  );
}

// ── Calc special case ──────────────────────────────────────────────

function CalcRowCase({
  node,
  selected,
  onSelect,
  pinned,
  onTogglePin,
  editable,
  onEdit,
  onContextMenu,
  sparklineSamples,
  onSparklineClick,
  testIdPrefix = 'model-tree-node',
}: ModelTreeNodeRowProps & { node: CalcTreeNode }) {
  const samples = sparklineSamples ?? [];
  const fullName = node.ownerPath
    ? `${node.ownerPath}.${node.name}`
    : node.name;
  return (
    <AttributeRow
      id={node.id}
      name={node.name}
      value={(node.value ?? null) as VariableValue}
      unit={node.unit}
      verdict={node.verdict}
      sparklineSamples={samples}
      pinned={pinned}
      onTogglePin={onTogglePin}
      editable={editable}
      onEdit={onEdit}
      onClick={onSelect}
      onContextMenu={onContextMenu}
      selected={selected}
      indent={node.depth * INDENT_PX}
      lastChangedTick={node.lastChangedTick}
      onSparklineClick={
        onSparklineClick ? () => onSparklineClick(fullName) : undefined
      }
      testIdPrefix={testIdPrefix}
    />
  );
}

// ── Section special case ────────────────────────────────────────────

function SectionRowCase({
  node,
  expanded,
  onToggleExpand,
  testIdPrefix = 'model-tree-node',
}: ModelTreeNodeRowProps & { node: SectionTreeNode }) {
  const indent = node.depth * INDENT_PX;
  const isOutputs = node.sectionKind === 'outputs';
  return (
    <div
      role="group"
      data-testid={`${testIdPrefix}-${node.id}`}
      data-kind="section"
      data-section-kind={node.sectionKind}
      className="flex items-center gap-1.5 py-[2px] select-none"
      style={{
        paddingLeft: 4 + indent,
        paddingRight: 8,
        minHeight: 'var(--row-dense)',
        color: 'var(--text-muted)',
        fontSize: 9,
        fontWeight: 700,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        // Thin divider separates the section from whatever came before
        // (typically the previous sibling part). A micro-breather so
        // the header feels like a header, not another row.
        borderTop: '1px dashed var(--border-default)',
        marginTop: 2,
      }}
    >
      <ChevronOrSpacer
        hasChildren={node.children.length > 0}
        expanded={expanded}
        onToggle={onToggleExpand}
        testId={`${testIdPrefix}-${node.id}-chevron`}
      />
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          flexShrink: 0,
        }}
      >
        {isOutputs ? 'output' : 'tune'}
      </span>
      <span data-testid={`${testIdPrefix}-${node.id}-label`}>
        {node.name}
      </span>
    </div>
  );
}

// ── Attribute special case ──────────────────────────────────────────

function AttributeRowCase({
  node,
  selected,
  onSelect,
  pinned,
  onTogglePin,
  editable,
  onEdit,
  onContextMenu,
  sparklineSamples,
  onSparklineClick,
  testIdPrefix = 'model-tree-node',
}: ModelTreeNodeRowProps & { node: AttributeTreeNode }) {
  const samples = sparklineSamples ?? [];
  const fullName = node.ownerPath
    ? `${node.ownerPath}.${node.name}`
    : node.name;
  return (
    <AttributeRow
      id={node.id}
      name={node.name}
      value={(node.value ?? null) as VariableValue}
      unit={node.unit}
      verdict={node.verdict}
      sparklineSamples={samples}
      pinned={pinned}
      onTogglePin={onTogglePin}
      editable={editable}
      onEdit={onEdit}
      onClick={onSelect}
      onContextMenu={onContextMenu}
      selected={selected}
      indent={node.depth * INDENT_PX}
      lastChangedTick={node.lastChangedTick}
      onSparklineClick={
        onSparklineClick ? () => onSparklineClick(fullName) : undefined
      }
      testIdPrefix={testIdPrefix}
    />
  );
}
