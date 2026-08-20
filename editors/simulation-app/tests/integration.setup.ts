/**
 * Shared helpers for integration tests against the real sysml-api backend.
 *
 * NO stubs, NO route interception. Every call hits the real API on port 8080
 * (proxied through Vite on port 3010).
 */
import { type Page, expect } from '@playwright/test';
import { repoPath } from './repo-paths';

// ── Constants ──────────────────────────────────────────────────────────

export const EXAMPLES_ROOT = repoPath('examples');
export const ESPRESSO_CELL = `${EXAMPLES_ROOT}/espresso-production-cell`;
export const RADIATION_COOLING = `${EXAMPLES_ROOT}/radiation-cooling`;
export const BOUNCING_BALL = `${EXAMPLES_ROOT}/bouncing-ball`;
export const ESPRESSO_PUMP_HYBRID = `${EXAMPLES_ROOT}/espresso-pump-hybrid`;
export const APP_URL = 'http://localhost:3010';
export const API_URL = 'http://localhost:8080';

export const SCREENSHOT_DIR = '/tmp/sim-app-integration/screenshots';

// ── Workspace loading ──────────────────────────────────────────────────

/**
 * Navigate to the app with the espresso-production-cell workspace loaded.
 * Waits for the workspace to finish loading (loading spinner disappears
 * and tree nodes or content becomes visible).
 */
export async function navigateWithWorkspace(
  page: Page,
  workspace: string = ESPRESSO_CELL,
): Promise<void> {
  await page.goto(
    `${APP_URL}/?workspace=${encodeURIComponent(workspace)}`,
    { waitUntil: 'domcontentloaded' },
  );
  await expect(page.getByTestId('app-shell')).toBeVisible({ timeout: 15_000 });
}

/**
 * Wait for the workspace to finish loading.
 * The loading spinner disappears and the Run model tree/status bar shows content.
 */
export async function waitForWorkspaceLoaded(page: Page): Promise<void> {
  // First ensure the app shell is visible
  await expect(page.getByTestId('app-shell')).toBeVisible({ timeout: 15_000 });

  await navigateToTool(page, 'session');

  // The espresso-production-cell can take 30-90s to parse on first load.
  const spinner = page.locator('text=Loading workspace...');
  await spinner.waitFor({ state: 'hidden', timeout: 120_000 }).catch(() => {
    // Spinner may have already finished before we got here
  });

  await expect(page.getByTestId('session-workspace')).toBeVisible({ timeout: 90_000 });
  await expect(page.getByTestId('status-bar')).toBeVisible({ timeout: 90_000 });

  // Element visibility is NOT model readiness. The shell, the tree
  // container and the status bar all mount while `load_workspace` and the
  // per-file `/stats` fetches are still in flight, and the status bar
  // renders its count strip only once `loadedFiles` is populated. Every
  // downstream assertion (counts, capabilities, a runnable tree row) was
  // racing that fill and failing intermittently on a warm backend. The
  // first count is the earliest honest "the model is here" signal.
  await expect(page.getByTestId('status-bar')).toContainText(/\d+\s+files?/, {
    timeout: 90_000,
  });
}

// ── Tool navigation ────────────────────────────────────────────────────

/**
 * Navigate to a specific tool tab by its key name.
 */
export async function navigateToTool(
  page: Page,
  tool: 'session' | 'compare' | 'verify' | 'analyze',
): Promise<void> {
  const tab = page.getByTestId(`tool-tab-${tool}`);
  if (await tab.isVisible().catch(() => false)) {
    await tab.click();
  } else {
    await expect(tab).toBeVisible({ timeout: 5_000 });
    await tab.click();
  }
  await page.waitForTimeout(300);
}

// ── Session operations ─────────────────────────────────────────────────

/**
 * Reveal a `▶ Run` affordance in the model tree and return its testid.
 *
 * The Run control is gated on a run target (`hasRunTarget` in
 * SessionHeader) and the ONLY way to pick one is the inline launcher on a
 * runnable tree row. For a simulation target that means a `StateDefinition`
 * / `StateUsage` row — and in a workspace the size of the production cell
 * those sit ~600 rows deep in a tree that (a) renders collapsed and (b)
 * row-virtualises above `ROW_VIRTUALIZATION_THRESHOLD`, so neither the
 * first paint nor `expand all` alone puts one in the DOM. Verified against
 * the live backend: the default render exposes exactly five launchers
 * (1 Analyze + 4 Verify) and zero Run.
 *
 * So: expand everything, then walk the virtualised scroller until a Run
 * launcher mounts. Prefers a `StateDefinition` row (the state machine
 * itself) over a `StateUsage` row (one of its states) when the window
 * holds both.
 */
