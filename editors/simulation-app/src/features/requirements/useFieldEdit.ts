/**
 * useApplyFieldEdit — the six-step writeback loop (workbench design §7.5):
 *
 *   1. compute   POST the per-field command → `FieldEditComputed`
 *   2. buffer    lazily seed the workspace-store buffer (`loadFile`) —
 *                hydrate registers files with `source: ''` (SourcePanel
 *                "Bug A"); an unseeded buffer can't be guard-checked
 *   3. guard     buffer slice must equal `expected_old_text`, else FAIL
 *                with nothing written (fieldEdit.applyGuardedEdit)
 *   4. splice    `updateSource` (dirty; editor owns save). Monaco is
 *                store-controlled, so any open editor updates and fires
 *                its own LSP didChange
 *   5. sync      POST `sysml.load_source` — the salsa overlay write the
 *                REST commands read from; synchronous, so its return IS
 *                the reparse-complete signal. Idempotent with Monaco's
 *                didChange (identical full text)
 *   6. refetch   invalidate rows/detail/source queries — the refetched
 *                row is the confirmation
 *
 * ONE edit in flight (BINDING): callers must route edit entry through
 * `useRequirementEditStore.beginEdit`, and this hook moves the cell
 * through pending → confirmed/failed.
 */

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { loadFile } from '@/shared/api/model';
import { useWorkspaceStore } from '@/store/workspace';
import { applyGuardedEdit } from './fieldEdit';
import type { FieldEditComputed } from './fieldEdit';
import { requirementKeys } from './queries';
import { cellKey, useRequirementEditStore } from './editStore';

export interface FieldEditRequest {
  /** Cell key (`editStore.cellKey`) driving pending/failed state. */
  key: string;
  /** e.g. `sysml.workspace.edit_requirement_doc`. */
  command: string;
  params: Record<string, unknown>;
}

/** Find the workspace-store buffer key for a backend file uri. The
 *  backend keys files by path; tolerate a `file://` prefix on either
 *  side rather than assuming the two stores always agree byte-for-byte. */
