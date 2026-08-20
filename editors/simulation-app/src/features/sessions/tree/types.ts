/**
 * Types for the Phase B session tree.
 *
 * shape". The tree is a pure client-side projection of the model's
 * containment hierarchy, with each node tagged by one of five
 * kinds the Run page knows how to render. Live session data (values,
 * SM states, constraint verdicts) is overlaid by a separate step so
 * the structural layer stays cheap to compute and diff.
 */
import type { ConstraintVerdict, VariableValue } from '@/features/variables/VariableTree';

/**
 * Which archetype does this node represent? Drives the renderer in
 * Phase B1. Non-target kinds land in `'other'` so the renderer can
 * show them as plain containers (Package, Definition, etc.) or hide
 * them without the classifier having to pick sides.
 */
export type ModelTreeNodeKind =
  | 'part'
  | 'port'
  | 'attribute'
  | 'sm'
  | 'action'
  | 'case'
  | 'constraint'
  | 'ode'
  | 'calc'
  | 'connection'
  | 'section'
  | 'other';

export interface ModelTreeNodeBase {
  /** Tree-position identity — unique within the built tree. Used for
   *  React keys, expansion state, pin state. For nodes reached via
   *  typed-definition inlining the backend mints a fresh UUID so
   *  every mount point of the same element stays distinguishable;
   *  the original element id is on `elementId`. */
  id: string;
  /** The underlying model element's id — equals `id` for direct
   *  children, differs when the backend's dedupe pass rewrote `id`
   *  to break a React-key collision from typed-def inlining. Use
   *  this for backend lookups (hover, AST, constraint matching). */
  elementId: string;
  /** URI of the file that owns this element. */
  uri: string;
  /** Display label — the element's name, or `(unnamed)` when the backend has none. */
  name: string;
  /** Original backend kind string (e.g. `"PartUsage"`). */
  rawKind: string;
  /** Target renderer archetype — derived from `rawKind` via `classifyTreeNode`. */
  kind: ModelTreeNodeKind;
  /**
   * Depth from the tree root (0 for roots). Handy for indent / testing
   * — the consumer can derive it, but caching on the node keeps
   * scrolling cheap.
   */
  depth: number;
  /** Dotted owner-path (e.g. `"ProductionCell.Station1"`). Empty at the root. */
  ownerPath: string;
  /** Children nodes (already pruned + classified). Empty for leaves. */
  children: ModelTreeNode[];
  /** Backend-provided resolved type id for Usage kinds — the
   *  `ElementId` of the PartDefinition / ItemDefinition /
   *  PortDefinition this usage is typed by. `undefined` for
   *  non-usage kinds or when no typing resolves. Drives the
   *  Usages filter's id-based definition drop. */
  typedAs?: string;
  /** Backend hint: this node should render collapsed on initial load
   *  (Commit 2 of the model-tree rework). Set server-side for Port /
   *  Connection archetypes that have children — typed-def inlining
   *  can produce many signal-attribute children that aren't usually
   *  relevant at first glance, so the FE seeds them collapsed.
   *  Explicit user expansion overrides on subsequent renders. */
  defaultCollapsed?: boolean;
}

/**
 * Discriminated union. Kind-specific live-state payloads are attached
 * by the live-merge step (not by the structural builder) so Phase B2's
 * building block stays pure.
 */
export interface PartTreeNode extends ModelTreeNodeBase {
  kind: 'part';
  /** Aggregate "one-line" hint shown next to the part row — filled by live merge. */
  oneLiner?: string;
}

export interface AttributeTreeNode extends ModelTreeNodeBase {
  kind: 'attribute';
  /** Current live value (null when not yet observed). */
  value?: VariableValue;
  /** Optional unit from ISQ inference / model authoring. */
  unit?: string;
  /** Constraint verdict if one constrains this attribute. */
  verdict?: ConstraintVerdict;
  /** Tick at which `value` last changed — drives flash + sparkline age. */
  lastChangedTick?: number;
  /** Backend-extracted comparison-bound markers for this attribute
   *  (R3.3 — replaces the FE's `boundExtractor.ts` walker). The
   *  backend resolves attribute references to ElementId, so two
   *  circuits sharing a `temperature` short name get separate
   *  per-instance bound lists. */
  bounds?: ReadonlyArray<{
    y: number;
    kind: 'upper' | 'lower' | 'target';
    constraintName: string;
    operator: '<' | '<=' | '>' | '>=' | '==';
  }>;
}

/**
 * Static transition descriptor built from the parse-tree
 * TransitionUsage children of a state machine. The backend's
 * `/tree` endpoint currently only exposes {id, name, kind}, so
 * source / target are heuristically parsed from the transition's
 * name via the common `source_to_target` convention. When the
 * convention doesn't match, source + target stay undefined and
 * the state-graph renders the edge as an unlabelled self-loop
 * near the current state.
 */
export interface SmTransitionDescriptor {
  id: string;
  name: string;
  /** Parsed state name — undefined when `name` didn't match
   *  `source_to_target`. */
  source?: string;
  target?: string;
}

/** Static state descriptor. Mirrors a StateUsage child. */
export interface SmStateDescriptor {
  id: string;
  name: string;
}

