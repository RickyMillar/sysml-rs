/**
 * requirementsLinksRailContext — registers the `requirements-links`
 * right-rail context (Phase 7.5 v1: the detail/links rail).
 *
 * The sanctioned rail-context pattern: registration is a module-load
 * side effect via the public `registerRailContext` API; `render()`
 * takes no args and the body reads the selection from
 * `requirementsSelectionStore` (F15). (Verify's rail context, the
 * original twin, was deleted with design 1a — the case view is
 * Verify's one detail surface; this is now the pattern's home.)
 *
 * Writable link groups (satisfied by / verified by / derives from /
 * derived to) end in a `+ add` affordance — R5 link WRITING (design
 * §7.6): a FuzzyCombobox picker over user-authored candidates; the
 * commit runs the same six-step splice loop, landing in the PICKED
 * element's file. `refines` stays read-only (filed, not R5).
 * Attribute VALUES are inline-editable (v2 §7.5). Kind is carried by
 * SHAPE (square = satisfier part, circle = verification case, ↑/↓ =
 * derivation direction), colour stays neutral: there is no semantic
 * element-kind colour token yet (the demo's requirement-magenta /
 * part-blue vocabulary maps only to `--nb-cat-*` primitives, which
 * components must not reference — an open token-sheet question).
 *
 * v1.5b adds a HISTORY section: the element's workflow event log
 * (attestations, comments, approvals — newest first). Superseded
 * clearings stay listed, flagged — append-only history is never
 * forgotten. Events attributed as tool state, not model claims.
 *
 * v1.6 (B2.1/R18) adds the EVALUATED CONTRACT: the verdict-input block
 * (subject + assume/require constraints + referenced attribute values)
 * rendered adjacent to the "verified by" chips — a pass/fail must be
 * inspectable, not asserted — and a separate NARRATIVE bucket (actors /
 * stakeholders / framed concerns / rationale). The bucket separation is
 * binding (design doc §2.1): narrative roles are not verdict inputs and
 * never sit next to the verdict.
 *
 * v2 THREE-REGISTER ZONING (binding spec:
 * The rail is no longer one flat scroll — each of the §1 boundary ADR's
 * layers gets its own zone and visual register:
 *   · COMPUTED digest pinned top — darker inset (`--surface-sunken`),
 *     dashed rules, an `ƒ` mark, never an edit affordance. The tool's
 *     read of the model (verified rollup, suspect flag).
 *   · MODEL scroll in the middle — the authored surface (statement,
 *     contract incl. maturity, links, narrative, source provenance):
 *     standard panel, solid hairlines, mono vocabulary, edit
 *     affordances, dirty-dot mechanics unchanged (§7.5).
 *   · PROCESS zone pinned bottom behind a 3px double rule — warm raised
 *     surface (`--surface-raised`), Sans register, every row attributed
 *     and append-only. Approval is a STEPPER here (never a chip — the
 *     maturity-vs-approval ruling); the full event log sits behind an
 *     expand so the pinned zone stays compact (hybrid refinement).
 */

import {
  createContext,
  useContext,
  useMemo,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { registerRailContext } from '@/app/rail/railRegistry';
import { rowDisplayId } from '@/features/requirements/rollup';
import {
  useElementSource,
  useRequirementDetail,
  useRequirementRows,
} from '@/features/requirements/queries';
import type {
  RequirementAttribute,
  RequirementConstraint,
  RequirementDetail,
  RequirementLinkRef,
  RequirementRow,
} from '@/features/requirements/types';
import { cellKey, useRequirementEditStore } from '@/features/requirements/editStore';
import { useRequirementCellEdit } from '@/features/requirements/useFieldEdit';
import { AddLinkRow, buildPickerOptions } from './AddLinkPopover';
import { useLinkTargetCandidates } from '@/features/requirements/queries';
import { useBaselineStore } from '@/features/baselines/store';
import { useSuspects } from '@/features/baselines/queries';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';
import { CellBadge, CellErrorLine, InlineTextEditor, MaturitySelect } from './InlineEditors';
import {
  useAssign,
  useComment,
  useSetApproval,
  useSignOff,
  useWorkflowLog,
  useWorkflowState,
} from '@/features/workflow/queries';
import { APPROVAL_INITIAL } from '@/features/workflow/types';
import type { WorkflowEventWire } from '@/features/workflow/types';
import { ActorGate, WORKFLOW_INPUT_STYLE } from '@/features/workflow/ActorGate';
import { ApprovalStepper, MaturityChip, MethodChip, VerifiedChip } from './RequirementChips';
import { useRequirementsSelectionStore } from './requirementsSelectionStore';

export const REQUIREMENTS_LINKS_CONTEXT_ID = 'requirements-links';

const GROUP_LABEL: CSSProperties = {
  color: 'var(--text-secondary)',
  fontSize: 'var(--text-xs)',
  margin: '12px 0 4px',
};

const LINK_ROW: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: 'var(--row-compact)',
  gap: 8,
  fontFamily: 'var(--font-mono)',
  fontSize: 11.5,
  color: 'var(--text-primary)',
};

function linkName(ref: RequirementLinkRef): string {
  return ref.name ?? ref.id;
}

/** Link-chip navigation: resolve a ref id to a table row and select it.
 *  Provided once by the rail body; LinkGroup rows whose target IS a
 *  table row become clickable (parts/cases aren't rows — those stay
 *  plain text until their own workbenches grow deep links). */
const RailNav = createContext<{
  resolve: (id: string) => RequirementRow | undefined;
  navigate: (row: RequirementRow) => void;
} | null>(null);

