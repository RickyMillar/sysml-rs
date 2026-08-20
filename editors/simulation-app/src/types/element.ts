/**
 * Element types for the workspace model tree and element references.
 */

// ── Element Reference ──────────────────────────────────────────────────
// Every interaction (selection, activity target, context menu) uses this
// to identify an element within a specific file.

export interface ElementRef {
  uri: string;        // file URI (e.g. "file:///project/boiler.sysml")
  elementId: string;  // element ID within the model graph
  name: string;       // display name
  kind: string;       // element kind (StateDefinition, ConstraintUsage, etc.)
}

// ── Tree Nodes ─────────────────────────────────────────────────────────
// Per-file tree (returned by the backend /models/{uri}/tree endpoint).

export interface TreeNode {
  /** Unique per tree position within a single `model_tree` response.
   *  Used for React keys, expansion state, pin state — anything that
   *  needs to distinguish "this mount point" from another occurrence
   *  of the same underlying element. */
  id: string;
  name: string | null;
  kind: string;
  /** Backend-projected archetype (R2.4) — replaces the FE's
   *  `classifyTreeNode` KIND_MAP. The FE upgrades
   *  `archetype === 'calc' && is_ode === true` → `'ode'` at
   *  build time. */
  archetype:
    | 'part'
    | 'port'
    | 'attribute'
    | 'sm'
    | 'action'
    | 'case'
    | 'constraint'
    | 'calc'
    | 'connection'
    | 'section'
    | 'other';
  children: TreeNode[];
  /** Authoritative ODE flag: `true` when this node is a CalculationUsage
   *  or CalculationDefinition whose subsetting chain reaches the spec's
   *  `GetDerivative` type (GAP-ODE-001). Omitted when false to keep the
   *  wire format minimal. */
  is_ode?: boolean;
  /** For Usage-kind elements, the ElementId of the resolved definition
   *  they are typed by — or absent when no `FeatureTyping` resolves.
   *  Comes from sysml-core's `find_feature_type` O(1) index; the
   *  backend (sysml-service::query) forwards it onto every tree node.
   *  Drives the Usages filter's "drop any def that a usage points at"
   *  check (no name-string heuristics). */
  typed_as?: string;
  /** URI of the source file that declares this element, from its
   *  first span. Per-node because the workspace-scoped tree
   *  (`__workspace__`) merges every file into one graph — there is no
   *  meaningful per-response uri. Also present on per-file trees,
   *  where it is more precise than the request uri (typed-def
   *  inlining surfaces children declared in other files). Absent for
   *  synthetic nodes with no span. */
  source_uri?: string;
  /** Original element id when `id` was rewritten by the backend's
   *  dedupe pass (typed-definition inlining surfaces the same element
   *  under every usage — React keys need per-mount uniqueness, but
   *  lookups by element id — hover, AST, detail panel — still need
   *  the real id). Absent when no rewrite happened. Prefer
   *  `element_id ?? id` for element-level lookups. */
  element_id?: string;
  /** For TransitionUsage / SuccessionAsUsage nodes, the source state's
   *  short name resolved by the backend (via the element's `source` /
   *  `unresolved_source` props). Lets the FE skip the legacy
   *  `parseTransitionName` regex on `state_a_to_state_b` names. */
  source?: string;
  /** Mirror of `source` for the transition's target state. */
  target?: string;
  /** For AttributeUsage nodes, the SI unit string inferred by the
   *  backend (e.g. "V", "kg·m/s²"). Lets the FE drop the
   *  `metricRegistry` name-keyed unit lookup (R3.1). */
  unit?: string;
  /** For AttributeUsage nodes, the canonical ISQ dimension string
   *  (e.g. "L^2·M·T^-3·I^-1"). */
  isq_dimension?: string;
  /** For SM nodes, static transitions extracted from
   *  TransitionUsage children at projection time. Backend-emitted
   *  in both views (R2.1 fixup). */
  transitions?: ReadonlyArray<{
    id: string;
    name?: string;
    source?: string;
    target?: string;
  }>;
  /** For AttributeUsage nodes, comparison-bound markers extracted by the
   *  backend from referencing constraints (R3.3). Replaces the FE's
   *  `boundExtractor.ts` AST walker. Empty when no constraint bounds
   *  this attribute. */
  bounds?: ReadonlyArray<{
    y: number;
    kind: 'upper' | 'lower' | 'target';
    constraint_name: string;
    operator: '<' | '<=' | '>' | '>=' | '==';
  }>;
  /** Backend hint: render collapsed on initial load. Used for Port /
   *  Connection archetypes whose typed-def inlining can fan out into
   *  many signal-attribute children that aren't usually relevant at
   *  first glance. Absent (false) for nodes the backend wants the FE
   *  to render expanded by default. */
  default_collapsed?: boolean;
}

