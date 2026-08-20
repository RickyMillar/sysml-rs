/**
 * sessionCreate — map a run target to `sysml.sessions.create` parameters.
 *
 * The backend exposes ONE creation entry point, `sysml.sessions.create(uri,
 * target?, dt_ms?, max_time_ms?) -> SessionSummary`. The server resolves
 * `target` against the workspace graph and infers the `SessionKind`
 * (simulation / action / orchestrator) itself — see
 *
 * So the frontend no longer picks a `*.start` command or inspects model
 * capabilities. It only says *what to run*: a named element (the server
 * decides how) or the whole workspace. This replaces the old
 * `commandForTarget` capability→command decision tree, which duplicated
 * backend dispatch on the client.
 */

import type { RunTargetSummary } from '@/features/run-targets/types';

/** Parameters for `sysml.sessions.create`. */
export interface CreateSessionParams {
  /** Model URI, or `__workspace__` for the merged workspace. */
  uri: string;
  /**
   * Optional element name to run. Omit (or `__workspace__`) to run the whole
   * multi-subsystem workspace orchestrator. A state-machine name runs a
   * simulation (or the orchestrator when the model has coupled dynamics); an
   * action name runs an action — the server decides.
   */
  target?: string;
  /** Time step in ms (orchestrator sessions). */
  dtMs?: number;
  /** Max simulation time in ms (orchestrator sessions). */
  maxTimeMs?: number;
  /**
   * Scenario overrides applied while the session is BUILT, so they hold from
   * the first tick — `[key, value]` pairs on the wire.
   *
   * This is the difference between "the severe run" and "a nominal run I
   * changed part-way through". `draftOverrides` (session store) is the second
   * thing: it drains into the next STEP, so ticks before it ran under the old
   * value. Only a create-time override can be quoted as the scenario a whole
   * trajectory was produced under, which is why the two are separate fields
   * here and separate fields on `SessionSummary`.
   *
   * Orchestrator sessions only; the backend refuses rather than silently
   * dropping them for a single-SM or action target.
   */
  overrides?: [string, string][];
}

/**
 * Derive `sessions.create` parameters from the selected run target.
 *
 * - No target → run the whole workspace orchestrator (`__workspace__`).
 * - A target → pass its source URI and name; the server infers the kind.
 *   (Targets that don't name a runnable element, e.g. a part or case, resolve
 *   server-side to the workspace orchestrator.)
 */
export function createParamsForTarget(
  target: RunTargetSummary | null,
  dtMs?: number,
  overrides?: [string, string][],
): CreateSessionParams {
  // An empty list is "no scenario", not "an empty scenario". The KEY is
  // omitted rather than set to `undefined`: an explicit `overrides: undefined`
  // is still an own property, so callers that assert on the params shape (and
  // anything that spreads them onto a request body) would see a scenario field
  // where none was configured.
  const scenario = overrides && overrides.length > 0 ? { overrides } : {};
  if (!target) return { uri: '__workspace__', dtMs, ...scenario };
  return {
    uri: target.uri,
    target: target.name ?? undefined,
    dtMs,
    ...scenario,
  };
}