function LinkGroup({
  label,
  refs,
  glyph,
  emptyText,
  add,
}: {
  label: string;
  refs: RequirementLinkRef[];
  glyph: (ref: RequirementLinkRef) => ReactNode;
  emptyText: string;
  /** Optional `+ add` affordance (R5 link writing, design §7.6). */
  add?: ReactNode;
}) {
  const nav = useContext(RailNav);
  return (
    <>
      <div style={GROUP_LABEL}>{label}</div>
      {refs.length === 0 ? (
        <div style={{ ...LINK_ROW, fontFamily: 'var(--font-body)', color: 'var(--text-disabled)' }}>
          {emptyText}
        </div>
      ) : (
        refs.map((ref) => {
          const target = nav?.resolve(ref.id);
          if (target && nav) {
            return (
              <button
                key={ref.id}
                type="button"
                data-testid={`rail-link-${ref.id}`}
                title={`${ref.kind} — ${rowDisplayId(target)}; click to open`}
                onClick={() => nav.navigate(target)}
                style={{
                  ...LINK_ROW,
                  width: '100%',
                  border: 'none',
                  background: 'transparent',
                  padding: 0,
                  cursor: 'pointer',
                  textAlign: 'left',
                  color: 'var(--accent-fg)',
                }}
              >
                {glyph(ref)}
                {linkName(ref)}
                <span
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: 'var(--text-xs)',
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  {rowDisplayId(target) !== linkName(ref) ? rowDisplayId(target) : ''}
                </span>
              </button>
            );
          }
          return (
            <div key={ref.id} style={LINK_ROW} title={ref.kind}>
              {glyph(ref)}
              {linkName(ref)}
            </div>
          );
        })
      )}
      {add}
    </>
  );
}

const squareGlyph = () => (
  <span
    aria-hidden
    style={{ width: 7, height: 7, flex: 'none', border: '1.5px solid var(--border-strong)' }}
  />
);
const dotGlyph = () => (
  <span
    aria-hidden
    style={{
      width: 6,
      height: 6,
      flex: 'none',
      borderRadius: '50%',
      background: 'var(--border-strong)',
    }}
  />
);
const arrowGlyph = (dir: '↑' | '↓') => () => (
  <span aria-hidden style={{ color: 'var(--text-muted)' }}>
    {dir}
  </span>
);

const VERIFY_CASE_KINDS = ['VerificationCaseDefinition', 'VerificationCaseUsage'];
const REQUIREMENT_KINDS = ['RequirementDefinition', 'RequirementUsage'];
const SATISFY_SUBJECT_KINDS = ['PartUsage'];

function RequirementsLinksRailBody() {
  const row = useRequirementsSelectionStore((s) => s.selectedRow);
  const setSelectedRow = useRequirementsSelectionStore((s) => s.setSelectedRow);
  const detail = useRequirementDetail(row?.id ?? null);
  const { commitSatisfyLink, commitVerifyLink, commitDeriveLink, commitRefineLink } =
    useRequirementCellEdit();
  // Shares the table's react-query cache — no extra fetch. Powers the
  // clickable link chips: a ref that IS a table row navigates to it.
  const rowsQuery = useRequirementRows();
  const nav = useMemo(() => {
    const byId = new Map((rowsQuery.data?.rows ?? []).map((r) => [r.id, r]));
    return {
      resolve: (id: string) => byId.get(id),
      navigate: (target: RequirementRow) => setSelectedRow(target),
    };
  }, [rowsQuery.data, setSelectedRow]);
  if (!row) {
    return (
      <div
        data-testid="requirements-links-empty"
        style={{ padding: 12, fontSize: 11, color: 'var(--text-muted)', lineHeight: 1.5 }}
      >
        Select a requirement to see its statement and satisfy / verify /
        derivation links.
      </div>
    );
  }
  return (
    <RailNav.Provider value={nav}>
    <div
      data-testid="requirements-links-body"
      style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}
    >
      <ComputedDigest row={row} rowsRevision={rowsQuery.data?.revision ?? null} />
      {/* MODEL zone — the authored surface. Scrolls between the two
          pinned zones; solid hairlines, mono vocabulary, edit
          affordances (§7.5 mechanics unchanged). */}
      <div
        data-testid="requirements-model-zone"
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: 12 }}
      >
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-secondary)',
          marginBottom: 8,
        }}
      >
        {rowDisplayId(row)}
        {row.name && row.name !== rowDisplayId(row) && (
          <span style={{ color: 'var(--text-primary)', marginLeft: 8 }}>{row.name}</span>
        )}
      </div>
      <div
        style={{
          fontSize: 'var(--text-sm)',
          color: 'var(--text-secondary)',
          lineHeight: 1.5,
          marginBottom: 4,
          whiteSpace: 'pre-line',
        }}
      >
        {row.text ?? '(no statement text)'}
      </div>
      <LinkGroup
        label="satisfied by"
        refs={row.satisfied_by}
        glyph={squareGlyph}
        emptyText="none"
        add={
          <AddLinkRow
            label="satisfying part"
            rowId={row.id}
            field="link_satisfy"
            kinds={SATISFY_SUBJECT_KINDS}
            excludeIds={row.satisfied_by.map((r) => r.id)}
            onCommit={(subjectId) => commitSatisfyLink(row.id, subjectId)}
          />
        }
      />
      {/* Verdict inputs render DIRECTLY ABOVE the verified chips — one
          visual unit: what verification evaluates, then who evaluated it. */}
      {detail.isError ? (
        <div
          data-testid="requirements-contract-error"
          style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', margin: '12px 0 4px' }}
        >
          contract unavailable
        </div>
      ) : (
        detail.data && <VerdictInputs detail={detail.data} row={row} />
      )}
      <LinkGroup
        label="verified by"
        refs={row.verified_by}
        glyph={dotGlyph}
        emptyText="none yet"
        add={
          <AddLinkRow
            label="verifying case"
            rowId={row.id}
            field="link_verify"
            kinds={VERIFY_CASE_KINDS}
            excludeIds={row.verified_by.map((r) => r.id)}
            onCommit={(caseId) => commitVerifyLink(row.id, caseId)}
          />
        }
      />
      {/* DECLARED method (B4) — the cases' @VerificationMethod union;
          model intent, rendered under the cases it belongs to and
          deliberately NOT inside the verdict block above. Hidden when no
          case declares one (never a fabricated default). */}
      {row.verification_methods.length > 0 && (
        <div data-testid="requirements-rail-method">
          <div style={GROUP_LABEL}>declared method</div>
          <MethodChip methods={row.verification_methods} />
        </div>
      )}
      <LinkGroup
        label="derives from"
        refs={row.derived_from}
        glyph={arrowGlyph('↑')}
        emptyText="none"
        add={
          <AddLinkRow
            label="original requirement"
            rowId={row.id}
            field="link_derive"
            kinds={REQUIREMENT_KINDS}
            excludeIds={[row.id, ...row.derived_from.map((r) => r.id)]}
            onCommit={(originalId) => commitDeriveLink(row.id, originalId, row.id)}
          />
        }
      />
      <LinkGroup
        label="derived to"
        refs={row.derives}
        glyph={arrowGlyph('↓')}
        emptyText="none"
        add={
          <AddLinkRow
            label="derived requirement"
            rowId={row.id}
            field="link_derive_to"
            kinds={REQUIREMENT_KINDS}
            excludeIds={[row.id, ...row.derives.map((r) => r.id)]}
            onCommit={(derivedId) => commitDeriveLink(derivedId, row.id, row.id, 'link_derive_to')}
          />
        }
      />
      <LinkGroup
        label="refines"
        refs={row.refines}
        glyph={arrowGlyph('↑')}
        emptyText="none"
        add={
          <AddLinkRow
            label="refined requirement"
            rowId={row.id}
            field="link_refine"
            kinds={REQUIREMENT_KINDS}
            excludeIds={[row.id, ...row.refines.map((r) => r.id)]}
            onCommit={(refinedId) => commitRefineLink(row.id, refinedId)}
          />
        }
      />
      {/* Reverse typing (defs only in practice): content usages
          instantiating this template — check occurrences ride the
          "verified by" chips instead, never here. Hidden when empty
          (most usages instantiate nothing). */}
      {detail.data && detail.data.instantiated_by.length > 0 && (
        <div data-testid="requirements-instantiated-by">
          <LinkGroup
            label="instantiated by"
            refs={detail.data.instantiated_by}
            glyph={dotGlyph}
            emptyText="none"
          />
        </div>
      )}
      {detail.data && <NarrativeBucket detail={detail.data} />}
      <SourceSection row={row} />
      </div>
      <ProcessZone elementId={row.id} />
    </div>
    </RailNav.Provider>
  );
}