export interface SmTreeNode extends ModelTreeNodeBase {
  kind: 'sm';
  /** Current active state (e.g. `"armed"`). */
  currentState?: string;
  /** Events this SM is willing to accept right now (for the inject menu). */
  acceptedEvents?: readonly string[];
  /** Live transitions eligible from the current state
   *  (GAP-SM-002) — `[event_name, target_state]` pairs projected
   *  from `SubsystemView.available_transitions`. The SM detail view
   *  renders these as inject-ready chips plus an annotated row in
   *  the transition table. */
  availableTransitions?: ReadonlyArray<readonly [string, string]>;
  /** Queued events that haven't yet been processed. */
  deferredCount?: number;
  /** Static topology: every StateUsage under this SM. */
  states?: readonly SmStateDescriptor[];
  /** Static topology: every TransitionUsage under this SM. */
  transitions?: readonly SmTransitionDescriptor[];
}

export interface ConstraintTreeNode extends ModelTreeNodeBase {
  kind: 'constraint';
  /** Latest pass/fail/inconclusive/error verdict. */
  verdict?: ConstraintVerdict;
  /** Human-readable expression (for hover / expanded view). */
  expression?: string;
  /** Live scalar values for each identifier referenced by the
   *  expression at the tick the verdict was computed
   *  (GAP-CONSTR-002). Lets ConstraintDetail render an operand-value
   *  overlay so the user can see *why* the constraint passed or
   *  failed. */
  operands?: Readonly<Record<string, number>>;
}

export interface OdeTreeNode extends ModelTreeNodeBase {
  kind: 'ode';
  /** Coarse numerical status reported by the integrator. */
  status?: 'stable' | 'stiff' | 'diverged';
  /** Integrated state value for this ODE (carried over from scalar_vars). */
  value?: VariableValue;
  /** Optional unit from ISQ inference / model authoring. */
  unit?: string;
}

/**
 * Plain calculation — NOT integrating state. Structural classifier
 * emits every CalculationUsage / CalculationDefinition as 'calc'.
 * `mergeLiveState` upgrades it to 'ode' when the calc's name
 * appears in `scalar_vars` (an integrator is producing state).
 * Plain calcs carry their computed value + unit + verdict so the
 * row can render like an attribute.
 */
export interface CalcTreeNode extends ModelTreeNodeBase {
  kind: 'calc';
  value?: VariableValue;
  unit?: string;
  verdict?: ConstraintVerdict;
  lastChangedTick?: number;
}

export interface OtherTreeNode extends ModelTreeNodeBase {
  kind: 'other';
}

/**
 * Port row — PortUsage / PortDefinition / ConjugatedPortDefinition.
 * Commit 1 of the model-tree rework adds Port as its own archetype so
 * the FE can render port rows distinctly. No live-state fields yet
 * (commit 2 wires those).
 */
export interface PortTreeNode extends ModelTreeNodeBase {
  kind: 'port';
}

/**
 * Connection row — ConnectionUsage / InterfaceUsage / AllocationUsage
 * / FlowUsage / SuccessionFlowUsage / BindingConnectorAsUsage and
 * their definition counterparts. Commit 1 of the model-tree rework
 * adds Connection as its own archetype. No live-state fields yet
 * (commit 2 wires typed-def inlining + default-collapse).
 */
export interface ConnectionTreeNode extends ModelTreeNodeBase {
  kind: 'connection';
}

/**
 * Action row — ActionUsage subtree (PerformActionUsage,
 * AcceptActionUsage, SendActionUsage, AssignmentActionUsage,
 * If/While/For/Loop/TerminateActionUsage, EventOccurrenceUsage). The
 * more-specific archetypes (Sm / Calc / Case / Constraint) match
 * before the generic Action bucket, so this catches what's left of
 * the action subtree. No live-state fields yet — render plain.
 */
export interface ActionTreeNode extends ModelTreeNodeBase {
  kind: 'action';
}

/**
 * Case row — the case family (CaseUsage / AnalysisCaseUsage /
 * VerificationCaseUsage / UseCaseUsage / IncludeUseCaseUsage and
 * their definitions). Spec-wise these subtype CalculationUsage →
 * ActionUsage, but they read as their own thing in the UI — a
 * verification case under an "Actions" header is misleading. Run /
 * Verify affordances key on `rawKind`, not this. No live-state
 * fields yet — render plain.
 */
export interface CaseTreeNode extends ModelTreeNodeBase {
  kind: 'case';
}

/**
 * Virtual container inserted by `splitAttrs` to group a part's live
 * outputs vs static parameters. Not backed by any model element —
 * `id` is synthesised (`<parent-id>__outputs` / `__parameters`), and
 * `rawKind` is `'Section'` so filterTree's isDefinitionKind /
 * isUsageKind helpers give a sensible `false`.
 *
 * Sections are always expanded when they render; the tree's
 * expand/collapse state still governs whether their children show,
 * so the parameters section is seeded into `expandedSet=false` by
 * default. Not selectable, not pinnable.
 */
export interface SectionTreeNode extends ModelTreeNodeBase {
  kind: 'section';
  /** Human-visible variant identifier (`'outputs'` / `'parameters'`). */
  sectionKind: 'outputs' | 'parameters';
  /** Count surfaced in the label — helps scan "how much am I hiding". */
  count: number;
}

export type ModelTreeNode =
  | PartTreeNode
  | PortTreeNode
  | AttributeTreeNode
  | SmTreeNode
  | ActionTreeNode
  | CaseTreeNode
  | ConstraintTreeNode
  | OdeTreeNode
  | CalcTreeNode
  | ConnectionTreeNode
  | SectionTreeNode
  | OtherTreeNode;
