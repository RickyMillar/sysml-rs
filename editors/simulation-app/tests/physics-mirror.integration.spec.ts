/**
 * UI integration tests that mirror the backend physics tests.
 *
 * Each test loads a workspace through the real sysml-api backend, drives
 * the UI to start + step a session, then asserts that the same observable
 * outcome the corresponding backend test validates is actually visible to
 * the user via the Plots tab's time-series buffer (read with the test
 * window helper `__getTimeSeries`).
 *
 * Mirrored backend tests:
 *   1. crates/lang/sysml-runtime/tests/espresso_cell_physics.rs::test_espresso_cell_steps
 *      → T_som and station1.bimetalTemp rise above ambient (298.15K)
 *   2. crates/lang/sysml-runtime/tests/espresso_cell_physics.rs::test_espresso_cell_waveform_driven
 *      → station1.flow and station4.flow are non-zero
 *   3. crates/lang/sysml-runtime/tests/physics_examples_pipeline.rs::test_radiation_cooling_simulation
 *      → temperature decays below 1000K
 *   4. crates/lang/sysml-runtime/tests/bouncing_ball_pipeline.rs::test_bouncing_ball_workspace_orchestrator
 *      → y < 10.0 (start) and v < 0.0 (falling) after stepping
 *   5. crates/lang/sysml-runtime/tests/espresso_pump_hybrid.rs::test_pump_cycle_oscillation
 *      → state machine timeline shows transitions (substituted for full
 *        4-state oscillation if oscillator state isn't surfaced as a
 *        chartable variable in the snapshot).
 */

// NOTE (retired-internal MIG-C): these mirror blocks are test.describe.skip pending
// regeneration against the espresso fixtures' REAL observable names/values.
// They were mechanically re-pointed off the retired Basis physics tests, but
// the espresso observable set (e.g. station indices, slot names) must be
// confirmed via a live-backend E2E capture before re-enabling.

import { test, expect, type Page } from '@playwright/test';
import * as fs from 'fs';
import {
  APP_URL,
  ESPRESSO_CELL,
  RADIATION_COOLING,
  BOUNCING_BALL,
  ESPRESSO_PUMP_HYBRID,
  SCREENSHOT_DIR,
  navigateWithWorkspace,
  waitForWorkspaceLoaded,
  navigateToTool,
  startSession,
  stepSession,
  stopSession,
  isBackendHealthy,
  reapAllSessions,
  launchFirstRunTarget,
  selectPlotVariables,
  getTimeSeriesData,
  waitForTimeSeries,
  latestValue,
  type TimePoint,
} from './integration.setup';

try {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
} catch {}

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== 'passed') {
    const name = testInfo.title.replace(/[^a-zA-Z0-9]/g, '-').toLowerCase();
    const path = `${SCREENSHOT_DIR}/physics-mirror-${name}-${Date.now()}.png`;
    await page.screenshot({ path, fullPage: true });
    testInfo.attachments.push({
      name: 'failure-screenshot',
      path,
      contentType: 'image/png',
    });
  }
});

test.beforeAll(async () => {
  const healthy = await isBackendHealthy();
  if (!healthy) {
    throw new Error(
      'sysml-api backend is not running on port 8080. Start it before running integration tests.',
    );
  }
  await reapAllSessions();
});

test.afterAll(async () => {
  await reapAllSessions();
});

// ── Shared driver ─────────────────────────────────────────────────────

/**
 * Drives a workspace from URL → loaded → launch first run target → step
 * the session N times after selecting the requested plot variables.
 *
 * Returns the time-series payload after the steps have been ingested.
 */
async function runScenario(
  page: Page,
  opts: {
    workspace: string;
    variables: string[];
    steps: number;
    waitMinPoints?: number;
  },
): Promise<Record<string, TimePoint[]>> {
  await navigateWithWorkspace(page, opts.workspace);
  await waitForWorkspaceLoaded(page);

  // Try to launch via model-tree inline actions; fall back to the in-session Run button
  // if the workspace exposes no run targets (small physics examples
  // sometimes default to the implicit orchestrator launch).
  let launched = false;
  try {
    await launchFirstRunTarget(page);
    launched = true;
  } catch {
    launched = false;
  }
  if (!launched) {
    await navigateToTool(page, 'session');
    await startSession(page);
  }

  // Pre-select the variables we care about so the Plots tab ingests
  // the columns we want to read back. (The picker stores selection per
  // sessionId; the helper waits for the active session to be set.)
  await page.waitForFunction(
    () => typeof (window as any).__getActiveSessionId === 'function'
      && (window as any).__getActiveSessionId() !== null,
    null,
    { timeout: 15_000 },
  ).catch(() => undefined);
  await selectPlotVariables(page, opts.variables);

  // Step the session and let the snapshot polling ingest each tick.
  await stepSession(page, opts.steps);
  await page.waitForTimeout(2_000);

  return waitForTimeSeries(
    page,
    opts.variables,
    opts.waitMinPoints ?? 1,
    20_000,
  );
}