/**
 * COMPUTED digest (pinned top) — the tool's read of the model, in the
 * computed register: darker inset, dashed bottom rule, `ƒ` mark, and
 * never an edit affordance. Verdict colours stay confined to the
 * VerifiedChip (§5). The suspect row appears only when a baseline is
 * selected AND this row is flagged against it — never a fabricated
 * "not suspect" claim when nothing was compared.
 */
function ComputedDigest({
  row,
  rowsRevision,
}: {
  row: RequirementRow;
  rowsRevision: number | null;
}) {
  const baseline = useBaselineStore((s) => s.selected);
  // Shares the workflow's suspects query (same key) — no extra fetch.
  const suspects = useSuspects(baseline, rowsRevision);
  const suspect = baseline !== null && (suspects.data?.has(row.id) ?? false);
  const digestRow: CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    minHeight: 'var(--row-compact)',
    gap: 10,
    padding: '0 12px',
    fontSize: 'var(--text-sm)',
  };
  return (
    <div
      data-testid="requirements-computed-digest"
      style={{
        flex: 'none',
        background: 'var(--surface-sunken)',
        borderBottom: '1px dashed var(--border-default)',
        paddingBottom: 6,
        fontFamily: 'var(--font-body)',
      }}
    >
      <div
        style={{
          ...digestRow,
          gap: 8,
          fontSize: 'var(--text-xs)',
          color: 'var(--text-disabled)',
        }}
      >
        <span aria-hidden style={{ fontFamily: 'var(--font-mono)', fontStyle: 'italic' }}>
          ƒ
        </span>
        computed — the tool's read · not authored
      </div>
      <div style={digestRow}>
        <span style={{ ...CONTRACT_TAG, width: 56 }}>verified</span>
        <VerifiedChip rollup={row.verification} />
      </div>
      {suspect && (
        <div data-testid="requirements-digest-suspect" style={digestRow}>
          <span style={{ ...CONTRACT_TAG, width: 56 }}>suspect</span>
          <span
            aria-hidden
            style={{ fontFamily: 'var(--font-mono)', color: 'var(--severity-warning)' }}
          >
            ⚑
          </span>
          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)' }}>
            changed since{' '}
            <span style={{ fontFamily: 'var(--font-mono)' }}>{baseline}</span>
          </span>
        </div>
      )}
    </div>
  );
}

/**
 * PROCESS zone (pinned bottom) — the tool-workflow layer in the process
 * register: warm raised surface behind a heavy double rule, Sans
 * typography, "record / sign" verbs, nothing revertible. Hybrid layout
 * (crib sheet open call #1, resolved as recommended): the approval
 * stepper + write rows stay visible; the full event log sits behind an
 * expand so the pinned zone stays compact.
 */
