/**
 * railCommands — client-side command-palette actions for the right rail
 * (ninebar Phase 1, plan §1 row 15 / Phase 1 task "Promote Cmd-K to
 * first-class — the long-tail command home").
 *
 * The palette is otherwise a pure proxy over backend `sysml.*` commands:
 * `commandCatalog.ts` fetches the catalog from `GET /commands` and
 * dispatches picks via `POST /api/command`. Rail open/close/pin/unpin
 * are pure client UI state (`useRightRailStore`) — there is no backend
 * command for them and there shouldn't be one. This module defines them
 * as `CommandMeta`-shaped entries with a `clientAction` instead of
 * backend params (`category: 'Client'`, see `commandCatalog.ts`),
 * merged into the palette's catalog by `CommandPalette.tsx` alongside
 * the fetched backend list. This is the minimal extension the palette's
 * backend-RPC-only architecture needed to host a client action at all —
 * see the doc comment on `CommandPalette.tsx`'s catalog-merge + select
 * handling for the other half.
 */
import type { CommandMeta } from './commandCatalog';
import { useRightRailStore } from '@/app/rail/railStore';

function railCommand(
  name: string,
  description: string,
  run: () => void,
): CommandMeta {
  return {
    name,
    category: 'Client',
    description,
    params: [],
    returns: 'void',
    stateful: false,
    clientAction: run,
  };
}

/**
 * The rail open/close/pin/unpin actions, in display order. A bare
 * function (not a hook) — zustand stores expose `getState()`/`setState()`
 * statically, so these run fine outside React (e.g. from a click
 * handler in the palette's picker list).
 */
export function getRailCommands(): CommandMeta[] {
  return [
    railCommand('rail.open.variables', 'Open variables rail', () =>
      useRightRailStore.getState().open('variables'),
    ),
    railCommand('rail.open.breakpoints', 'Open breakpoints rail', () =>
      useRightRailStore.getState().open('breakpoints'),
    ),
    railCommand('rail.open.diagnostics', 'Open diagnostics rail', () =>
      useRightRailStore.getState().open('diagnostics'),
    ),
    railCommand('rail.open.views', 'Open views rail', () =>
      useRightRailStore.getState().open('views'),
    ),
    // Phase 6 (plan row 24): the archive left its interim rail home —
    // the history-browser modal is the permanent one.
    railCommand('modal.history', 'Open history (archived runs, golden management)', () => {
      void Promise.all([
        import('@/shared/overlays/modalStore'),
        import('@/features/archive/HistoryBrowserModal'),
      ]).then(([{ useModalStore }, { HISTORY_BROWSER_MODAL_ID }]) =>
        useModalStore.getState().openModal(HISTORY_BROWSER_MODAL_ID),
      );
    }),
    railCommand('rail.open.inspector', 'Open inspector rail', () =>
      useRightRailStore.getState().open('inspector'),
    ),
    railCommand(
      'rail.open.requirements-links',
      'Open requirement links rail (Requirements workbench)',
      () => {
        // The context registers as a side effect of the Requirements
        // workflow module — lazy-import first (same pattern as the
        // modal openers below) so the palette works before the route
        // has ever been visited.
        void import('@/workflows/requirements/requirementsLinksRailContext').then(
          () => useRightRailStore.getState().open('requirements-links'),
        );
      },
    ),
    railCommand('rail.close', 'Close rail', () =>
      useRightRailStore.getState().close(),
    ),
    railCommand(
      'rail.pin',
      'Pin rail (promotes whichever context is currently open)',
      () => {
        const { transient, pin } = useRightRailStore.getState();
        if (transient) pin(transient);
      },
    ),
    railCommand('rail.unpin', 'Unpin rail', () =>
      useRightRailStore.getState().unpin(),
    ),
    // Overlay openers (ninebar Phase 3 W3-A/C) — modals by id; lazy
    // imports keep the palette from pulling results-feature code into
    // its own chunk.
    railCommand('modal.equations', 'Open equations (expression view)', () => {
      void Promise.all([
        import('@/shared/overlays/modalStore'),
        import('@/features/results/equations/EquationsModal'),
      ]).then(([{ useModalStore }, { EQUATIONS_MODAL_ID }]) =>
        useModalStore.getState().openModal(EQUATIONS_MODAL_ID),
      );
    }),
    railCommand('modal.create-view', 'Create a new view (guided)', () => {
      void Promise.all([
        import('@/shared/overlays/modalStore'),
        import('@/components/diagram/CreateViewModal'),
      ]).then(([{ useModalStore }, { CREATE_VIEW_MODAL_ID }]) =>
        useModalStore.getState().openModal(CREATE_VIEW_MODAL_ID),
      );
    }),
    railCommand('modal.kpis', 'Open KPI manager', () => {
      void Promise.all([
        import('@/shared/overlays/modalStore'),
        import('@/features/results/kpis/KpiManagerModal'),
      ]).then(([{ useModalStore }, { KPI_MANAGER_MODAL_ID }]) =>
        useModalStore.getState().openModal(KPI_MANAGER_MODAL_ID),
      );
    }),
    // Phase 6 — Compare is a Simulate mode with no nav tab; Cmd-K is
    // one of its three doors (frame session switcher + promote flows
    // are the others). Navigation resolves in the palette component
    // via `navigateTo` (router context lives there, not here).
    {
      name: 'open.compare',
      category: 'Client',
      description: 'Open Compare (multi-session diff canvas)',
      params: [],
      returns: 'void',
      stateful: false,
      navigateTo: '/run/compare',
    },
  ];
}
