/**
 * KpiManagerModal — the old workbench KPIs tab re-homed as a modal
 * (ninebar Phase 3 W3-A; plan §1 row 4b: the strip carries the compact
 * meter ROW, management is an overlay). Renders `KpisTab` verbatim over
 * live store data — one surface, two shells (flag-off keeps the tab).
 */
import { registerModal } from '@/shared/overlays/modalStore';
import { useTick, useTimeMs } from '@/features/sessions/sessionLiveStore';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { KpisTab } from './KpisTab';

export const KPI_MANAGER_MODAL_ID = 'kpi-manager';

function KpiManagerModal() {
  // Re-render per tick so live values in the manager stay current.
  useTick();
  const timeMs = useTimeMs();
  const timeSeries = useTimeSeriesStore.getState().getTimeSeries();
  return (
    <div style={{ minWidth: 520, maxWidth: 680 }}>
      <KpisTab timeSeries={timeSeries} clockTime={(timeMs ?? 0) / 1000} expanded />
    </div>
  );
}

registerModal({
  id: KPI_MANAGER_MODAL_ID,
  title: 'KPIs',
  component: KpiManagerModal,
});