function ProcessZone({ elementId }: { elementId: string }) {
  return (
    <div
      data-testid="requirements-process-zone"
      style={{
        flex: 'none',
        maxHeight: '50%',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        background: 'var(--surface-raised)',
        borderTop: '3px double var(--border-strong)',
        fontFamily: 'var(--font-body)',
      }}
    >
      <div
        title="process — recorded as you · append-only · never in source"
        style={{
          lineHeight: 'var(--row-compact)',
          padding: '0 12px',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-muted)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        process — recorded as you · append-only · never in source
      </div>
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '0 12px 10px' }}>
        <WorkflowControls elementId={elementId} />
        <WorkflowHistory elementId={elementId} />
      </div>
    </div>
  );
}

/**
 * Source provenance (bottom of the rail, collapsed by default): where in
 * the `.sysml` text this requirement is declared, expandable to the
 * verbatim declaration slice (`sysml.get_source` — the same honesty as
 * the contract's verbatim constraint text).
 */
function SourceSection({ row }: { row: RequirementRow }) {
  const [open, setOpen] = useState(false);
  const span = row.source_span;
  const source = useElementSource(open ? (span?.file ?? null) : null, row.id);
  if (!span) return null;
  const shortFile = span.file.split('/').slice(-2).join('/');
  return (
    <div data-testid="requirements-source">
      <button
        type="button"
        data-testid="requirements-source-toggle"
        onClick={() => setOpen(!open)}
        style={{
          ...GROUP_LABEL,
          display: 'flex',
          gap: 6,
          alignItems: 'baseline',
          width: '100%',
          border: 'none',
          background: 'transparent',
          padding: 0,
          cursor: 'pointer',
          textAlign: 'left',
        }}
        title={span.file}
      >
        <span aria-hidden>{open ? '▾' : '▸'}</span>
        source
        <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-muted)' }}>
          {shortFile}
          {span.line != null ? `:${span.line}` : ''}
        </span>
      </button>
      {open &&
        (source.data ? (
          <pre
            data-testid="requirements-source-text"
            style={{
              margin: '4px 0 0',
              padding: 8,
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--text-xs)',
              lineHeight: 1.5,
              color: 'var(--text-secondary)',
              background: 'var(--surface-sunken, rgba(0,0,0,0.15))',
              borderRadius: 'var(--radius-sm)',
              overflowX: 'auto',
              whiteSpace: 'pre',
            }}
          >
            {source.data.text}
          </pre>
        ) : (
          <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
            {source.isError ? 'source unavailable' : 'loading…'}
          </div>
        ))}
    </div>
  );
}

const CONTRACT_ROW: CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: 11.5,
  lineHeight: 1.6,
  color: 'var(--text-primary)',
  display: 'flex',
  gap: 8,
  alignItems: 'baseline',
};

const CONTRACT_TAG: CSSProperties = {
  flex: 'none',
  color: 'var(--text-muted)',
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--text-xs)',
};

function constraintText(c: RequirementConstraint): string {
  if (c.text !== null) return c.text;
  if (c.referenced_definition !== null) {
    return `: ${c.referenced_definition.name ?? c.referenced_definition.id}`;
  }
  // Reference form whose name didn't resolve unambiguously — show the
  // gap, never a guess (ADR-009 posture).
  return '(unresolved constraint reference)';
}

/**
 * The verdict-input block (R18): subject, assume/require constraint text,
 * referenced attribute values. Everything the requirement's verdict
 * expression reads — and nothing else (narrative goes in
 * [`NarrativeBucket`]). Constraint text is verbatim source (tooltip says
 * so — do not overclaim AST fidelity).
 */
function VerdictInputs({ detail, row }: { detail: RequirementDetail; row: RequirementRow }) {
  // Owned first, then inherited (single-hop typing target, mirrors the
  // evaluator; populated only when the requirement owns none). The
  // provenance suffix on inherited rows is BINDING — an unlabeled
  // inherited row misleads about where to edit it.
  const constraints = [
    ...detail.assumed_constraints.map((c) => ({ tag: 'assume', c })),
    ...detail.required_constraints.map((c) => ({ tag: 'require', c })),
    ...detail.inherited_assumed_constraints.map((c) => ({ tag: 'assume', c })),
    ...detail.inherited_required_constraints.map((c) => ({ tag: 'require', c })),
  ];
  return (
    <div data-testid="requirements-verdict-inputs">
      <div style={GROUP_LABEL}>contract</div>
      {detail.subject ? (
        <div style={CONTRACT_ROW} title={detail.subject.kind}>
          <span style={CONTRACT_TAG}>subject</span>
          <span>{linkName(detail.subject)}</span>
        </div>
      ) : (
        <AddRolePicker elementId={detail.id} role="subject" label="subject" />
      )}
      <MaturityRow row={row} />
      {constraints.map(({ tag, c }) => (
        <div
          key={c.id}
          data-testid={`requirements-constraint-${tag}`}
          style={CONTRACT_ROW}
          title="verbatim source text"
        >
          <span style={CONTRACT_TAG}>{tag}</span>
          <span style={{ whiteSpace: 'pre-wrap' }}>
            {constraintText(c)}
            {c.inherited_from && (
              <span
                data-testid="requirements-constraint-inherited-from"
                title={`Inherited from the ${c.inherited_via ?? 'typing'} ${c.inherited_from.kind} — edit it there`}
                style={{ color: 'var(--text-muted)' }}
              >
                {' '}
                · from {c.inherited_from.name ?? c.inherited_from.id}
                {c.inherited_via === 'specialization' ? ' (:>)' : ''}
              </span>
            )}
          </span>
        </div>
      ))}
      <AddConstraintRow elementId={detail.id} />
      {detail.referenced_attributes.map((attr) => (
        <AttributeValueRow key={attr.id} attr={attr} />
      ))}
      <AddAttributeRow elementId={detail.id} />
    </div>
  );
}

/**
 * Maturity inside the model contract (register ruling: a `@StatusInfo`
 * SOURCE field you edit — lowercase mono chip on the model surface,
 * with edit affordance; contrast the approval stepper, which lives only
 * in the process zone). Click the chip to open the closed-vocab select;
 * same commit path as grid/document maturity editing (§7.5).
 */