export async function revealRunLaunchButton(
  page: Page,
): Promise<string | null> {
  // The tree is fed by a DIFFERENT query than the status-bar counts and
  // lands LATER: measured against the live backend, at the instant the
  // first file count appears the tree header still reads "loading…" with
  // zero rows, and the rows arrive ~0.5s after that. Gating on the count
  // strip alone (waitForWorkspaceLoaded) therefore still let this sweep
  // run against an empty tree and report "no Run affordance".
  await expect(
    page.locator('[data-testid="session-tree-v2"] [data-raw-kind]').first(),
  ).toBeVisible({ timeout: 60_000 });

  const expandAll = page.getByTestId('session-tree-v2-expand-all').first();
  if (await expandAll.isVisible().catch(() => false)) {
    await expandAll.click();
    await page.waitForTimeout(1_000);
  }

  return page.evaluate(async () => {
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    // `session-tree-v2` now resolves to exactly one element — the tree's
    // scroll container. It used to be shared with the rail panel that wraps
    // it, so this selector needed a `[role="tree"]` qualifier to pick the
    // scroller out of the pair (finding 16, since fixed by renaming the
    // wrapper to `session-tree-v2-panel`).
    const scroller = document.querySelector<HTMLElement>(
      '[data-testid="session-tree-v2"]',
    );
    if (!scroller) return null;

    const findRun = (): HTMLElement | null => {
      const buttons = [
        ...scroller.querySelectorAll<HTMLElement>('[data-testid$="-launch"]'),
      ].filter((b) => (b.textContent ?? '').trim().endsWith('Run'));
      if (buttons.length === 0) return null;
      const onStateDef = buttons.find(
        (b) => b.closest('[data-raw-kind]')?.getAttribute('data-raw-kind') === 'StateDefinition',
      );
      return onStateDef ?? buttons[0]!;
    };

    scroller.scrollTop = 0;
    await sleep(250);
    let hit = findRun();
    const step = Math.max(200, Math.floor(scroller.clientHeight * 0.7));
    let guard = 0;
    while (
      !hit &&
      guard++ < 500 &&
      scroller.scrollTop + scroller.clientHeight < scroller.scrollHeight - 2
    ) {
      scroller.scrollTop += step;
      await sleep(80);
      hit = findRun();
    }
    return hit?.getAttribute('data-testid') ?? null;
  });
}

/**
 * Make sure a run target is selected so the Run control is enabled.
 * No-op when the control is already enabled (a session is active, or an
 * earlier test in a serial group already picked a target).
 */
export async function selectRunTarget(page: Page): Promise<void> {
  const runBtn = page.getByTestId('control-run');
  await expect(runBtn).toBeVisible({ timeout: 15_000 });
  if (await runBtn.isEnabled()) return;

  const testId = await revealRunLaunchButton(page);
  if (!testId) {
    throw new Error(
      'No ▶ Run affordance found in the model tree — the Run control cannot ' +
        'be enabled without a run target.',
    );
  }
  await page.getByTestId(testId).click();
  await expect(runBtn).toBeEnabled({ timeout: 10_000 });
}

/**
 * Start an orchestrator session via the Session tool.
 *
 * Two steps, not one: pick a run target in the tree, THEN click Run. The
 * Run control ships disabled until a target is selected ("No run target —
 * in the tree, click ▶ Run on a state machine, analysis case, or
 * verification case to select one"), so a bare `control-run` click just
 * waits out the test timeout.
 */
export async function startSession(page: Page): Promise<void> {
  await navigateToTool(page, 'session');
  await page.waitForTimeout(500);

  await selectRunTarget(page);

  const runBtn = page.getByTestId('control-run');
  await expect(runBtn).toBeEnabled({ timeout: 10_000 });
  await runBtn.click();
  await page.waitForTimeout(2_000); // Allow backend to process
}

/**
 * Step the session N times using the Step button.
 */
export async function stepSession(
  page: Page,
  times: number = 1,
): Promise<void> {
  for (let i = 0; i < times; i++) {
    const stepBtn = page.getByTestId('control-step');
    await expect(stepBtn).toBeVisible({ timeout: 5_000 });
    // The button may be disabled if session phase is completed/error
    const isDisabled = await stepBtn.isDisabled();
    if (isDisabled) break;
    await stepBtn.click();
    await page.waitForTimeout(800); // Allow backend to process step
  }
}

/**
 * Stop the running session.
 */
export async function stopSession(page: Page): Promise<void> {
  const stopBtn = page.getByTestId('control-stop');
  if (await stopBtn.isVisible().catch(() => false)) {
    const isDisabled = await stopBtn.isDisabled();
    if (!isDisabled) {
      await stopBtn.click();
      await page.waitForTimeout(1_000);
    }
  }
}

