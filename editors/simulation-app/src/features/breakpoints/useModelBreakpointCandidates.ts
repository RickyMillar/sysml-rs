/**
 * useModelBreakpointCandidates — BP-UX1 (plan Phase 3).
 *
 * Sources Add-breakpoint autocomplete candidates from the COMPILED
 * MODEL, not the live snapshot. The old wiring pulled variables from
 * `metricRegistry` (populated only by `useTimeSeriesIngest` *during* a
 * run — empty on an idle/never-stepped session) and hardcoded element
 * candidates to `[]`, so the user free-typed a name the backend then
 * silently no-op'd when it didn't match the model. Model-sourced
 * candidates exist BEFORE any session — the natural debugger flow is
 * pick target → set breakpoints → run (BP-UX2).
 *
 * Sources:
 *  - the workspace model tree (same session-free read the Browse floor
 *    uses — `useSessionModelTree({ expectedSessionId: null })`):
 *      attribute / calc nodes → variable candidates (dotted owner path,
 *        the same naming `scalar_vars` uses);
 *      sm / action / constraint / ode nodes → element candidates
 *        (bare name; the backend matches elements by name).
 *  - `metricRegistry` names still SUPPLEMENT the variable list once a
 *    session has produced data (plan: "`sessions.timeseries_names` may
 *    supplement after a session has data, but the model is the source
 *    of truth").
 *
 * Both lists are deduped + sorted; the dialog input stays free-form
 * either way.
 */
import { useMemo } from 'react';
import { useSessionModelTree } from '@/features/sessions/tree/useSessionModelTree';
import type { ModelTreeNode } from '@/features/sessions/tree/types';
import { metricRegistry } from '@/shared/metrics/registry';
import type { FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';

const ELEMENT_KINDS = new Set(['sm', 'action', 'constraint', 'ode']);
const VARIABLE_KINDS = new Set(['attribute', 'calc']);
/** State rows classify as archetype 'other' — detect them by rawKind. */
const STATE_RAW_KINDS = new Set(['StateUsage', 'ExhibitStateUsage', 'StateDefinition']);

/** Human labels for the detail column (`<what> · <where>`). */
const KIND_LABEL: Record<string, string> = {
  sm: 'state machine',
  action: 'action',
  constraint: 'constraint',
  ode: 'ode',
};

function walk(
  nodes: readonly ModelTreeNode[],
  variables: Set<string>,
  elements: Map<string, string | undefined>, // value → detail
  /** Name of the nearest enclosing state machine, if any. */
  enclosingSm: string | null,
): void {
  for (const node of nodes) {
    if (VARIABLE_KINDS.has(node.kind)) {
      variables.add(node.ownerPath ? `${node.ownerPath}.${node.name}` : node.name);
    } else if (STATE_RAW_KINDS.has(node.rawKind)) {
      // Individual states — the detail names the OWNING MACHINE so the
      // list reads `armed  ·  state · BreakerStates`, and typing the
      // machine's name fuzzy-matches its states (detail is searchable).
      const where = enclosingSm ?? node.ownerPath.split('.').pop() ?? '';
      elements.set(node.name, where ? `state · ${where}` : 'state');
      if (node.ownerPath) {
        elements.set(`${node.ownerPath}.${node.name}`, where ? `state · ${where}` : 'state');
      }
    } else if (ELEMENT_KINDS.has(node.kind)) {
      // Elements resolve by NAME against the compiled model (all 5
      // element-targeting kinds); offer the bare name, plus the dotted
      // path when an owner exists — nested-scope models disambiguate
      // with the path form.
      const label = KIND_LABEL[node.kind] ?? node.kind;
      const detail = node.ownerPath ? `${label} · ${node.ownerPath}` : label;
      elements.set(node.name, detail);
      if (node.ownerPath) elements.set(`${node.ownerPath}.${node.name}`, detail);
    }
    if (node.children.length > 0) {
      walk(node.children, variables, elements, node.kind === 'sm' ? node.name : enclosingSm);
    }
  }
}

export interface ModelBreakpointCandidates {
  variableCandidates: string[];
  elementCandidates: FuzzyCandidate[];
}

export function useModelBreakpointCandidates(): ModelBreakpointCandidates {
  // Session-free model read — candidates must exist on a fresh, idle,
  // never-stepped workspace (BP-UX2 arm-before-run).
  const { tree } = useSessionModelTree({
    groupByPackage: false,
    expectedSessionId: null,
  });

  return useMemo(() => {
    const variables = new Set<string>();
    const elements = new Map<string, string | undefined>();
    walk(tree, variables, elements, null);
    // Live supplement: names the metric registry has actually seen
    // stream (post-run only; empty before the first snapshot).
    for (const m of metricRegistry.list()) {
      if (m.source === 'variable') variables.add(m.name);
    }
    return {
      variableCandidates: [...variables].sort(),
      elementCandidates: [...elements.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([value, detail]) => (detail ? { value, detail } : value)),
    };
  }, [tree]);
}