function MaturityRow({ row }: { row: RequirementRow }) {
  const key = cellKey(row.id, 'maturity');
  const editing = useRequirementEditStore((s) => s.editingKey === key);
  const pending = useRequirementEditStore((s) => s.pendingKey === key);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitMaturity } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;
  return (
    <div data-testid="req-rail-maturity" style={CONTRACT_ROW}>
      <span style={CONTRACT_TAG}>maturity</span>
      {editing ? (
        <MaturitySelect
          data-testid="req-rail-maturity-select"
          initial={row.maturity}
          onCommit={(status) => commitMaturity(row, status)}
          onCancel={cancelEdit}
        />
      ) : (
        <span
          onClick={() => beginEdit(key)}
          title="Maturity (@StatusInfo) — a source field; click to edit"
          style={{ cursor: 'pointer', display: 'inline-flex', alignItems: 'center', gap: 6 }}
        >
          {row.maturity !== null ? (
            <MaturityChip maturity={row.maturity} />
          ) : (
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-disabled)' }}>
              + set maturity
            </span>
          )}
          {pending && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
        </span>
      )}
      {!editing && failure && <CellErrorLine message={failure} />}
    </div>
  );
}

/** `+ add constraint` (§7.7) — assume/require toggle + optional name + a
 *  single-line expression, spliced into `<kind> constraint [name] { expr }`. */
function AddConstraintRow({ elementId }: { elementId: string }) {
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<'assume' | 'require'>('require');
  const [name, setName] = useState('');
  const [expr, setExpr] = useState('');
  const key = cellKey(elementId, 'constraint_add');
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitAddConstraint } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;

  const close = () => {
    setOpen(false);
    setName('');
    setExpr('');
    cancelEdit();
  };
  const submit = () => {
    const e = expr.trim();
    if (!e || /[{};]/.test(e)) return;
    commitAddConstraint(elementId, kind, e, name.trim() || null);
    close();
  };

  if (!open) {
    return (
      <>
        <button
          type="button"
          data-testid="req-add-constraint"
          onClick={() => {
            if (beginEdit(key)) setOpen(true);
          }}
          style={ADD_BUTTON}
        >
          + add constraint
          {pendingKey === key && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
        </button>
        {failure && <CellErrorLine message={failure} />}
      </>
    );
  }
  return (
    <div
      onKeyDown={(e) => {
        if (e.key === 'Escape') close();
        if (e.key === 'Enter') submit();
      }}
      style={{ display: 'flex', gap: 6, alignItems: 'center', marginTop: 4, flexWrap: 'wrap' }}
    >
      <select
        data-testid="req-add-constraint-kind"
        value={kind}
        onChange={(e) => setKind(e.target.value as 'assume' | 'require')}
        style={{ ...ATTR_INPUT, width: 82 }}
      >
        <option value="require">require</option>
        <option value="assume">assume</option>
      </select>
      <input
        data-testid="req-add-constraint-name"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="name (optional)"
        style={{ ...ATTR_INPUT, width: 110, fontFamily: 'var(--font-mono)' }}
      />
      <input
        data-testid="req-add-constraint-expr"
        autoFocus
        value={expr}
        onChange={(e) => setExpr(e.target.value)}
        placeholder="expression e.g. gap >= 4.0"
        style={{ ...ATTR_INPUT, flex: 1, minWidth: 140, fontFamily: 'var(--font-mono)' }}
      />
      <button
        type="button"
        data-testid="req-add-constraint-submit"
        onClick={submit}
        style={{
          border: '1px solid var(--accent)',
          background: 'var(--accent-tint)',
          color: 'var(--text-primary)',
          borderRadius: 4,
          padding: '2px 8px',
          fontSize: 'var(--text-xs)',
          cursor: 'pointer',
        }}
      >
        add
      </button>
    </div>
  );
}

/** `+ add attribute` (§7.7) — a compact name(+optional value) form that
 *  commits `attribute name = value;` into the requirement body. */
function AddAttributeRow({ elementId }: { elementId: string }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const key = cellKey(elementId, 'attribute_add');
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitAddAttribute } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;

  const close = () => {
    setOpen(false);
    setName('');
    setValue('');
    cancelEdit();
  };
  const submit = () => {
    const n = name.trim();
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(n)) return;
    commitAddAttribute(elementId, n, value.trim() || null);
    close();
  };

  if (!open) {
    return (
      <>
        <button
          type="button"
          data-testid="req-add-attribute"
          onClick={() => {
            if (beginEdit(key)) setOpen(true);
          }}
          style={ADD_BUTTON}
        >
          + add attribute
          {pendingKey === key && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
        </button>
        {failure && <CellErrorLine message={failure} />}
      </>
    );
  }
  return (
    <div
      onKeyDown={(e) => {
        if (e.key === 'Escape') close();
        if (e.key === 'Enter') submit();
      }}
      style={{ display: 'flex', gap: 6, alignItems: 'center', marginTop: 4 }}
    >
      <input
        data-testid="req-add-attribute-name"
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="name"
        style={{ ...ATTR_INPUT, width: 100, fontFamily: 'var(--font-mono)' }}
      />
      <span style={{ color: 'var(--text-muted)' }}>=</span>
      <input
        data-testid="req-add-attribute-value"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder="value (optional)"
        style={{ ...ATTR_INPUT, flex: 1 }}
      />
      <button
        type="button"
        data-testid="req-add-attribute-submit"
        onClick={submit}
        style={{
          border: '1px solid var(--accent)',
          background: 'var(--accent-tint)',
          color: 'var(--text-primary)',
          borderRadius: 4,
          padding: '2px 8px',
          fontSize: 'var(--text-xs)',
          cursor: 'pointer',
        }}
      >
        add
      </button>
    </div>
  );
}

