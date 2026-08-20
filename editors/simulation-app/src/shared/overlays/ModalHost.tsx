/**
 * ModalHost — mounted once in `AppShell`; renders whichever modal
 * `useModalStore` currently has active by looking it up in the registry
 * (see `modalStore.ts`). Renders nothing when no modal is registered
 * under the active id, or when no modal is open.
 */
import { Modal } from './Modal';
import { getModal, useModalStore } from './modalStore';

export function ModalHost() {
  const activeId = useModalStore((s) => s.activeId);
  const props = useModalStore((s) => s.props);
  const closeModal = useModalStore((s) => s.closeModal);

  if (!activeId) return null;
  const descriptor = getModal(activeId);
  if (!descriptor) return null;

  const Body = descriptor.component;
  return (
    <Modal open onClose={closeModal} title={descriptor.title}>
      <Body {...(props ?? {})} />
    </Modal>
  );
}
