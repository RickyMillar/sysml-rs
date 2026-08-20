/**
 * Phase 3 — Source-preview surfaces.
 *
 * `useSourcePreview` is a hover-intent debouncer. Panels feed it the
 * current hover state of their row + the element they're hovering
 * over; the hook returns `armed: true` once the user has dwelled long
 * enough that we want to show the preview popover.
 *
 * Data fetching itself stays in `useGetSource` (used by `<SneakPeek>`),
 * so a click after a hover — or vice versa — reuses the same cached
 * response. The hook stays thin on purpose: it owns *when* to show,
 * not what to fetch.
 */
import { useEffect, useState } from 'react';

export interface UseSourcePreviewOptions {
  /**
   * Whether the consumer is currently hovering / focusing the trigger.
   * The popover stays disarmed until this has been true for
   * `debounceMs` continuously, so scrolling past rows doesn't open a
   * cascade of previews.
   */
  hovering: boolean;
  /**
   * Milliseconds the user must dwell before the popover arms.
   * Defaults to 180ms — tight enough to feel responsive, loose
   * enough to ignore mouse-scroll passes.
   */
  debounceMs?: number;
}

export interface SourcePreviewArmed {
  /** True once the dwell timer has elapsed and `uri` + `id` are set. */
  armed: boolean;
}

const DEFAULT_DEBOUNCE_MS = 180;

export function useSourcePreview(
  uri: string | null,
  id: string | null,
  { hovering, debounceMs = DEFAULT_DEBOUNCE_MS }: UseSourcePreviewOptions,
): SourcePreviewArmed {
  const [dwellElapsed, setDwellElapsed] = useState(false);

  useEffect(() => {
    if (!hovering) {
      setDwellElapsed(false);
      return;
    }
    const t = window.setTimeout(() => setDwellElapsed(true), debounceMs);
    return () => {
      window.clearTimeout(t);
    };
  }, [hovering, debounceMs]);

  return {
    armed: dwellElapsed && !!uri && !!id,
  };
}
