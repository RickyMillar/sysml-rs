/**
 * promoteToPlots — one-click "add this variable to the Plots
 * card" helper, used by sparklines in the tree + PartDetail key-signals.
 *
 * Thin wrapper over usePlotSelectionStore.addMany so:
 *   - callers don't need to know the selection shape,
 *   - adding a duplicate is a no-op (addMany dedupes internally),
 *   - the no-session case is a quiet no-op rather than an exception
 *     (the sparkline is still visible pre-session; clicking it
 *     before the session starts just becomes a noop).
 *
 * Pure function — takes the sessionId explicitly so tests can drive it
 * without booting a session. Consumers usually pull activeSessionId
 * from useSessionStore at the point of wiring.
 */
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';

export function promoteToPlots(
  name: string,
  sessionId: string | null,
): void {
  if (!sessionId) return;
  if (!name) return;
  usePlotSelectionStore.getState().addMany(sessionId, [name]);
}
