/**
 * CreateViewModal — the guided create-view flow for workspaces that
 * ALREADY declare views (the view-less canvas state only appears when
 * none exist, so authoring view #2 needs its own door).
 *
 * Placement per the §6 authoring loop: authoring belongs to Browse, not
 * Run — the visible opener lives in the Views rail context header
 * ("+ New"), with Cmd-K (`modal.create-view`) as the universal path.
 * Run's canvas picker stays clean.
 *
 * The modal is the SAME `CreateViewPrompt` component the view-less
 * state renders — one flow, two doors. Target prefills from the current
 * selection when there is one.
 */
import { registerModal } from '@/shared/overlays/modalStore';
import { useSelectionStore } from '@/features/selection/store';
import { CreateViewPrompt } from './CreateViewPrompt';

export const CREATE_VIEW_MODAL_ID = 'create-view';

function CreateViewModal() {
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  return (
    <div style={{ minWidth: 480, maxWidth: 560 }}>
      <CreateViewPrompt targetId={selectedElementId ?? ''} context="modal" />
    </div>
  );
}

registerModal({
  id: CREATE_VIEW_MODAL_ID,
  title: 'Create view',
  component: CreateViewModal,
});