// ── Direct API helpers ─────────────────────────────────────────────────

/**
 * Check if the backend API is healthy.
 */
export async function isBackendHealthy(): Promise<boolean> {
  try {
    const resp = await fetch(`${API_URL}/health`);
    return resp.ok;
  } catch {
    return false;
  }
}

/**
 * Pre-load the workspace via direct API call (for beforeAll setup).
 * The espresso-production-cell parse takes 30-60s, so we use a generous timeout.
 * If the workspace is already loaded, the backend returns quickly.
 */
export async function preloadWorkspaceViaAPI(): Promise<void> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 90_000);
  try {
    const resp = await fetch(`${API_URL}/api/command`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        command: 'sysml.load_workspace',
        params: { root: ESPRESSO_CELL },
      }),
      signal: controller.signal,
    });
    if (!resp.ok) {
      throw new Error(
        `Failed to preload workspace: ${resp.status} ${resp.statusText}`,
      );
    }
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Navigate straight to the Compare surface.
 *
 * Compare is reached by ROUTE, not by a nav tab: ninebar Phase 6 W1 moved
 * it under the Simulate door at `/run/compare` and set `visibleInNav:
 * false`, so `tool-tab-compare` is never rendered and
 * `navigateToTool(page, 'compare')` can only time out. Matches the smoke
 * suite's `gotoCompare` (commit 8bae1294).
 */
export async function gotoCompare(page: Page): Promise<void> {
  await page.goto(`${APP_URL}/run/compare`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('compare-workflow')).toBeVisible({ timeout: 15_000 });
}

/** Every session the backend currently holds. */
export async function listSessions(): Promise<Array<{ id?: string }>> {
  try {
    const resp = await fetch(`${API_URL}/api/command`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ command: 'sysml.sessions.list', params: {} }),
    });
    if (!resp.ok) return [];
    const body = await resp.json();
    return Array.isArray(body) ? body : [];
  } catch {
    return [];
  }
}

/** Fraction of a quota that must be consumed before reaping is worth its cost. */
const REAP_PRESSURE = 0.5;

/**
 * Stop every active session on the backend — but only when the quota is
 * actually under pressure.
 *
 * Each test that starts a session leaves it parked until the 60-min TTL reaps
 * it; across consecutive runs the orchestrator quota (cap 20) fills up and new
 * sessions fail with QuotaExceeded. That is the problem this solves.
 *
 * The blunt version solved it by stopping EVERY session on a shared backend on
 * every run — including one a human was driving in the browser, which happened
 * repeatedly during the UX sweep and silently destroyed hand-built state
 * (punch-list finding 18). The backend is shared; the suite does not own it.
 *
 * So: ask `sessions.quota` first and only reap when some class is at or above
 * REAP_PRESSURE of its cap. A developer with one or two sessions open is far
 * below that and is now left alone, while a CI run with a filling quota still
 * gets its clean slate. `SYSML_TEST_REAP=always` forces the old behaviour and
 * `SYSML_TEST_REAP=never` opts out entirely.
 *
 * Reaping is announced on stderr rather than done silently — if it does take
 * someone's session, they should be able to find out why.
 *
 * Idempotent: silently ignores an empty list, missing IDs, or already-stopped
 * sessions.
 */
async function quotaUnderPressure(): Promise<boolean> {
  try {
    const resp = await fetch(`${API_URL}/api/command`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ command: 'sysml.sessions.quota', params: {} }),
    });
    if (!resp.ok) return true; // can't tell → fall back to the safe-for-CI path
    const body = (await resp.json()) as Record<string, { cap?: number; used?: number }>;
    const classes = Object.values(body ?? {});
    if (classes.length === 0) return true;
    return classes.some(
      (c) => typeof c?.cap === 'number' && typeof c?.used === 'number'
        && c.cap > 0 && c.used / c.cap >= REAP_PRESSURE,
    );
  } catch {
    return true;
  }
}

export async function reapAllSessions(): Promise<void> {
  const mode = process.env.SYSML_TEST_REAP;
  if (mode === 'never') return;
  if (mode !== 'always' && !(await quotaUnderPressure())) return;

  let sessions: Array<{ id?: string }> = [];
  try {
    const listResp = await fetch(`${API_URL}/api/command`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ command: 'sysml.sessions.list', params: {} }),
    });
    if (!listResp.ok) return;
    const body = await listResp.json();
    if (Array.isArray(body)) sessions = body;
  } catch {
    return;
  }

  const ids = sessions.map((s) => s.id).filter((id): id is string => typeof id === 'string');
  if (ids.length > 0) {
    // eslint-disable-next-line no-console
    console.warn(
      `[integration] reaping ${ids.length} backend session(s) — quota is under pressure. ` +
        `Set SYSML_TEST_REAP=never if you are driving a session by hand.`,
    );
  }

  await Promise.all(
    ids
      .map((id) =>
        fetch(`${API_URL}/api/command`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            command: 'sysml.sessions.stop',
            params: { session_id: id },
          }),
        }).catch(() => undefined),
      ),
  );
}

