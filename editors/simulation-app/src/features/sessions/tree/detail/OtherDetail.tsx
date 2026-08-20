import type {
  ActionTreeNode,
  ConnectionTreeNode,
  OtherTreeNode,
  PortTreeNode,
} from '../types';
import { DetailMeta, DetailShell } from './common';

export function OtherDetail({
  node,
  testIdPrefix,
}: {
  // Port / Connection / Action archetypes don't have dedicated detail
  // panels yet — they route through this generic panel.
  node: OtherTreeNode | PortTreeNode | ConnectionTreeNode | ActionTreeNode;
  testIdPrefix: string;
}) {
  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="other">
      <DetailMeta node={node} extra={node.rawKind} />
      <div
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          lineHeight: 1.5,
        }}
      >
        This element kind doesn’t have a custom detail view yet. Live
        values aren’t surfaced for <code>{node.rawKind}</code> today.
      </div>
    </DetailShell>
  );
}