/** Closed-state `+ add …` affordance — one shared shape (the settled
 *  "add" language): block-level so stacked affordances never run
 *  together on one line. */
const ADD_BUTTON: CSSProperties = {
  display: 'block',
  border: 'none',
  background: 'transparent',
  color: 'var(--text-muted)',
  fontSize: 'var(--text-xs)',
  cursor: 'pointer',
  padding: '2px 0',
  textAlign: 'left',
};

const ATTR_INPUT: CSSProperties = {
  boxSizing: 'border-box',
  background: 'var(--surface-canvas)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 4,
  padding: '3px 6px',
  fontSize: 'var(--text-sm)',
  outline: 'none',
};

/** Candidate definition kinds per role — spec-grounded (subject = Anything,
 *  narrowed to structural defs; actor/stakeholder = Part; concern = Concern). */
const ROLE_KINDS: Record<'subject' | 'actor' | 'stakeholder' | 'concern', string[]> = {
  subject: ['PartDefinition', 'ItemDefinition'],
  actor: ['PartDefinition'],
  stakeholder: ['PartDefinition'],
  concern: ['ConcernDefinition'],
};

/** `+ add <role>` (§7.7) — pick a definition (fuzzy over user-authored
 *  candidates), then a name pre-filled from the type (editable, so the exact
 *  written text is always visible — nothing silently derived). Commits
 *  `<keyword> <name> : <Type>;`. */
function AddRolePicker({
  elementId,
  role,
  label,
}: {
  elementId: string;
  role: 'subject' | 'actor' | 'stakeholder' | 'concern';
  label: string;
}) {
  const [open, setOpen] = useState(false);
  const [pickedValue, setPickedValue] = useState('');
  const [name, setName] = useState('');
  const key = cellKey(elementId, `role_${role}`);
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitAddRole } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;

  const candidates = useLinkTargetCandidates(open ? ROLE_KINDS[role] : null);
  const options = useMemo(() => buildPickerOptions(candidates.data ?? [], []), [candidates.data]);
  const fuzzy: FuzzyCandidate[] = useMemo(
    () => options.map((o) => (o.detail ? { value: o.value, detail: o.detail } : o.value)),
    [options],
  );
  const picked = options.find((o) => o.value === pickedValue) ?? null;

  const close = () => {
    setOpen(false);
    setPickedValue('');
    setName('');
    cancelEdit();
  };
  const submit = () => {
    const n = name.trim();
    if (!picked || !/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(n)) return;
    commitAddRole(elementId, role, picked.id, n);
    close();
  };

  if (!open) {
    return (
      <>
        <button
          type="button"
          data-testid={`req-add-role-${role}`}
          onClick={() => {
            if (beginEdit(key)) setOpen(true);
          }}
          style={ADD_BUTTON}
        >
          + add {label}
          {pendingKey === key && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
        </button>
        {failure && <CellErrorLine message={failure} />}
      </>
    );
  }
  return (
    <div
      onKeyDown={(e) => {
        if (e.key === 'Escape') close();
      }}
      style={{ display: 'flex', gap: 6, alignItems: 'center', marginTop: 4, flexWrap: 'wrap' }}
    >
      <div style={{ flex: 1, minWidth: 130 }}>
        <FuzzyCombobox
          value={pickedValue}
          onChange={(v) => {
            setPickedValue(v);
            // Pre-fill the name from the picked type (lowercase first char),
            // shown editable — the SE always sees the exact name written.
            const match = options.find((o) => o.value === v);
            if (match) setName(v.charAt(0).toLowerCase() + v.slice(1).replace(/::.*$/, ''));
          }}
          candidates={fuzzy}
          placeholder={candidates.isLoading ? 'loading…' : `${label} type`}
          testId={`req-role-${role}-type`}
          inputStyle={ATTR_INPUT}
        />
      </div>
      <span style={{ color: 'var(--text-muted)' }}>name</span>
      <input
        data-testid={`req-role-${role}-name`}
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="name"
        style={{ ...ATTR_INPUT, width: 90, fontFamily: 'var(--font-mono)' }}
      />
      <button
        type="button"
        data-testid={`req-role-${role}-submit`}
        disabled={picked === null}
        onClick={submit}
        style={{
          border: '1px solid var(--accent)',
          background: picked ? 'var(--accent-tint)' : 'transparent',
          color: picked ? 'var(--text-primary)' : 'var(--text-disabled)',
          borderRadius: 4,
          padding: '2px 8px',
          fontSize: 'var(--text-xs)',
          cursor: picked ? 'pointer' : 'default',
        }}
      >
        add
      </button>
    </div>
  );
}

/**
 * One referenced-attribute row — double-click the value enters inline
 * edit (v2 §7.5; `sysml.workspace.edit_attribute_value`). Only declared
 * values are editable: an attribute with no `= value` in source has no
 * value span to splice (assignment insertion is not a 2d surface).
 */
function AttributeValueRow({ attr }: { attr: RequirementAttribute }) {
  const key = cellKey(attr.id, 'attribute_value');
  const editing = useRequirementEditStore((s) => s.editingKey === key);
  const pending = useRequirementEditStore((s) => s.pendingKey === key);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitAttributeValue } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;
  const editable = attr.value !== null;

  return (
    <div data-testid="requirements-contract-value" style={CONTRACT_ROW}>
      <span style={CONTRACT_TAG}>value</span>
      {editing ? (
        <span style={{ flex: 1, minWidth: 0, display: 'flex', gap: 6 }}>
          <span>{attr.name ?? attr.id} =</span>
          <InlineTextEditor
            data-testid="req-attribute-editor"
            initial={attr.value ?? ''}
            onCommit={(value) => commitAttributeValue(attr.id, value)}
            onCancel={cancelEdit}
          />
        </span>
      ) : (
        <span
          onDoubleClick={
            editable
              ? (e) => {
                  e.stopPropagation();
                  beginEdit(key);
                }
              : undefined
          }
          title={editable ? 'double-click to edit the declared value' : undefined}
          style={editable ? { cursor: 'text' } : undefined}
        >
          {attr.name ?? attr.id}
          {attr.value !== null && ` = ${attr.value}`}
          {attr.live_value !== null && (
            <span style={{ color: 'var(--text-secondary)' }}> (live {attr.live_value})</span>
          )}
          {pending && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
          {failure && <CellErrorLine message={failure} />}
        </span>
      )}
    </div>
  );
}

