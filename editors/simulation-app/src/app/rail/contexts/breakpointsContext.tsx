/**
 * breakpoints — right-rail context (ninebar Phase 1, plan §1 row 11b /
 * Phase 1 task "Re-home the always-on panels").
 *
 * Re-homes the `BreakpointsPanel` (the exact component the
 * `breakpoints` panel descriptor — `shared/panels/breakpoints.ts` —
 * renders in the old shell's utility drawer) into the rail without
 * forking it. The panel is self-contained (pulls from its own store)
 * and defaults its `width` prop to 280px, comfortably inside the
 * `--rail-right-width` (320px) budget; its own body scrolls
 * internally, so this wrapper only needs to fill the rail's height.
 */
import { registerRailContext } from '../railRegistry';
import { BreakpointsPanel } from '@/features/breakpoints/BreakpointsPanel';

function BreakpointsRailContext() {
  return (
    <div
      data-testid="rail-context-breakpoints"
      className="flex flex-col h-full overflow-hidden"
    >
      <BreakpointsPanel width="100%" />
    </div>
  );
}

registerRailContext({
  id: 'breakpoints',
  title: 'Breakpoints',
  icon: 'radio_button_checked',
  render: () => <BreakpointsRailContext />,
});
