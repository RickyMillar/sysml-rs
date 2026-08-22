/**
 * createViewFlow — pure logic behind the guided create-view flow
 * (ninebar Phase 3, audit F14). Exported separately from the component
 * so the recommendation heuristic and the snippet rewriter are
 * unit-testable without a DOM.
 *
 * Design constraints (plan §4 + views-first-class anti-patterns):
 *  - The user authors a REAL declared view in the MODEL — we never
 *    synthesize an implicit default view or resurrect
 *    `ViewType::SimulationContext`.
 *  - The backend (`POST /views/scratch`) owns spec-correct expose-ref
 *    shaping (qualified names from element ids); the frontend never
 *    hand-builds qualified names. We rewrite only the NAME and the
 *    SUPERTYPE tokens of the returned snippet.
 *  - §6 authoring loop: the flow writes the view def into the source
 *    BUFFER (dirty); saving belongs to the editor.
 */
import type { ModelTreeNode } from '@/features/sessions/tree/types';

/** The 8 standard view kinds (book ch13; mirrors sysml-views.json). */
export interface ViewTypeInfo {
  /** Internal kind key (eligibility logic, testids, availability). */
  token: string;
  /** Canonical std-lib supertype name written into SOURCE
   *  (`view def X :> <sourceToken>`) — always the `*View` spelling;
   *  the standard library defines no bare-name aliases. */
  sourceToken: string;
  label: string;
  /** One-line "when to use" — shown on the radio card. */
  blurb: string;
}

/**
 * The renderer's REAL 8 kinds (`ViewType`, sysml-diagram) — NOT the
 * book's list: there is no standard `RequirementView`/`UseCaseView`
 * (see requirements-parametric-retirement.md), so offering them as
 * cards would promise projections that don't exist.
 */
export const VIEW_TYPES: readonly ViewTypeInfo[] = [
  { token: 'General', sourceToken: 'GeneralView', label: 'General', blurb: 'Any elements as a plain node graph — the safe default.' },
  { token: 'Interconnection', sourceToken: 'InterconnectionView', label: 'Interconnection (IBD)', blurb: 'Parts with their ports and the connections between them.' },
  { token: 'StateTransition', sourceToken: 'StateTransitionView', label: 'State transition', blurb: 'States and the transitions between them.' },
  { token: 'ActionFlow', sourceToken: 'ActionFlowView', label: 'Action flow', blurb: 'Actions and the control/object flow between them.' },
  { token: 'Sequence', sourceToken: 'SequenceView', label: 'Sequence', blurb: 'Lifelines and the messages exchanged over time.' },
  { token: 'Browser', sourceToken: 'BrowserView', label: 'Browser', blurb: 'A containment tree of the exposed elements.' },
  { token: 'Grid', sourceToken: 'GridView', label: 'Grid', blurb: 'A tabular projection of the exposed elements and their attributes.' },
  { token: 'Geometry', sourceToken: 'GeometryView', label: 'Geometry', blurb: 'Spatial layout of parts (geometry-bearing models).' },
] as const;

/** Canonical std-lib supertype name for an internal kind key. */
export function sourceTokenFor(token: string): string {
  return VIEW_TYPES.find((vt) => vt.token === token)?.sourceToken ?? token;
}

// (v1's target-derived `recommendViewType` was deleted with the
// projection-first inversion — there is no target yet to recommend
// from; the cards carry model-wide availability counts instead.)

// ── v2: projection-first scope building ─────────────────────────────

/**
 * One row of the type-specialized scope picker: eligible rows are
 * selectable (checkbox → one `expose` line each); ineligible ancestors
 * render as muted group headers so the HIERARCHY stays readable.
 */
export interface ScopeRow {
  node: ModelTreeNode;
  depth: number;
  eligible: boolean;
  /** Context line, e.g. a machine's states or a part's port count. */
  hint?: string;
}

const STATE_RAW_RE = /^(State|ExhibitState)/;

function isEligible(node: ModelTreeNode, kind: string): boolean {
  switch (kind) {
    case 'StateTransition':
      return node.kind === 'sm';
    case 'Interconnection':
    case 'Geometry':
      return node.kind === 'part';
    case 'ActionFlow':
    case 'Sequence':
      return node.kind === 'action';
    case 'Grid':
      return node.children.some((c) => c.kind === 'attribute' || c.kind === 'calc');
    default: // General / Browser — any named, targetable element
      return node.kind === 'part' || node.kind === 'sm' || node.kind === 'action';
  }
}