/**
 * Narrative roles + rationale (§2.1 bucket separation) — context about
 * the requirement, never inputs to its verdict.
 */
function NarrativeBucket({ detail }: { detail: RequirementDetail }) {
  return (
    <div data-testid="requirements-narrative">
      {detail.actors.length > 0 && (
        <LinkGroup label="actors" refs={detail.actors} glyph={dotGlyph} emptyText="none" />
      )}
      <AddRolePicker elementId={detail.id} role="actor" label="actor" />
      {detail.stakeholders.length > 0 && (
        <LinkGroup
          label="stakeholders"
          refs={detail.stakeholders}
          glyph={dotGlyph}
          emptyText="none"
        />
      )}
      <AddRolePicker elementId={detail.id} role="stakeholder" label="stakeholder" />
      {detail.framed_concerns.length > 0 && (
        <LinkGroup
          label="framed concerns"
          refs={detail.framed_concerns}
          glyph={dotGlyph}
          emptyText="none"
        />
      )}
      <AddRolePicker elementId={detail.id} role="concern" label="concern" />
      <RationaleSection elementId={detail.id} rationale={detail.rationale} />
    </div>
  );
}

/** Rationale — read text (when present) + an `+ add` affordance (§7.7).
 *  v1 is add-only (a requirement may carry several rationale annotations). */
function RationaleSection({
  elementId,
  rationale,
}: {
  elementId: string;
  rationale: string | null;
}) {
  const [adding, setAdding] = useState(false);
  const key = cellKey(elementId, 'rationale');
  const pendingKey = useRequirementEditStore((s) => s.pendingKey);
  const failed = useRequirementEditStore((s) => s.failed);
  const beginEdit = useRequirementEditStore((s) => s.beginEdit);
  const cancelEdit = useRequirementEditStore((s) => s.cancelEdit);
  const { commitAddRationale } = useRequirementCellEdit();
  const failure = failed?.key === key ? failed.message : null;

  return (
    <>
      <div style={GROUP_LABEL}>rationale</div>
      {rationale !== null && (
        <div
          data-testid="requirements-rationale"
          style={{
            fontSize: 'var(--text-xs)',
            color: 'var(--text-secondary)',
            lineHeight: 1.5,
            whiteSpace: 'pre-line',
          }}
        >
          {rationale}
        </div>
      )}
      {adding ? (
        <InlineTextEditor
          data-testid="req-rationale-editor"
          initial=""
          placeholder="design rationale (single line)"
          onCommit={(text) => {
            const t = text.trim();
            if (t) commitAddRationale(elementId, t);
            setAdding(false);
          }}
          onCancel={() => {
            setAdding(false);
            cancelEdit();
          }}
        />
      ) : (
        <button
          type="button"
          data-testid="req-add-rationale"
          onClick={() => {
            if (beginEdit(key)) setAdding(true);
          }}
          style={ADD_BUTTON}
        >
          + add rationale
          {pendingKey === key && <CellBadge state="pending" />}
          {failure && <CellBadge state="failed" message={failure} />}
        </button>
      )}
      {failure && <CellErrorLine message={failure} />}
    </>
  );
}

/** One workflow-write input row: Enter records, cleared on success. */
function WorkflowWriteRow({
  tag,
  testid,
  placeholder,
  pending,
  onSubmit,
}: {
  tag: string;
  testid: string;
  placeholder: string;
  pending: boolean;
  onSubmit: (value: string) => void;
}) {
  const [draft, setDraft] = useState('');
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'baseline' }}>
      <span style={{ ...CONTRACT_TAG, width: 56 }}>{tag}</span>
      <input
        data-testid={testid}
        value={draft}
        placeholder={placeholder}
        disabled={pending}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && draft.trim() !== '') {
            onSubmit(draft.trim());
            setDraft('');
          }
        }}
        style={WORKFLOW_INPUT_STYLE}
      />
    </div>
  );
}

/**
 * Live workflow writes (v1.5b follow-on): approval transition, assign,
 * comment, sign-off — the four typed sidecar commands. All writes are
 * signed (ActorGate) and land in the append-only log rendered by
 * [`WorkflowHistory`] directly below. Approval is the Sans lifecycle
 * STEPPER (register ruling — never a chip, never a select); it shows the
 * folded current state (server-derived `from` — a stale rail can never
 * forge a transition's start) and a click records the transition.
 */