function resolveBufferUri(
  loadedFiles: Map<string, { uri: string }>,
  uri: string,
): string | null {
  if (loadedFiles.has(uri)) return uri;
  const stripped = uri.replace(/^file:\/\//, '');
  if (loadedFiles.has(stripped)) return stripped;
  const prefixed = `file://${uri}`;
  if (loadedFiles.has(prefixed)) return prefixed;
  return null;
}

async function performFieldEdit(req: FieldEditRequest): Promise<FieldEditComputed> {
  // 1. compute
  const computed = await httpPost<FieldEditComputed>('/api/command', {
    command: req.command,
    params: req.params,
  });

  // 2. buffer
  const ws = useWorkspaceStore.getState();
  const bufUri = resolveBufferUri(ws.loadedFiles, computed.uri);
  if (bufUri === null) {
    throw new Error(
      `file is not tracked in the loaded workspace: ${computed.uri} — reload the workspace and retry`,
    );
  }
  if (ws.loadedFiles.get(bufUri)!.source === '') {
    const fetched = await loadFile(bufUri);
    useWorkspaceStore.getState().seedSource(bufUri, fetched.source);
  }
  const source = useWorkspaceStore.getState().loadedFiles.get(bufUri)!.source;

  // 3. guard + splice text
  const result = applyGuardedEdit(source, computed.edit);
  if (!result.ok) throw new Error(result.reason);

  // 4. splice into the buffer (dirty; editor owns save)
  useWorkspaceStore.getState().updateSource(bufUri, result.next);

  // 5. sync the analysis host (reparse-complete when this returns)
  await httpPost('/api/command', {
    command: 'sysml.load_source',
    params: { uri: computed.uri, source: result.next },
  });

  return computed;
}

/**
 * Field-level commit helpers — the edit/add routing lives HERE, not in
 * components: `edit_*` commands fail hard when the construct is absent
 * (creation is a different act, §7.3), so the router picks `add_*` off
 * the row's current value.
 */
export function useRequirementCellEdit() {
  const apply = useApplyFieldEdit();

  return {
    /** Doc text — `edit` when the row has statement text, `add` when not. */
    commitDoc(row: { id: string; text: string | null }, newText: string) {
      apply.mutate({
        key: cellKey(row.id, 'doc'),
        command:
          row.text === null
            ? 'sysml.workspace.add_requirement_doc'
            : 'sysml.workspace.edit_requirement_doc',
        params: { element_id: row.id, new_text: newText },
      });
    },
    /** Maturity — `edit` when @StatusInfo exists, `add` when not. */
    commitMaturity(row: { id: string; maturity: string | null }, status: string) {
      apply.mutate({
        key: cellKey(row.id, 'maturity'),
        command:
          row.maturity === null
            ? 'sysml.workspace.add_requirement_maturity'
            : 'sysml.workspace.edit_requirement_maturity',
        params: { element_id: row.id, status },
      });
    },
    /** Add a design-rationale annotation to a requirement (§7.7). */
    commitAddRationale(elementId: string, text: string) {
      apply.mutate({
        key: cellKey(elementId, 'rationale'),
        command: 'sysml.workspace.add_rationale',
        params: { element_id: elementId, text },
      });
    },
    /** Add a typed role (subject/actor/stakeholder/concern) to a requirement (§7.7). */
    commitAddRole(
      elementId: string,
      role: 'subject' | 'actor' | 'stakeholder' | 'concern',
      typeId: string,
      name: string,
    ) {
      apply.mutate({
        key: cellKey(elementId, `role_${role}`),
        command: 'sysml.workspace.add_requirement_role',
        params: { requirement_id: elementId, role, type_id: typeId, name },
      });
    },
    /** Add an assume/require constraint to a requirement (§7.7). */
    commitAddConstraint(
      elementId: string,
      kind: 'assume' | 'require',
      expr: string,
      name: string | null,
    ) {
      apply.mutate({
        key: cellKey(elementId, 'constraint_add'),
        command: 'sysml.workspace.add_constraint',
        params: { element_id: elementId, kind, expr, ...(name ? { name } : {}) },
      });
    },
    /** Add a new attribute to a requirement (§7.7). */
    commitAddAttribute(elementId: string, name: string, value: string | null) {
      apply.mutate({
        key: cellKey(elementId, 'attribute_add'),
        command: 'sysml.workspace.add_attribute',
        params: { element_id: elementId, name, ...(value ? { value } : {}) },
      });
    },
    /** Attribute value — always an edit (the attribute row exists). */
    commitAttributeValue(attributeId: string, newValue: string) {
      apply.mutate({
        key: cellKey(attributeId, 'attribute_value'),
        command: 'sysml.workspace.edit_attribute_value',
        params: { element_id: attributeId, new_value: newValue },
      });
    },
    /**
     * R5 link writing (§7.6) — one command per relationship; the
     * direction-symmetric "derived to" add reuses `add_derive_link` with
     * the roles swapped by the caller. Cross-file is inherent (the edit
     * lands in the PICKED element's file) and the loop is uri-agnostic.
     */
    commitSatisfyLink(requirementId: string, subjectId: string) {
      apply.mutate({
        key: cellKey(requirementId, 'link_satisfy'),
        command: 'sysml.workspace.add_satisfy_link',
        params: { requirement_id: requirementId, subject_id: subjectId },
      });
    },
    commitVerifyLink(requirementId: string, caseId: string) {
      apply.mutate({
        key: cellKey(requirementId, 'link_verify'),
        command: 'sysml.workspace.add_verify_link',
        params: { requirement_id: requirementId, case_id: caseId },
      });
    },
    /** `requirementId` refines `refinedId` (the row's outgoing `refines`). */
    commitRefineLink(requirementId: string, refinedId: string) {
      apply.mutate({
        key: cellKey(requirementId, 'link_refine'),
        command: 'sysml.workspace.add_refine_link',
        params: { requirement_id: requirementId, refined_id: refinedId },
      });
    },
    /** `derivedId` derives FROM `originalId`; the pending badge rides the
     *  ROW+GROUP the picker was opened on (`badgeRowId`/`badgeField` —
     *  both derive directions share this one command). */
    commitDeriveLink(
      derivedId: string,
      originalId: string,
      badgeRowId: string,
      badgeField: 'link_derive' | 'link_derive_to' = 'link_derive',
    ) {
      apply.mutate({
        key: cellKey(badgeRowId, badgeField),
        command: 'sysml.workspace.add_derive_link',
        params: { requirement_id: derivedId, original_id: originalId },
      });
    },
    /** Guided create — new requirement under a package/requirement. */
    commitCreate(
      parentId: string,
      name: string,
      shortName: string | null,
      doc: string | null,
    ) {
      apply.mutate({
        key: cellKey(parentId, 'create'),
        command: 'sysml.workspace.create_requirement',
        params: {
          parent_id: parentId,
          name,
          ...(shortName ? { short_name: shortName } : {}),
          ...(doc ? { doc } : {}),
        },
      });
    },
  };
}

export function useApplyFieldEdit() {
  const queryClient = useQueryClient();
  const markPending = useRequirementEditStore((s) => s.markPending);
  const markConfirmed = useRequirementEditStore((s) => s.markConfirmed);
  const markFailed = useRequirementEditStore((s) => s.markFailed);

  return useMutation({
    mutationFn: performFieldEdit,
    onMutate: (req: FieldEditRequest) => markPending(req.key),
    onSuccess: async (computed, req) => {
      // 6. refetch — rows first so the badge clears onto fresh data.
      await queryClient.invalidateQueries({ queryKey: requirementKeys.rows });
      await queryClient.invalidateQueries({
        queryKey: requirementKeys.detail(computed.element_id),
      });
      await queryClient.invalidateQueries({ queryKey: ['element-source'] });
      markConfirmed(req.key);
    },
    onError: (error: unknown, req) => {
      markFailed(req.key, error instanceof Error ? error.message : String(error));
    },
  });
}
