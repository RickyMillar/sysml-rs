/**
 * Workflow sidecar data layer: element history + folded state reads,
 * the suspect-clearing attestation write, and the re-verify action.
 *
 * Re-verify deliberately writes NO workflow event (steward ruling: it
 * reruns computed verification, it is not a review fact) — it calls
 * `sysml.workspace.verify` and refetches rows + suspects.
 */

import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useProjectId } from '@/features/baselines/queries';
import type {
  ElementWorkflowStateWire,
  VerificationAttestationWire,
  WorkflowEventWire,
} from './types';

export const workflowKeys = {
  log: (project: string, elementId: string) => ['workflow-log', project, elementId] as const,
  state: (project: string, elementId: string) => ['workflow-state', project, elementId] as const,
};

function command<T>(name: string, params: Record<string, unknown>): Promise<T> {
  return httpPost<T>('/api/command', { command: name, params });
}

/** One element's raw event history (oldest-first on the wire). */
export function useWorkflowLog(elementId: string | null) {
  const project = useProjectId();
  return useQuery({
    queryKey: workflowKeys.log(project ?? '', elementId ?? ''),
    enabled: project !== null && elementId !== null,
    queryFn: () =>
      command<WorkflowEventWire[]>('sysml.workflow.log', {
        project,
        element_id: elementId,
      }),
  });
}

/** One element's folded workflow state (superseded flags computed). */
export function useWorkflowState(elementId: string | null) {
  const project = useProjectId();
  return useQuery({
    queryKey: workflowKeys.state(project ?? '', elementId ?? ''),
    enabled: project !== null && elementId !== null,
    queryFn: () =>
      command<ElementWorkflowStateWire>('sysml.workflow.state', {
        project,
        element_id: elementId,
      }),
  });
}

/** One verification attestation carrying the element it was signed against
 *  — the flattened shape the history/attestation surfaces consume. */
export interface CaseVerificationAttestation extends VerificationAttestationWire {
  element_id: string;
}

/**
 * Verification attestations across a set of case elements — the data
 * behind the history timeline's attestation annotation strip (1c).
 *
 * Reuses the ONE workflow-state read (`sysml.workflow.state`, keyed by
 * `workflowKeys.state`): this is the plural form of `useWorkflowState`, not
 * a second fetch pattern. Attestations are folded per element, so the
 * strip is the union across the cases in view. Returns them flattened,
 * newest-first, with the element they were signed against.
 */
export function useVerificationAttestations(elementIds: string[]) {
  const project = useProjectId();
  const enabled = project !== null && elementIds.length > 0;
  const results = useQueries({
    queries: elementIds.map((elementId) => ({
      queryKey: workflowKeys.state(project ?? '', elementId),
      enabled,
      queryFn: () =>
        command<ElementWorkflowStateWire>('sysml.workflow.state', {
          project,
          element_id: elementId,
        }),
    })),
  });

  const attestations: CaseVerificationAttestation[] = [];
  for (let i = 0; i < results.length; i += 1) {
    const state = results[i].data;
    if (!state?.verification_attestations) continue;
    for (const record of state.verification_attestations) {
      attestations.push({ ...record, element_id: elementIds[i] });
    }
  }
  attestations.sort((a, b) => b.timestamp_ms - a.timestamp_ms);

  return {
    attestations,
    isLoading: enabled && results.some((r) => r.isLoading),
    isError: results.some((r) => r.isError),
  };
}

/**
 * Approval states across a set of elements — the plural read behind the
 * Verify latest-status table's approval column and the case view's
 * stepper (test-management study P4: verification cases reuse the ONE
 * approval sidecar; the state read is `sysml.workflow.state`, same key).
 *
 * "No event yet IS draft" — an element with a null folded `approval`
 * reads as `draft`, never a sentinel. Returns a map keyed by element id;
 * elements whose state is still loading are absent (callers render
 * nothing rather than a guessed state).
 */
export function useApprovalStates(elementIds: string[]) {
  const project = useProjectId();
  const enabled = project !== null && elementIds.length > 0;
  const results = useQueries({
    queries: elementIds.map((elementId) => ({
      queryKey: workflowKeys.state(project ?? '', elementId),
      enabled,
      queryFn: () =>
        command<ElementWorkflowStateWire>('sysml.workflow.state', {
          project,
          element_id: elementId,
        }),
    })),
  });

  const states = new Map<string, string>();
  for (let i = 0; i < results.length; i += 1) {
    const state = results[i].data;
    if (!state) continue;
    states.set(elementIds[i], state.approval?.[0] ?? 'draft');
  }
  return {
    states,
    isLoading: enabled && results.some((r) => r.isLoading),
  };
}

export interface AttestInput {
  elementId: string;
  baseline: string;
  rationale: string;
  actor: string;
}

/** "Attest unchanged intent" — on success the suspect set and this
 *  element's history/state are stale; invalidate them. */
export function useAttestClearing() {
  const project = useProjectId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: AttestInput) => {
      if (!project) throw new Error('no workspace loaded');
      return command<WorkflowEventWire>('sysml.workflow.attest_suspect_clearing', {
        project,
        element_id: input.elementId,
        baseline: input.baseline,
        rationale: input.rationale,
        actor: input.actor,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['suspects'] });
      void queryClient.invalidateQueries({ queryKey: ['workflow-log'] });
      void queryClient.invalidateQueries({ queryKey: ['workflow-state'] });
    },
  });
}

/** Shared shape of the four per-kind workflow writes (steward ruling:
 *  typed commands, never a generic append). Each takes the element, its
 *  one payload field, and the explicit actor; on success the element's
 *  history and folded state are stale. */
function useWorkflowWrite<TInput>(
  command_name: string,
  toParams: (input: TInput) => Record<string, unknown>,
) {
  const project = useProjectId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: TInput) => {
      if (!project) throw new Error('no workspace loaded');
      return command<WorkflowEventWire>(command_name, { project, ...toParams(input) });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['workflow-log'] });
      void queryClient.invalidateQueries({ queryKey: ['workflow-state'] });
    },
  });
}

/** Record a review comment on an element. */
export function useComment() {
  return useWorkflowWrite<{ elementId: string; body: string; actor: string }>(
    'sysml.workflow.comment',
    (i) => ({ element_id: i.elementId, body: i.body, actor: i.actor }),
  );
}

/** Assign an engineer to an element. */
export function useAssign() {
  return useWorkflowWrite<{ elementId: string; assignee: string; actor: string }>(
    'sysml.workflow.assign',
    (i) => ({ element_id: i.elementId, assignee: i.assignee, actor: i.actor }),
  );
}

/** Transition an element's approval state. `from` is derived
 *  server-side from the folded log — a stale client can never forge a
 *  transition's starting point. */
export function useSetApproval() {
  return useWorkflowWrite<{ elementId: string; to: string; actor: string }>(
    'sysml.workflow.set_approval',
    (i) => ({ element_id: i.elementId, to: i.to, actor: i.actor }),
  );
}

/** Record a sign-off attestation statement. */
export function useSignOff() {
  return useWorkflowWrite<{ elementId: string; statement: string; actor: string }>(
    'sysml.workflow.sign_off',
    (i) => ({ element_id: i.elementId, statement: i.statement, actor: i.actor }),
  );
}

/** Re-run workspace verification, then refetch everything whose truth
 *  it feeds (rows carry the verification rollup). */
export function useReverify() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () =>
      command<unknown>('sysml.workspace.verify', {}),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['requirement-rows'] });
      void queryClient.invalidateQueries({ queryKey: ['suspects'] });
    },
  });
}
