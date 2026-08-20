/**
 * SmContextMenu — right-click menu for state-machine tree rows.
 *
 * Two actions:
 *   1. Break on state entry — prompts the user to pick which state.
 *      When the SM exposes its child states (`SmTreeNode.states`),
 *      one menu item per state breaks on entry to that state.
 *      Wires to `sysml.breakpoint.set` with `kind: 'state-entry'`
 *      and `element_id` from the state descriptor.
 *
 *   2. Inject event — submenu of `availableTransitions` from the
 *      live snapshot. Each item is `{event} → {target_state}` and
 *      dispatches `sysml.sessions.inject`.
 *
 * Pin / Copy reuse the per-row chrome from
 * `VariablesPaneContextMenu` semantically but operate on element
 * ids, so they go through their own callbacks.
 */
import { useMemo } from 'react';
import type { SmTreeNode } from '../types';
import {
  ContextMenuShell,
  type ContextMenuItem,
} from './ContextMenuShell';

export interface SmContextMenuProps {
  /** SM node the user right-clicked. Null hides the menu. */
  node: SmTreeNode | null;
  position: { x: number; y: number };
  /** Set a state-entry breakpoint on the given state id. */
  onBreakOnStateEntry: (stateElementId: string, stateName: string) => void;
  /** Inject the named event into this SM's subsystem. */
  onInjectEvent: (eventName: string) => void;
  /** Copy the SM's name to clipboard. */
  onCopyName: (name: string) => void;
  onClose: () => void;
}

export function SmContextMenu({
  node,
  position,
  onBreakOnStateEntry,
  onInjectEvent,
  onCopyName,
  onClose,
}: SmContextMenuProps) {
  const items = useMemo<ContextMenuItem[]>(() => {
    if (!node) return [];
    const out: ContextMenuItem[] = [];

    // Break on state entry. One row per state child. When the SM
    // doesn't expose states (older tree builds, or SMs whose
    // structure didn't parse), render a single disabled
    // explanatory row so the user understands why the action
    // isn't available.
    const states = node.states ?? [];
    if (states.length === 0) {
      out.push({
        id: 'bp-no-states',
        icon: 'flag',
        label: 'Break on state entry — no states',
        onClick: () => {},
        accent: 'var(--sim-breakpoint-mark)',
        disabled: true,
      });
    } else {
      out.push(
        ...states.map((state, i) => ({
          id: `bp-state-${state.id}`,
          icon: 'flag',
          label: i === 0 ? `Break on entry: ${state.name}` : `      ${state.name}`,
          onClick: () => {
            onBreakOnStateEntry(state.id, state.name);
            onClose();
          },
          accent: i === 0 ? 'var(--sim-breakpoint-mark)' : undefined,
          // Highlight the active state so users see "I'm here right now".
          trailing: state.name === node.currentState ? '● live' : undefined,
        })),
      );
    }

    // Inject event submenu — driven by live availableTransitions.
    const transitions = node.availableTransitions ?? [];
    out.push({
      id: 'inject-header',
      icon: 'bolt',
      label: transitions.length === 0 ? 'Inject event — none ready' : 'Inject event:',
      onClick: () => {},
      // Header for the list built from node.availableTransitions — a
      // strong semantic match for sim-state-available (currently
      // reachable transitions), not selection/accent and not a
      // generic warning.
      accent: 'var(--sim-state-available)',
      separator: true,
      disabled: true,
    });
    for (const [event, target] of transitions) {
      out.push({
        id: `inject-${event}`,
        icon: 'arrow_forward',
        label: `      ${event}`,
        trailing: `→ ${target}`,
        onClick: () => {
          onInjectEvent(event);
          onClose();
        },
      });
    }

    out.push({
      id: 'copy',
      icon: 'content_copy',
      label: 'Copy name',
      onClick: () => {
        onCopyName(node.name);
        onClose();
      },
      separator: true,
    });

    return out;
  }, [node, onBreakOnStateEntry, onInjectEvent, onCopyName, onClose]);

  return (
    <ContextMenuShell
      open={!!node}
      header={node?.name ?? ''}
      position={position}
      items={items}
      onClose={onClose}
      testId="sm-context-menu"
      width={260}
    />
  );
}
