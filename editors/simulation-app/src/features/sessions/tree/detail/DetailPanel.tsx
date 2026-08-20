/**
 * DetailPanel — the bottom pane of Zone 1's two-stack split.
 *
 * Observes `focusPath` from the session store, resolves it against
 * the current tree, and dispatches to a per-kind detail component.
 * Each per-kind subcomponent lives in its own file under `detail/`
 * so AttributeDetail / ConstraintDetail / SmDetail / OdeDetail /
 * CalcDetail / PartDetail can evolve independently.
 *
 * Round 2 Task #142 ships the scaffold: real content lands in
 * tasks #143 (AttributeDetail), #144 (ConstraintDetail), and
 * follow-ups for the remaining archetypes.
 */
import { useSessionStore } from '../../store';
import { resolveFocusPath } from '../buildModelTree';
import type { ModelTreeNode } from '../types';
import { useSessionModelTree } from '../useSessionModelTree';
import { ActionDetail } from './ActionDetail';
import { AttributeDetail } from './AttributeDetail';
import { CalcDetail } from './CalcDetail';
import { ConstraintDetail } from './ConstraintDetail';
import { EmptyDetail } from './EmptyDetail';
import { OdeDetail } from './OdeDetail';
import { PartDetail } from './PartDetail';
import { SectionDetail } from './SectionDetail';
import { SmDetail } from './SmDetail';
import { OtherDetail } from './OtherDetail';

export interface DetailPanelProps {
  /** data-testid prefix (default `tree-detail`). */
  testIdPrefix?: string;
}

export function DetailPanel({
  testIdPrefix = 'tree-detail',
}: DetailPanelProps) {
  const focusPath = useSessionStore((s) => s.focusPath);
  const { tree } = useSessionModelTree();

  const chain = resolveFocusPath(tree, focusPath);
  const focused = chain.length > 0 ? chain[chain.length - 1] : null;

  return (
    <div
      data-testid={testIdPrefix}
      data-focused-id={focused?.id}
      data-focused-kind={focused?.kind}
      className="flex flex-col h-full overflow-hidden"
      style={{
        background: 'var(--surface-container-low)',
        borderTop: '1px solid var(--outline-variant)',
      }}
    >
      <Header chain={chain} focused={focused} testIdPrefix={testIdPrefix} />
      <div
        className="flex-1 overflow-y-auto min-h-0"
        data-testid={`${testIdPrefix}-body`}
      >
        <Dispatch node={focused} testIdPrefix={testIdPrefix} />
      </div>
    </div>
  );
}

function Header({
  chain,
  focused,
  testIdPrefix,
}: {
  chain: readonly ModelTreeNode[];
  focused: ModelTreeNode | null;
  testIdPrefix: string;
}) {
  return (
    <div
      className="flex items-center gap-2 px-3 shrink-0"
      style={{
        height: 26,
        borderBottom: '1px solid var(--outline-variant)',
        fontSize: 9,
        fontWeight: 600,
        letterSpacing: '0.1em',
        textTransform: 'uppercase',
        color: 'var(--outline)',
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
        read_more
      </span>
      <span>Detail</span>
      {focused && (
        <span
          className="mono-text truncate"
          data-testid={`${testIdPrefix}-header-name`}
          style={{
            color: 'var(--on-surface)',
            fontSize: 11,
            fontWeight: 500,
            letterSpacing: 0,
            textTransform: 'none',
            marginLeft: 4,
            flex: '0 1 auto',
            minWidth: 0,
          }}
          title={chain.map((n) => n.name).join(' › ')}
        >
          {focused.name}
        </span>
      )}
      {focused && (
        <span
          data-testid={`${testIdPrefix}-header-kind`}
          className="mono-text"
          style={{
            marginLeft: 'auto',
            fontSize: 9,
            color: 'var(--outline)',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            flexShrink: 0,
          }}
        >
          {focused.kind}
        </span>
      )}
    </div>
  );
}

/**
 * Routes the focused node to its kind-specific detail. Exported
 * for the test harness so the dispatch logic can be exercised
 * without mounting the header / scroll container.
 */
export function Dispatch({
  node,
  testIdPrefix,
}: {
  node: ModelTreeNode | null;
  testIdPrefix: string;
}) {
  if (!node) return <EmptyDetail testIdPrefix={testIdPrefix} />;
  switch (node.kind) {
    case 'attribute':
      return <AttributeDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'constraint':
      return <ConstraintDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'sm':
      return <SmDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'ode':
      return <OdeDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'calc':
      return <CalcDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'part':
      return <PartDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'port':
    case 'connection':
      // These archetypes don't have dedicated detail panels yet; fall
      // through to the generic Other panel so users still see something
      // on click.
      return <OtherDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'action':
      return <ActionDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'section':
      return <SectionDetail node={node} testIdPrefix={testIdPrefix} />;
    case 'other':
      return <OtherDetail node={node} testIdPrefix={testIdPrefix} />;
  }
}
