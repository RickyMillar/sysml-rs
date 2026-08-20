/**
 * diagnostics — right-rail context (ninebar Phase 1, plan §1 row 7 /
 * Phase 1 task "Re-home the always-on panels").
 *
 * Re-homes the `DiagnosticsPanel` (the exact component the
 * `diagnostics` panel descriptor — `shared/panels/diagnostics.ts` —
 * renders in the old shell's utility drawer) into the rail without
 * forking it. Self-contained: pulls parse/semantic diagnostics via
 * react-query and navigates via react-router; its `width` prop
 * defaults to 300px, inside the `--rail-right-width` (320px) budget,
 * and its body scrolls internally.
 */
import { registerRailContext } from '../railRegistry';
import { DiagnosticsPanel } from '@/features/diagnostics/DiagnosticsPanel';

function DiagnosticsRailContext() {
  return (
    <div
      data-testid="rail-context-diagnostics"
      className="flex flex-col h-full overflow-hidden"
    >
      <DiagnosticsPanel width="100%" />
    </div>
  );
}

registerRailContext({
  id: 'diagnostics',
  title: 'Diagnostics',
  icon: 'bug_report',
  render: () => <DiagnosticsRailContext />,
});
