/**
 * useModalStore — the modal registry + open/close state (ninebar Phase 1).
 *
 * Frame chips, Cmd-K, and any future caller open a modal purely by id —
 * `openModal('configure-run', { targetId })` — so callers never
 * prop-drill a concrete `<Modal>` instance through the tree.
 * `<ModalHost/>` (mounted once in `AppShell`) looks the id up in the
 * registry and renders the matching `<Modal>` wrapped around the
 * registered body component. Phase 1 ships the registry + host only — no
 * concrete modal is registered yet; the config/action modals from the
 * plan's placement matrix (§1) register here as their phases land.
 */
import type { ComponentType } from 'react';
import { create } from 'zustand';

export interface ModalDescriptor<P = Record<string, unknown>> {
  id: string;
  title: string;
  component: ComponentType<P>;
}

const registry = new Map<string, ModalDescriptor<any>>();

export function registerModal<P>(descriptor: ModalDescriptor<P>): void {
  registry.set(descriptor.id, descriptor);
}

/** Lookup helper — returns `undefined` when no modal matches. */
export function getModal(id: string): ModalDescriptor<any> | undefined {
  return registry.get(id);
}

interface ModalStoreState {
  activeId: string | null;
  props: Record<string, unknown> | undefined;
  openModal: (id: string, props?: Record<string, unknown>) => void;
  closeModal: () => void;
}

export const useModalStore = create<ModalStoreState>((set) => ({
  activeId: null,
  props: undefined,

  openModal: (id, props) => set({ activeId: id, props }),
  closeModal: () => set({ activeId: null, props: undefined }),
}));