function hintFor(node: ModelTreeNode, kind: string): string | undefined {
  if (kind === 'StateTransition') {
    const states = node.children
      .filter((c) => STATE_RAW_RE.test(c.rawKind))
      .map((c) => c.name);
    return states.length > 0 ? states.join(' · ') : undefined;
  }
  if (kind === 'Interconnection') {
    const ports = node.children.filter((c) => c.kind === 'port').length;
    const conns = node.children.filter((c) => c.kind === 'connection').length;
    if (ports + conns === 0) return undefined;
    return [ports > 0 ? `${ports} port${ports === 1 ? '' : 's'}` : null, conns > 0 ? `${conns} connection${conns === 1 ? '' : 's'}` : null]
      .filter(Boolean)
      .join(' · ');
  }
  if (kind === 'Grid') {
    const attrs = node.children.filter((c) => c.kind === 'attribute' || c.kind === 'calc').length;
    return `${attrs} attribute${attrs === 1 ? '' : 's'}`;
  }
  return undefined;
}

/**
 * Flatten the model tree into picker rows for a view kind: eligible
 * nodes plus the ancestor chain needed to show them in place. Subtrees
 * with no eligible descendants are pruned entirely.
 */
export function buildScopeRows(nodes: readonly ModelTreeNode[], kind: string, depth = 0): ScopeRow[] {
  const rows: ScopeRow[] = [];
  for (const node of nodes) {
    const childRows = buildScopeRows(node.children, kind, depth + 1);
    const eligible = isEligible(node, kind);
    if (eligible || childRows.length > 0) {
      rows.push({ node, depth, eligible, hint: eligible ? hintFor(node, kind) : undefined });
      rows.push(...childRows);
    }
  }
  return rows;
}

/** Eligible-target count per kind — drives the availability badges on
 *  the projection cards (replaces v1's target-derived recommendation:
 *  in a type-first flow there is no target yet to recommend from). */
export function kindAvailability(nodes: readonly ModelTreeNode[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const vt of VIEW_TYPES) counts[vt.token] = 0;
  const walk = (list: readonly ModelTreeNode[]) => {
    for (const node of list) {
      for (const vt of VIEW_TYPES) {
        if (isEligible(node, vt.token)) counts[vt.token] += 1;
      }
      walk(node.children);
    }
  };
  walk(nodes);
  return counts;
}

/** `motor.trip_unit` → `MotorTripUnitView` — a legal identifier default. */
export function defaultViewName(targetName: string): string {
  const parts = targetName.split(/[^A-Za-z0-9]+/).filter(Boolean);
  const pascal = parts.map((p) => p.charAt(0).toUpperCase() + p.slice(1)).join('');
  const base = pascal.replace(/^[0-9]+/, '');
  const capped = base.charAt(0).toUpperCase() + base.slice(1);
  return `${capped || 'New'}View`;
}

export function isValidViewName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name);
}

/**
 * Rewrite the backend scratch snippet's NAME and SUPERTYPE, preserving
 * everything else (the spec-correct expose refs the backend computed).
 * Header shapes handled: `view scratch : InterconnectionView {` (usage,
 * post-F14) and `view def scratch :> InterconnectionView {` (def form).
 * Pass a canonical std-lib name (`sourceTokenFor`) as `supertypeToken` —
 * the token lands verbatim in the user's model.
 * The def/usage form and the `:`/`:>` relator are preserved as-is —
 * the backend chose them spec-correctly for its form.
 *
 * Returns null when the header doesn't match — callers must fail loud,
 * never paste a half-rewritten snippet into the user's model.
 */
export function rewriteScratchSnippet(
  snippet: string,
  name: string,
  supertypeToken: string,
): string | null {
  const HEADER = /^(\s*view\s+(?:def\s+)?)([A-Za-z_][A-Za-z0-9_]*)(\s*:>?\s*)([A-Za-z_][A-Za-z0-9_:]*)/m;
  if (!HEADER.test(snippet)) return null;
  return snippet.replace(HEADER, (_m, pre, _oldName, relator, _oldType) => `${pre}${name}${relator}${supertypeToken}`);
}