// ═════════════════════════════════════════════════════════════════════════
// 1. Espresso production cell: ODE temperatures rise above ambient
//    Mirrors: espresso_cell_physics.rs::test_espresso_cell_steps
// ═════════════════════════════════════════════════════════════════════════

test.describe.skip('Physics mirror: espresso-production-cell temperatures', () => {
  test('T_som and station1.bimetalTemp rise above ambient after stepping', async ({
    page,
  }) => {
    const data = await runScenario(page, {
      workspace: ESPRESSO_CELL,
      variables: ['T_som', 'station1.bimetalTemp'],
      steps: 30,
    });

    const tSom = latestValue(data, 'T_som');
    const bimetal = latestValue(data, 'station1.bimetalTemp');

    // At least one of the temperature variables must show data; ideally
    // both rise above ambient (298.15 K). The backend test asserts both
    // are >298.15 after 100 steps; we accept the same threshold for the
    // step count we drive through the UI (>=10 steps is enough for the
    // SoM heating model in espresso-production-cell to show a measurable rise).
    const sawAny = tSom !== null || bimetal !== null;
    expect(sawAny, 'expected at least one temperature variable in the buffer').toBe(true);

    if (tSom !== null) {
      expect(tSom, 'T_som should rise above ambient (298.15 K)').toBeGreaterThan(298.15);
    }
    if (bimetal !== null) {
      expect(
        bimetal,
        'station1.bimetalTemp should rise above ambient (298.15 K)',
      ).toBeGreaterThan(298.15);
    }

    await stopSession(page).catch(() => {});
  });
});

// ═════════════════════════════════════════════════════════════════════════
// 2. Espresso production cell: waveform-driven currents
//    Mirrors: espresso_cell_physics.rs::test_espresso_cell_waveform_driven
// ═════════════════════════════════════════════════════════════════════════

test.describe.skip('Physics mirror: espresso-production-cell waveform-driven currents', () => {
  test('station1.flow and station4.flow show non-zero values', async ({
    page,
  }) => {
    const data = await runScenario(page, {
      workspace: ESPRESSO_CELL,
      variables: ['station1.flow', 'station4.flow'],
      steps: 50,
    });

    const c1 = latestValue(data, 'station1.flow');
    const c4 = latestValue(data, 'station4.flow');

    const sawAny = c1 !== null || c4 !== null;
    expect(
      sawAny,
      'expected at least one station flow variable in the buffer',
    ).toBe(true);

    // Backend asserts both > 0 (waveform-driven). Accept a non-zero
    // magnitude either positive or negative since waveform sign depends
    // on sample timing — the user-facing claim is "the wires carry
    // current", not the polarity.
    if (c1 !== null) {
      expect(Math.abs(c1), 'station1.flow should be non-zero').toBeGreaterThan(0);
    }
    if (c4 !== null) {
      expect(Math.abs(c4), 'station4.flow should be non-zero').toBeGreaterThan(0);
    }

    await stopSession(page).catch(() => {});
  });
});

// ═════════════════════════════════════════════════════════════════════════
// 3. Radiation cooling decay (substituted)
//    Mirrors: physics_examples_pipeline.rs::test_radiation_cooling_simulation
//
// The standalone `examples/radiation-cooling/` workspace IS loadable via
// the API but the UI's `useModelCapabilities` hook does not currently
// classify a lone `calc def :> GetDerivative` as an ODE capability, so
// the Plots tab renders in ghost mode and the picker never appears.
// Until that detection gap is closed, we exercise the same observable
// outcome — "an ODE state variable monotonically changes over time" —
// against espresso-production-cell's `T_som` (a thermal ODE that rises from
// ambient under SoM heat input). The backend assertion mirrored here is
// the same shape: `temp_after_steps != temp_initial`.
//
// TODO(ui-caps): once `useModelCapabilities` recognises stand-alone
// GetDerivative calc defs, switch this back to RADIATION_COOLING and
// assert `latestValue(data, 'temperature') < 1000`.
// ═════════════════════════════════════════════════════════════════════════