// Workspace-level tree (files as roots, elements nested under each).
// Built client-side from per-file trees.

export interface WorkspaceTreeNode {
  type: 'file' | 'element';
  uri: string;             // which file this belongs to
  elementId?: string;      // for element nodes
  name: string;            // file name or element name
  kind: string;            // 'file' or element kind
  children: WorkspaceTreeNode[];
}

// ── Element Detail ─────────────────────────────────────────────────────
// Fetched on demand when an element is selected (click in tree or diagram).
//
// Mirrors the actual backend response from
//   GET /models/:uri/elements/:id            (id, kind, name, owner,
//                                             owning_membership, qname,
//                                             props, spans)
//   GET /models/:uri/elements/:id/children   (Vec<Element>)
//
// Children are fetched separately and merged into this struct.

export interface ElementChild {
  id: string;
  name: string | null;
  kind: string;
}

export interface ElementSpan {
  uri?: string | null;
  start?: number;
  end?: number;
  // Spans may carry a couple of shapes depending on serializer; keep loose.
  [key: string]: unknown;
}

export interface ElementDetail {
  id: string;
  name: string | null;
  kind: string;
  /** Element.owner (cached from owning_membership). */
  owner: string | null;
  /** Element.owning_membership — pointer to the OwningMembership element. */
  owningMembership: string | null;
  /** Optional qualified name when computed by the resolver. */
  qualifiedName: string | null;
  /** Element.props as a stringified key/value map. */
  props: Record<string, string>;
  /** Children fetched from /elements/:id/children. */
  children: ElementChild[];
  /** Source spans (one per file location). */
  spans: ElementSpan[];
}

// ── Element Kind Helpers ───────────────────────────────────────────────

export const SIMULATABLE_KINDS = [
  'StateDefinition',
  'ActionDefinition',
  'ActionUsage',
  'ExhibitStateUsage',
] as const;

export const CONSTRAINT_KINDS = [
  'ConstraintUsage',
  'ConstraintDefinition',
] as const;

export const REQUIREMENT_KINDS = [
  'RequirementDefinition',
  'RequirementUsage',
  'VerificationCaseDefinition',
  'VerificationCaseUsage',
] as const;

export const ANALYSIS_KINDS = [
  'AnalysisCaseDefinition',
  'AnalysisCaseUsage',
] as const;

export const PORT_KINDS = [
  'PortUsage',
  'PortDefinition',
  'FlowConnectionUsage',
] as const;

export function isSimulatable(kind: string): boolean {
  return (SIMULATABLE_KINDS as readonly string[]).includes(kind);
}

export function isConstraint(kind: string): boolean {
  return (CONSTRAINT_KINDS as readonly string[]).includes(kind);
}

export function isRequirement(kind: string): boolean {
  return (REQUIREMENT_KINDS as readonly string[]).includes(kind);
}

export function isAnalysisCase(kind: string): boolean {
  return (ANALYSIS_KINDS as readonly string[]).includes(kind);
}

export function isPort(kind: string): boolean {
  return (PORT_KINDS as readonly string[]).includes(kind);
}

// ── Tree Construction ──────────────────────────────────────────────────

/** Build workspace tree from per-file trees. */
export function buildWorkspaceTree(
  files: Map<string, { uri: string; tree: TreeNode[] }>,
): WorkspaceTreeNode[] {
  const result: WorkspaceTreeNode[] = [];

  for (const [uri, file] of files) {
    const fileName = uri.split('/').pop() ?? uri;
    result.push({
      type: 'file',
      uri,
      name: fileName,
      kind: 'file',
      children: file.tree.map((node) => treeNodeToWorkspace(uri, node)),
    });
  }

  return result;
}

/** Kinds that are meaningful even without a name (structural containers). */
const ALWAYS_SHOW_KINDS = new Set([
  'Package', 'StateDefinition', 'StateUsage', 'TransitionUsage',
  'ActionDefinition', 'ActionUsage', 'ConstraintDefinition', 'ConstraintUsage',
  'RequirementDefinition', 'VerificationCaseDefinition', 'AnalysisCaseDefinition',
  'PartDefinition', 'PartUsage', 'PortDefinition', 'PortUsage', 'AttributeDefinition',
]);

function treeNodeToWorkspace(uri: string, node: TreeNode): WorkspaceTreeNode {
  // Filter children: hide unnamed nodes unless they have a meaningful kind
  // or have named children themselves
  const children = node.children
    .map((child) => treeNodeToWorkspace(uri, child))
    .filter((child) => {
      if (child.name !== '(unnamed)') return true;
      if (ALWAYS_SHOW_KINDS.has(child.kind)) return true;
      if (child.children.length > 0) return true;
      return false;
    });

  return {
    type: 'element',
    uri,
    elementId: node.id,
    name: node.name ?? '(unnamed)',
    kind: node.kind,
    children,
  };
}