function WorkflowControls({ elementId }: { elementId: string }) {
  const state = useWorkflowState(elementId);
  const comment = useComment();
  const assign = useAssign();
  const setApproval = useSetApproval();
  const signOff = useSignOff();

  const currentApproval = state.data?.approval?.[0] ?? APPROVAL_INITIAL;
  const failed = [setApproval, assign, comment, signOff].find((m) => m.isError);

  return (
    <div data-testid="requirements-workflow-controls">
      <ActorGate prompt="Workflow actions are signed">
        {(actor) => (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <ApprovalStepper
              current={currentApproval}
              disabled={setApproval.isPending || state.data === undefined}
              onTransition={(to) => setApproval.mutate({ elementId, to, actor })}
            />
            <WorkflowWriteRow
              tag="assignee"
              testid="workflow-assign-input"
              placeholder={state.data?.assignee ?? 'unassigned — Enter to assign'}
              pending={assign.isPending}
              onSubmit={(assignee) => assign.mutate({ elementId, assignee, actor })}
            />
            <WorkflowWriteRow
              tag="comment"
              testid="workflow-comment-input"
              placeholder={`add a comment — signs as ${actor}, permanent`}
              pending={comment.isPending}
              onSubmit={(body) => comment.mutate({ elementId, body, actor })}
            />
            <WorkflowWriteRow
              tag="sign off"
              testid="workflow-signoff-input"
              placeholder="sign-off statement — Enter to sign"
              pending={signOff.isPending}
              onSubmit={(statement) => signOff.mutate({ elementId, statement, actor })}
            />
            {failed && (
              <div
                data-testid="workflow-controls-error"
                style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-error)' }}
              >
                Workflow write failed:{' '}
                {failed.error instanceof Error ? failed.error.message : 'unknown error'}
              </div>
            )}
          </div>
        )}
      </ActorGate>
    </div>
  );
}

function describeEvent(event: WorkflowEventWire): string {
  switch (event.kind) {
    case 'suspect_clearing_attestation':
      return `attested unchanged intent vs ${event.baseline_name} — "${event.rationale}"`;
    case 'comment':
      return event.body;
    case 'approval_state_changed':
      return `approval: ${event.from} → ${event.to}`;
    case 'sign_off_attestation':
      return `signed off — "${event.statement}"`;
    case 'engineer_assigned':
      return `assigned to ${event.assignee}`;
    case 'relinked':
      return `history re-linked from a prior identity — "${event.rationale}"`;
  }
}

function eventDate(timestampMs: number): string {
  const d = new Date(timestampMs);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** How many newest events stay visible in the pinned process zone; the
 *  rest sit behind the expand (hybrid refinement — glanceable state,
 *  on-demand log). */
const HISTORY_PINNED_COUNT = 2;

/** Workflow event history (v1.5b) — tool state, newest first. Hybrid:
 *  the newest rows are always visible; the full append-only log expands
 *  on demand. Nothing is ever dropped — the toggle names the total. */
function WorkflowHistory({ elementId }: { elementId: string }) {
  const log = useWorkflowLog(elementId);
  const state = useWorkflowState(elementId);
  const [expanded, setExpanded] = useState(false);
  const events = [...(log.data ?? [])].reverse();
  const shown = expanded ? events : events.slice(0, HISTORY_PINNED_COUNT);
  const hiddenCount = events.length - shown.length;
  const supersededSeqs = new Set(
    (state.data?.suspect_clearings ?? []).filter((c) => c.superseded).map((c) => c.seq),
  );

  return (
    <div data-testid="requirements-history">
      <div style={{ ...GROUP_LABEL, fontFamily: 'var(--font-body)' }}>history</div>
      {state.data?.orphaned && (
        <div
          data-testid="requirements-history-orphaned"
          style={{
            fontSize: 'var(--text-xs)',
            color: 'var(--severity-warning)',
            lineHeight: 1.5,
            marginBottom: 4,
          }}
        >
          Element identity changed — this history refers to a prior
          identity and was not automatically re-attached.
        </div>
      )}
      {log.isError ? (
        <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
          history unavailable
        </div>
      ) : events.length === 0 ? (
        <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-disabled)' }}>
          no workflow events
        </div>
      ) : (
        shown.map((event) => {
          const superseded =
            event.kind === 'suspect_clearing_attestation' && supersededSeqs.has(event.seq);
          return (
            <div
              key={event.seq}
              data-testid={`workflow-event-${event.seq}`}
              style={{
                fontSize: 'var(--text-xs)',
                lineHeight: 1.5,
                padding: '4px 0',
                borderBottom: '1px solid var(--border-hairline)',
                color: 'var(--text-secondary)',
              }}
            >
              <div
                style={{
                  display: 'flex',
                  gap: 6,
                  fontFamily: 'var(--font-mono)',
                  color: 'var(--text-muted)',
                }}
              >
                <span>{eventDate(event.timestamp_ms)}</span>
                <span>{event.actor}</span>
                {superseded && (
                  <span
                    data-testid={`workflow-event-${event.seq}-superseded`}
                    title="The requirement changed again after this attestation — it no longer clears suspicion"
                    style={{ color: 'var(--severity-warning)' }}
                  >
                    superseded
                  </span>
                )}
                <span style={{ flex: 1 }} />
                <span
                  title="Append-only record number in the workflow store"
                  style={{ color: 'var(--text-disabled)' }}
                >
                  #{event.seq}
                </span>
              </div>
              <div
                style={{
                  textDecoration: superseded ? 'line-through' : 'none',
                  color: superseded ? 'var(--text-muted)' : 'var(--text-secondary)',
                }}
              >
                {describeEvent(event)}
              </div>
            </div>
          );
        })
      )}
      {(hiddenCount > 0 || expanded) && (
        <button
          type="button"
          data-testid="workflow-history-toggle"
          onClick={() => setExpanded(!expanded)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            width: '100%',
            border: 'none',
            background: 'transparent',
            padding: '4px 0',
            fontSize: 'var(--text-xs)',
            color: 'var(--text-disabled)',
            cursor: 'pointer',
            textAlign: 'left',
          }}
        >
          <span aria-hidden>{expanded ? '▾' : '▸'}</span>
          full history —{' '}
          <span style={{ fontFamily: 'var(--font-mono)' }}>{events.length}</span> events
        </button>
      )}
    </div>
  );
}

registerRailContext({
  id: REQUIREMENTS_LINKS_CONTEXT_ID,
  title: 'Links',
  icon: 'link',
  render: () => <RequirementsLinksRailBody />,
});
