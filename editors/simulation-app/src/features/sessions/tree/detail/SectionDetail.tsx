import type { SectionTreeNode } from '../types';
import { DetailShell } from './common';

export function SectionDetail({
  node,
  testIdPrefix,
}: {
  node: SectionTreeNode;
  testIdPrefix: string;
}) {
  const label = node.sectionKind === 'outputs' ? 'Outputs' : 'Parameters';
  const hint =
    node.sectionKind === 'outputs'
      ? 'Attributes whose value changed in the last 20 ticks of the current session.'
      : 'Attributes whose value is static, or hasn’t been observed yet this session.';
  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="section">
      <div
        style={{
          fontSize: 14,
          fontWeight: 600,
          color: 'var(--on-surface)',
        }}
      >
        {label} ({node.count})
      </div>
      <div
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          lineHeight: 1.5,
        }}
        data-testid={`${testIdPrefix}-section-hint`}
      >
        {hint}
      </div>
    </DetailShell>
  );
}
