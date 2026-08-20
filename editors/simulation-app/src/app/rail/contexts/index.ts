/**
 * Side-effect barrel — importing this module registers every built-in
 * rail context (see `railRegistry.ts`). `RightRail.tsx` imports it once
 * so the host owns its own registration bootstrap, decoupled from
 * `AppShell`.
 *
 * ninebar Phase 1 "Re-home the always-on panels": variables /
 * breakpoints / diagnostics re-home the Run-page resident panels;
 * views is a Browse-bound interim home (Browse itself lands in
 * Phase 7 — see its doc comment). The interim `archive` context moved
 * to its permanent home in Phase 6: the history-browser modal
 * (`features/archive/HistoryBrowserModal.tsx`, plan §1 row 24).
 *
 * ninebar Phase 1.5 adds `inspector` — the Browse floor's
 * selection-driven element detail panel (see `inspectorContext.tsx`).
 */
import './streamStatusContext';
import './variablesContext';
import './breakpointsContext';
import './diagnosticsContext';
import './viewsContext';
import './inspectorContext';
