/**
 * EquationsModal — the old workbench Equations tab re-homed as a modal
 * (ninebar Phase 3 W3-C; plan §1 row 4c: "deep, occasional; not
 * always-on"). Renders `EquationsTab` verbatim over the same data the
 * workbench fed it — one surface, two shells (flag-off keeps the tab).
 */
import { registerModal } from '@/shared/overlays/modalStore';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionDetail } from '@/features/sessions/queries';
import { useSelectionStore } from '@/features/selection/store';
import { useTick } from '@/features/sessions/sessionLiveStore';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useExpressionAst } from '../useExpressionAst';
import { EquationsTab } from './EquationsTab';

export const EQUATIONS_MODAL_ID = 'equations';

function EquationsModal() {
  useTick(); // live values refresh per tick
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const { data: sessionDetail } = useSessionDetail(activeSessionId);
  const sessionUri = sessionDetail?.summary?.uri ?? null;
  const { data: results = [], isLoading, error } = useExpressionAst(sessionUri);
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const timeSeries = useTimeSeriesStore.getState().getTimeSeries();

  return (
    <div style={{ minWidth: 560, maxWidth: 720 }}>
      <EquationsTab
        results={results}
        timeSeries={timeSeries}
        uri={sessionUri}
        loading={isLoading}
        error={error ? String(error) : null}
        selectedElementId={selectedElementId}
        expanded
      />
    </div>
  );
}

registerModal({
  id: EQUATIONS_MODAL_ID,
  title: 'Equations',
  component: EquationsModal,
});