// ── Plot / time-series helpers ─────────────────────────────────────────

/**
 * TimePoint shape mirrored from the sim-app (kept loose to avoid
 * test-time imports of app source).
 */
export interface TimePoint {
  t: number;
  v: number;
}

/**
 * Open the plot variable picker, check the boxes for `names`, and close
 * the picker. Uses the test-only `__setPlotSelection` window helper for
 * reliability — the picker UI groups variables by domain so locating each
 * checkbox is brittle.
 *
 * Falls back to the picker UI if the helper is missing for any reason.
 */
export async function selectPlotVariables(
  page: Page,
  names: string[],
): Promise<void> {
  const ok = await page.evaluate((vars) => {
    const w = window as any;
    if (typeof w.__setPlotSelection === 'function') {
      return w.__setPlotSelection(vars) === true;
    }
    return false;
  }, names);

  if (ok) return;

  // UI fallback: open the picker, tick each checkbox by label.
  const openBtn = page.getByTestId('plot-pick-0').first();
  await expect(openBtn).toBeVisible({ timeout: 5_000 });
  await openBtn.click();
  const dialog = page.getByTestId('plot-variable-picker');
  await expect(dialog).toBeVisible({ timeout: 5_000 });

  for (const name of names) {
    const row = dialog.locator(`label:has-text("${name}")`).first();
    if (await row.isVisible().catch(() => false)) {
      const cb = row.locator('input[type="checkbox"]');
      const checked = await cb.isChecked().catch(() => false);
      if (!checked) await cb.click();
    }
  }

  await dialog.locator('button:has-text("Done")').click();
}

/**
 * Read the time-series data the Plots tab would render. Uses
 * `window.__getTimeSeries`, which is mounted by App.tsx and returns the
 * same `Record<string, TimePoint[]>` the chart consumes.
 */
export async function getTimeSeriesData(
  page: Page,
): Promise<Record<string, TimePoint[]>> {
  return page.evaluate(() => {
    const w = window as any;
    if (typeof w.__getTimeSeries !== 'function') return {};
    return w.__getTimeSeries();
  });
}

/**
 * Pick a tick-speed multiplier (label matches the SessionHeader buttons:
 * '0.5x' | '1x' | '2x' | '5x' | '10x').
 */
export async function setTickSpeed(
  page: Page,
  label: '0.5x' | '1x' | '2x' | '5x' | '10x',
): Promise<void> {
  const btn = page.getByTestId(`tick-speed-${label}`);
  if (await btn.isVisible().catch(() => false)) {
    await btn.click();
    await page.waitForTimeout(100);
  }
}

/**
 * Launch the first available runnable model-tree row.
 * Uses the inline Run/Analyze/Verify affordance exposed by the tree.
 */
export async function launchFirstRunTarget(page: Page): Promise<void> {
  await navigateToTool(page, 'session');
  await page.waitForTimeout(2_000);

  const launchBtn = page.locator('[data-testid$="-launch"]', { hasText: /Run|Analyze|Verify/ }).first();
  await expect(launchBtn).toBeVisible({ timeout: 15_000 });
  await launchBtn.click();
  await page.waitForTimeout(500);

  // Confirm we landed on the session workspace.
  await expect(page.getByTestId('session-workspace')).toBeVisible({
    timeout: 10_000,
  });
}

/**
 * Returns the latest value for a variable in the buffered time series,
 * or `null` if the variable hasn't been ingested yet.
 */
export function latestValue(
  data: Record<string, TimePoint[]>,
  name: string,
): number | null {
  const pts = data[name];
  if (!pts || pts.length === 0) return null;
  return pts[pts.length - 1].v;
}

/**
 * Wait until the given variable names show up in the time-series buffer
 * with at least `minPoints` entries each. Useful after stepping to make
 * sure the snapshot polling loop has had time to ingest data.
 */
export async function waitForTimeSeries(
  page: Page,
  names: string[],
  minPoints = 1,
  timeoutMs = 15_000,
): Promise<Record<string, TimePoint[]>> {
  const deadline = Date.now() + timeoutMs;
  let last: Record<string, TimePoint[]> = {};
  while (Date.now() < deadline) {
    last = await getTimeSeriesData(page);
    const allReady = names.every(
      (n) => Array.isArray(last[n]) && last[n].length >= minPoints,
    );
    if (allReady) return last;
    await page.waitForTimeout(500);
  }
  return last;
}
