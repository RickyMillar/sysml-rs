/**
 * Smoke tests for simulation-app.
 *
 * These validate that the UI shell renders correctly without a running
 * sysml-api backend. All backend fetches are intercepted and stubbed
 * so tests run fully offline.
 *
 * Target: core workflow surfaces, session controls, results workbench,
 * status bar, and run dropdown.
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

// ── Route interception: stub every backend API call ────────────────────

async function stubBackend(page: Page) {
  // Intercept only localhost API routes — do NOT match external resources
  // (Google Fonts, etc.) by scoping to localhost:3010.
  await page.route('**/health', (route) => {
    if (route.request().url().includes('localhost')) {
      return route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' });
    }
    return route.continue();
  });
  await page.route('http://localhost:3010/sessions**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/workspace**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"uris":[]}' }),
  );
  await page.route('http://localhost:3010/sources**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/models**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }),
  );
  await page.route('http://localhost:3010/files**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/api/**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
}

/** Navigate and wait for the app shell to be visible. */
async function gotoApp(page: Page) {
  await stubBackend(page);
  await page.goto(APP, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeVisible({ timeout: 15_000 });
}

/**
 * Navigate straight to the Compare surface.
 *
 * Compare is reached by route, not by a nav tab: ninebar Phase 6 W1 moved
 * it under the Simulate door at `/run/compare` and took its tab out of the
 * switcher. In the product it is opened from the frame session switcher,
 * Cmd-K, and the promote-to-Compare actions; none of those are a tab click,
 * so the tests navigate.
 */
async function gotoCompare(page: Page) {
  await stubBackend(page);
  await page.goto(`${APP}/run/compare`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeVisible({ timeout: 15_000 });
}

// ── 1. App loads ────────────────────────────────────────────────────────

test('app shell renders with primary tool tabs', async ({ page }) => {
  await gotoApp(page);

  const nav = page.getByTestId('tool-nav');
  await expect(nav).toBeVisible();

  await expect(page.getByTestId('workspace-bar')).toBeVisible();
  await expect(page.getByTestId('tool-tab-session')).toContainText('Run');
  // Compare is NOT a top-level tab: ninebar Phase 6 W1 demoted it to a
  // Simulate mode (`visibleInNav: false` in routes.ts), so the switcher
  // filters it out. Asserted absent rather than dropped so this suite
  // states the ruling instead of going silent on it — the ninebar suite
  // pins the same thing (ninebar-compare.spec.ts).
  await expect(page.getByTestId('tool-tab-compare')).toHaveCount(0);
  await expect(page.getByTestId('tool-tab-packages')).toHaveCount(0);
  await expect(page.getByTestId('tool-tab-runTargets')).toHaveCount(0);
});

// ── 2. Workspace bar ──────────────────────────────────────────────────

test('workspace bar is visible and owns workspace loading', async ({ page }) => {
  await gotoApp(page);

  await expect(page.getByTestId('workspace-bar')).toBeVisible();
  await expect(page.getByTestId('workspace-bar-input')).toBeVisible();
  await expect(page.getByTestId('workspace-bar-load')).toBeVisible();
});

// ── 3. Removed setup routes redirect to Run ───────────────────────────

test('removed packages and run-targets routes redirect to Run', async ({ page }) => {
  await gotoApp(page);

  await page.goto(`${APP}/packages`);
  await expect(page.getByTestId('session-workspace')).toBeVisible({ timeout: 5_000 });

  await page.goto(`${APP}/run-targets`);
  await expect(page.getByTestId('session-workspace')).toBeVisible({ timeout: 5_000 });
});

// ── 4. Run workflow default ────────────────────────────────────────────

test('session workspace renders with 4 zones', async ({ page }) => {
  await gotoApp(page);

  // Session is the default tool — workspace should be visible immediately
  const workspace = page.getByTestId('session-workspace');
  await expect(workspace).toBeVisible();

  // Zone 1: Session header
  await expect(page.getByTestId('session-header')).toBeVisible();

  // Zone 3: Results workbench
  await expect(page.getByTestId('results-workbench')).toBeVisible();

  // Zone 4: Status bar
  await expect(page.getByTestId('status-bar')).toBeVisible();
});

// ── 5. Session header controls ─────────────────────────────────────────

test('session header has phase-aware controls', async ({ page }) => {
  await gotoApp(page);

  const header = page.getByTestId('session-header');
  await expect(header).toBeVisible();

  // In idle phase with no run target selected (the default on a stubbed,
  // empty workspace): Run is correctly DISABLED with an explanatory
  // title rather than silently no-op'ing — see SessionHeader's
  // `hasRunTarget` gate.
  const runBtn = page.getByTestId('control-run');
  await expect(runBtn).toBeVisible();
  await expect(runBtn).toBeDisabled();
  await expect(runBtn).toHaveAttribute(
    'title',
    'No run target — in the tree, click ▶ Run on a state machine, analysis case, or verification case to select one',
  );

  // Step button should be visible (always shown) and enabled — it can
  // lazily create a session even without a pre-selected target.
  const stepBtn = page.getByTestId('control-step');
  await expect(stepBtn).toBeVisible();
  await expect(stepBtn).not.toBeDisabled();

  // Stop button should be visible but disabled (idle phase)
  const stopBtn = page.getByTestId('control-stop');
  await expect(stopBtn).toBeVisible();
  await expect(stopBtn).toBeDisabled();

  // Phase badge should show "idle"
  await expect(header).toContainText('idle');
});

// ── 6. Results workbench visible ───────────────────────────────────────

test('results workbench is visible with task tabs', async ({ page }) => {
  await gotoApp(page);

  const workbench = page.getByTestId('results-workbench');
  await expect(workbench).toBeVisible();

  await expect(page.getByTestId('results-workbench-tab-plots')).toBeVisible();
  await expect(page.getByTestId('results-workbench-tab-kpis')).toBeVisible();
  await expect(page.getByTestId('results-workbench-tab-equations')).toBeVisible();
});

// ── 7. Inactive tab empty states render ────────────────────────────────

test('inactive workbench tabs render contextual empty states', async ({ page }) => {
  await gotoApp(page);

  await page.getByTestId('results-workbench-tab-plots').click();
  await expect(page.getByTestId('results-workbench-empty-plots')).toBeVisible();
  await expect(page.getByTestId('results-workbench-empty-plots')).toContainText(
    'No plottable signals yet',
  );
});

// ── 8. Compare tool ────────────────────────────────────────────────────

test('compare workspace renders with session selectors', async ({ page }) => {
  await gotoCompare(page);

  // CompareWorkflow (R4.2) replaced the old baseline/compare-select
  // header with an N-session picker sidebar + a shared playhead.
  const workflow = page.getByTestId('compare-workflow');
  await expect(workflow).toBeVisible({ timeout: 5_000 });

  // Left — session picker (checkbox list, 0/6 picked on a stubbed empty
  // archive).
  const picker = page.getByTestId('compare-session-picker');
  await expect(picker).toBeVisible();
  await expect(picker).toContainText('Sessions');
  await expect(page.getByTestId('compare-session-picker-count')).toContainText('0/6');

  // Empty state when fewer than 2 sessions are picked.
  const needMore = page.getByTestId('compare-need-more');
  await expect(needMore).toBeVisible();
  await expect(needMore).toContainText('Pick 2 more sessions');
});

// ── 9. Status bar ──────────────────────────────────────────────────────

test('status bar shows API health indicator', async ({ page }) => {
  await gotoApp(page);

  const statusBar = page.getByTestId('status-bar');
  await expect(statusBar).toBeVisible();

  // Should show "API" health text
  await expect(statusBar).toContainText('API');
});

// ── 10. Scenario dropdown ──────────────────────────────────────────────

test('run dropdown opens and shows menu items', async ({ page }) => {
  await gotoApp(page);

  // Wait for session header to be ready
  await expect(page.getByTestId('session-header')).toBeVisible();

  // On a stubbed empty workspace there is no runnable target, so the
  // whole Run split-button (primary + chevron toggle) is disabled —
  // the disabled-with-reason pattern (see SessionHeader's
  // `hasRunTarget` gate). The menu therefore cannot open here; assert
  // the toggle exists, is disabled, and the menu stays closed.
  //
  // Menu-content note (for when a target IS selected): the only
  // Run-internal item left in this dropdown is "Step-by-step" —
  // Parameter Sweep / Monte Carlo / What-If Comparison moved to their
  // own workflows under /analyze and /compare (see RunDropdown's doc
  // comment in SessionHeader.tsx).
  const toggle = page.getByTestId('run-dropdown-toggle');
  await expect(toggle).toBeVisible();
  await expect(toggle).toBeDisabled();

  await expect(page.getByTestId('run-dropdown-menu')).toHaveCount(0);
});

// ── 11. Tool tab switching ─────────────────────────────────────────────

test('clicking workflow tabs switches the surface', async ({ page }) => {
  await gotoApp(page);

  // Start on Run (default)
  await expect(page.getByTestId('session-workspace')).toBeVisible();

  // Verify, not Compare — Compare stopped being a tab in ninebar Phase 6
  // (see gotoCompare). This test is about the tab-click gesture, so it
  // uses a tab that still exists rather than becoming a navigation test.
  await page.getByTestId('tool-tab-verify').click();
  await expect(page.getByTestId('verify-workflow')).toBeVisible({ timeout: 3_000 });
  await expect(page.getByTestId('session-workspace')).toHaveCount(0);

  await page.getByTestId('tool-tab-session').click();
  await expect(page.getByTestId('session-workspace')).toBeVisible({ timeout: 3_000 });
});

// ── 12. No target selected message ─────────────────────────────────────

test('session header shows "No target selected" in idle state', async ({ page }) => {
  await gotoApp(page);

  const header = page.getByTestId('session-header');
  await expect(header).toBeVisible();

  await expect(header).toContainText('No target selected');
});