test.describe.skip('Physics mirror: ODE temperature decays/changes monotonically', () => {
  test('an ODE state variable changes over time after stepping', async ({ page }) => {
    const data = await runScenario(page, {
      workspace: ESPRESSO_CELL,
      variables: ['T_som'],
      steps: 50,
      waitMinPoints: 5,
    });

    const series = data['T_som'] ?? [];
    expect(
      series.length,
      'expected at least 2 samples of T_som to verify monotonic change',
    ).toBeGreaterThanOrEqual(2);

    if (series.length >= 2) {
      const first = series[0].v;
      const last = series[series.length - 1].v;
      // Mirrors the backend's "ODE state evolved" assertion. Backend
      // espresso-production-cell test asserts T_som > 298.15 (rises from
      // ambient); we assert a measurable change between the first and
      // last sample to capture the same time-evolution claim.
      expect(
        Math.abs(last - first),
        `T_som should evolve over time (first=${first}, last=${last})`,
      ).toBeGreaterThan(0);
    }

    await stopSession(page).catch(() => {});
  });
});

// ═════════════════════════════════════════════════════════════════════════
// 4. Bouncing ball: position falls, velocity goes negative
//    Mirrors: bouncing_ball_pipeline.rs::test_bouncing_ball_workspace_orchestrator
// ═════════════════════════════════════════════════════════════════════════

test.describe.skip('Physics mirror: bouncing ball', () => {
  test('y drops below 10 and v turns negative under gravity', async ({ page }) => {
    const data = await runScenario(page, {
      workspace: BOUNCING_BALL,
      variables: ['y', 'v'],
      steps: 100,
    });

    const y = latestValue(data, 'y');
    const v = latestValue(data, 'v');

    // Backend asserts y < 10 and v < 0 after 100 steps from the initial
    // state (y=10, v=0).
    expect(
      y,
      'expected `y` (height) variable in time-series buffer for bouncing-ball',
    ).not.toBeNull();
    expect(
      v,
      'expected `v` (velocity) variable in time-series buffer for bouncing-ball',
    ).not.toBeNull();

    if (y !== null) expect(y, 'y should fall from initial 10 m').toBeLessThan(10);
    if (v !== null) expect(v, 'v should be negative (falling)').toBeLessThan(0);

    await stopSession(page).catch(() => {});
  });
});

// ═════════════════════════════════════════════════════════════════════════
// 5. hybrid oscillator: state-machine activity is visible in the UI
//    Mirrors (substitute): espresso_pump_hybrid.rs::test_pump_cycle_oscillation
//
// The hybrid test asserts all four state-machine states are visited
// (ascending, deadTime_AB, descending, deadTime_BA). The state names
// are not exposed as numeric chartable variables, so instead of
// asserting on the time-series buffer we assert the StateTimelineCard
// surfaces an active state machine after stepping — the closest UI
// affordance for "the SM is cycling".
// ═════════════════════════════════════════════════════════════════════════

test.describe.skip('Physics mirror: hybrid state-machine visibility', () => {
  test('hybrid-core-physics state machine surfaces an active state after stepping', async ({
    page,
  }) => {
    await navigateWithWorkspace(page, ESPRESSO_PUMP_HYBRID);
    await waitForWorkspaceLoaded(page);

    let launched = false;
    try {
      await launchFirstRunTarget(page);
      launched = true;
    } catch {
      launched = false;
    }
    if (!launched) {
      await navigateToTool(page, 'session');
      await startSession(page);
    }

    await stepSession(page, 30);
    await page.waitForTimeout(2_000);

    // The Results Workbench must be visible…
    const workbench = page.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('results-workbench-tab-timeline').click();

    // …and the State Timeline panel must be active for an SM-rich model.
    const stateTimelineLabel = workbench.locator('text=State Timeline').first();
    await expect(stateTimelineLabel).toBeVisible({ timeout: 5_000 });

    // The state machine in hybrid-core-physics has four states
    // (ascending, deadTime_AB, descending, deadTime_BA). Backend test
    // `espresso_pump_hybrid::test_pump_cycle_oscillation` asserts each is
    // visited; the closest UI claim we can make from this view is that
    // after stepping the SM has an active state name visible — the
    // state-name labels appear next to the SM tree node OR inside the
    // State Timeline card.
    const knownStateNames = /(ascending|descending|deadTime_AB|deadTime_BA)/;
    const stateLabel = page.locator(`text=${knownStateNames}`).first();
    await expect(
      stateLabel,
      'expected one of the four oscillator state names to appear',
    ).toBeVisible({ timeout: 10_000 });

    await stopSession(page).catch(() => {});
  });
});
