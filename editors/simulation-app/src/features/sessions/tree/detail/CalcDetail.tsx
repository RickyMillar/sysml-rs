import { formatVariableValue } from '@/features/variables/VariableTree';
import type { CalcTreeNode } from '../types';
import { ComingSoon, DetailMeta, DetailShell } from './common';

export function CalcDetail({
  node,
  testIdPrefix,
}: {
  node: CalcTreeNode;
  testIdPrefix: string;
}) {
  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="calc">
      <DetailMeta node={node} />
      <div
        className="mono-text"
        style={{
          fontSize: 22,
          color: 'var(--on-surface)',
          fontWeight: 600,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {node.value === undefined
          ? '—'
          : formatVariableValue(node.value ?? null, node.unit)}
      </div>
      <ComingSoon
        message="Expression source + operand overlay piggyback on the same work as ConstraintDetail (Task #144)."
      />
    </DetailShell>
  );
}
