/**
 * variables — right-rail context (ninebar Phase 1, plan §1 row 10 /
 * Phase 1 task "Re-home the always-on panels").
 *
 * Re-homes the legacy Variables pane into the rail without forking it:
 * this wraps the exact `VariablesPane` the `variables` panel descriptor
 * (`shared/panels/variables.ts`) renders in the old shell's utility
 * drawer. `VariablesPane` was made dual-mount-safe in `1744188c` (its
 * keyboard-shortcut singleton and re-render guardrail tolerate being
 * mounted alongside the drawer instance), so both hosts can render it
 * concurrently during the Phase 1-7 transition without racing.
 *
 * `VariablesPane` already constrains its own width (`minWidth: 300`,
 * `maxWidth: 380` unexpanded — see the component) and scrolls its body
 * internally (`data-testid="variables-pane-body"`), so this wrapper only
 * needs to fill the rail's height and let the pane's own scroll region
 * do the rest.
 */
import { registerRailContext } from '../railRegistry';
import { VariablesPane } from '@/features/variables/VariablesPane';

function VariablesRailContext() {
  return (
    <div
      data-testid="rail-context-variables"
      className="flex flex-col h-full overflow-hidden"
    >
      <VariablesPane />
    </div>
  );
}

registerRailContext({
  id: 'variables',
  title: 'Variables',
  icon: 'data_object',
  render: () => <VariablesRailContext />,
});
