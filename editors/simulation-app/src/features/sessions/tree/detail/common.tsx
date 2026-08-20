/**
 * Shared layout primitives for the per-kind detail components.
 */
import type { ReactNode } from 'react';
import type { ModelTreeNode } from '../types';

export function DetailShell({
  testIdPrefix,
  suffix,
  children,
}: {
  testIdPrefix: string;
  suffix: string;
  children: ReactNode;
}) {
  return (
    <div
      data-testid={`${testIdPrefix}-${suffix}`}
      className="flex flex-col gap-3 p-3"
    >
      {children}
    </div>
  );
}

export function DetailMeta({
  node,
  extra,
}: {
  node: ModelTreeNode;
  extra?: string;
}) {
  return (
    <div
      className="mono-text"
      style={{ fontSize: 10, color: 'var(--outline)' }}
    >
      {node.ownerPath
        ? `${node.ownerPath}.${node.name}`
        : node.name}
      {extra ? ` · ${extra}` : ''}
    </div>
  );
}

export function ComingSoon({
  message,
  gap,
}: {
  message: string;
  gap?: string;
}) {
  return (
    <div
      style={{
        fontSize: 11,
        color: 'var(--outline)',
        padding: '8px 10px',
        border: '1px dashed var(--outline-variant)',
        borderRadius: 4,
        lineHeight: 1.5,
      }}
    >
      <div>{message}</div>
      {gap && (
        <div
          className="mono-text"
          style={{ marginTop: 4, fontSize: 9, color: 'var(--outline-variant)' }}
        >
          Tracked as {gap}.
        </div>
      )}
    </div>
  );
}
