/**
 * ConstraintContextMenu — right-click menu for constraint tree rows.
 *
 * One simulation action plus copy:
 *   - Break on violation — single click, no prompt. Wires to
 *     `sysml.breakpoint.set` with `kind: 'constraint-violation'`
 *     and the constraint's element id. Pauses the session whenever
 *     this constraint evaluates to false.
 *   - Copy name — clipboard.
 *
 * No "Add to chart" because constraints don't carry a numeric
 * series; their verdict pill in the tree row + the operand-value
 * overlay in `ConstraintDetail` cover the live-state UX.
 */
import { useMemo } from 'react';
import type { ConstraintTreeNode } from '../types';
import {
  ContextMenuShell,
  type ContextMenuItem,
} from './ContextMenuShell';

export interface ConstraintContextMenuProps {
  node: ConstraintTreeNode | null;
  position: { x: number; y: number };
  /** Set a constraint-violation breakpoint on this constraint's
   *  underlying element id (NOT the dedupe-rewritten tree-position
   *  id — see `TreeNode.element_id`). */
  onBreakOnViolation: (elementId: string, name: string) => void;
  onCopyName: (name: string) => void;
  onClose: () => void;
}

export function ConstraintContextMenu({
  node,
  position,
  onBreakOnViolation,
  onCopyName,
  onClose,
}: ConstraintContextMenuProps) {
  const items = useMemo<ContextMenuItem[]>(() => {
    if (!node) return [];
    return [
      {
        id: 'break-violation',
        icon: 'flag',
        label: 'Break on violation',
        onClick: () => {
          onBreakOnViolation(node.elementId, node.name);
          onClose();
        },
        accent: 'var(--sim-breakpoint-mark)',
      },
      {
        id: 'copy',
        icon: 'content_copy',
        label: 'Copy name',
        onClick: () => {
          onCopyName(node.name);
          onClose();
        },
        separator: true,
      },
    ];
  }, [node, onBreakOnViolation, onCopyName, onClose]);

  return (
    <ContextMenuShell
      open={!!node}
      header={node?.name ?? ''}
      position={position}
      items={items}
      onClose={onClose}
      testId="constraint-context-menu"
    />
  );
}
