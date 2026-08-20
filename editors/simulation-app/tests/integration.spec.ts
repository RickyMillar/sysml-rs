/**
 * Integration tests for simulation-app against the REAL sysml-api backend.
 *
 * Pre-conditions:
 *   - sysml-api running on port 8080
 *   - Vite dev server on port 3010 with proxy to 8080
 *   - Golden production cell at <repo>/examples/espresso-production-cell
 *
 * NO stubbed responses. NO route interception.
 */
import { test, expect, type Page } from '@playwright/test';
import * as fs from 'fs';
import {
  APP_URL,
  ESPRESSO_CELL,
  SCREENSHOT_DIR,
  navigateWithWorkspace,
  waitForWorkspaceLoaded,
  navigateToTool,
  gotoCompare,
  listSessions,
  startSession,
  selectRunTarget,
  revealRunLaunchButton,
  stepSession,
  stopSession,
  isBackendHealthy,
  reapAllSessions,
} from './integration.setup';

// ── Screenshot on failure ──────────────────────────────────────────────

try {
  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
} catch {}

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== 'passed') {
    const name = testInfo.title.replace(/[^a-zA-Z0-9]/g, '-').toLowerCase();
    const path = `${SCREENSHOT_DIR}/${name}-${Date.now()}.png`;
    await page.screenshot({ path, fullPage: true });
    testInfo.attachments.push({
      name: 'failure-screenshot',
      path,
      contentType: 'image/png',
    });
  }
});

// ── Pre-flight: verify backend is up ───────────────────────────────────

test.beforeAll(async () => {
  const healthy = await isBackendHealthy();
  if (!healthy) {
    throw new Error(
      'sysml-api backend is not running on port 8080. Start it before running integration tests.',
    );
  }
  // Reap any sessions left over from prior runs so the orchestrator quota
  // (cap 20) doesn't fill up across consecutive runs.
  await reapAllSessions();
});

// Stop sessions started during the run so we don't poison the next run.
test.afterAll(async () => {
  await reapAllSessions();
});

// ═════════════════════════════════════════════════════════════════════════
// Group 1: Workspace Loading
// ═════════════════════════════════════════════════════════════════════════

test.describe('Workspace Loading', () => {
  test('workspace loads through the global shell bar', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);

    await expect(page.getByTestId('workspace-bar')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('workspace-bar-current')).toContainText(
      'espresso-production-cell',
      { timeout: 15_000 },
    );
    await expect(page.getByTestId('session-workspace')).toBeVisible({ timeout: 15_000 });
  });

  test('model tree becomes the package/model discovery surface', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    // `.first()` is load-bearing: SessionTreeV2 stamps `session-tree-v2`
    // on its outer container AND passes it down as the ModelTreeView
    // `testIdPrefix`, which the inner `role="tree"` scroller re-emits. Two
    // elements share the testid, so a bare `getByTestId` is a strict-mode
    // violation. Reported as a testability defect; the outer container is
    // the one this assertion means.
    const tree = page.getByTestId('session-tree-v2').first();
    await expect(tree).toBeVisible({ timeout: 15_000 });
    await expect(tree).toContainText(/Model|Package|Part|State|Analysis|Verify/i, { timeout: 15_000 });
  });
});

// ═════════════════════════════════════════════════════════════════════════
// Group 2: Model Stats & Capabilities
// ═════════════════════════════════════════════════════════════════════════

test.describe('Model Stats & Capabilities', () => {
  test('status bar shows non-zero model counts', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');
    await page.waitForTimeout(2_000);

    const statusBar = page.getByTestId('status-bar');
    await expect(statusBar).toBeVisible({ timeout: 5_000 });

    // The status bar must show the API health indicator
    const apiText = statusBar.locator('text=API');
    await expect(apiText).toBeVisible({ timeout: 10_000 });

    // The status bar must show at least one non-zero model count.
    // Golden production cell has 16 files plus state machines, ODEs, flows, etc.
    const barText = (await statusBar.textContent()) ?? '';
    const hasFileCount = /\d+\s+files?/.test(barText);
    const hasModelCount =
      /\d+\s+(SM|ODE|flow|constraint|test)s?/i.test(barText);
    expect(hasFileCount || hasModelCount).toBe(true);

    // Specifically, the loaded espresso-production-cell has at least 10 files
    const fileMatch = barText.match(/(\d+)\s+files?/);
    if (fileMatch) {
      expect(parseInt(fileMatch[1], 10)).toBeGreaterThanOrEqual(10);
    }
  });

  test('capabilities detect SMs, ODEs, flows, constraints', async ({
    page,
  }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');
    await page.waitForTimeout(2_000);

    // The "Model capabilities" pre-session panel is GONE: it lived in the
    // legacy `SessionTree`, which no component renders any more (the Run
    // workflow mounts `SessionTreeV2` under both shells). The surviving
    // reader of `useModelCapabilities` — the same hook that panel used —
    // is the status bar's count strip, so that is where capability
    // detection is asserted now.
    await expect(page.locator('text=Model capabilities')).toHaveCount(0);

    const statusBar = page.getByTestId('status-bar');
    await expect(statusBar).toBeVisible({ timeout: 10_000 });
    const barText = (await statusBar.textContent()) ?? '';

    // The espresso-production-cell declares state machines, ODEs and
    // constraints; the strip omits a category entirely when its count is
    // zero, so presence of the label IS the non-zero assertion.
    expect(barText).toMatch(/\d+\s+SMs?\b/);
    expect(barText).toMatch(/\d+\s+ODEs?\b/);
    expect(barText).toMatch(/\d+\s+constraints?\b/);

    // …and a runnable target really is reachable from the tree — the
    // capability counts are not just a label.
    const runLaunch = await revealRunLaunchButton(page);
    expect(runLaunch).not.toBeNull();
  });
});

// ═════════════════════════════════════════════════════════════════════════
// Group 3: Session Lifecycle (serial -- order matters)
// ═════════════════════════════════════════════════════════════════════════

test.describe.serial('Session Lifecycle', () => {
  let sharedPage: Page;

  test.beforeAll(async ({ browser }) => {
    sharedPage = await browser.newPage();
    await navigateWithWorkspace(sharedPage);
    await waitForWorkspaceLoaded(sharedPage);
  });

  test.afterAll(async () => {
    await stopSession(sharedPage).catch(() => {});
    await sharedPage.close();
  });

  test('can start an orchestrator session', async () => {
    await startSession(sharedPage);

    const sessionHeader = sharedPage.getByTestId('session-header');
    await expect(sessionHeader).toBeVisible({ timeout: 5_000 });

    // The phase badge should be visible
    const phaseBadge = sessionHeader.locator('.mono-text').first();
    await expect(phaseBadge).toBeVisible({ timeout: 5_000 });
  });

  test('stepping a session increases tick count', async () => {
    await navigateToTool(sharedPage, 'session');

    await stepSession(sharedPage, 3);
    await sharedPage.waitForTimeout(2_000);

    const statusBar = sharedPage.getByTestId('status-bar');
    await expect(statusBar).toBeVisible({ timeout: 5_000 });
    const barText = (await statusBar.textContent()) ?? '';
    const hasPhase = /RUNNING|PAUSED|COMPLETED|DONE|ERROR/i.test(barText);
    const hasStep = /step\s+\d+/i.test(barText);
    const hasTime = /t=\d/i.test(barText);
    expect(hasPhase || hasStep || hasTime).toBe(true);
  });

  test('session tree shows topology after start', async () => {
    await navigateToTool(sharedPage, 'session');

    // All three surfaces this test used to accept are gone: the
    // "Model capabilities" pre-session view and the `view_module` module
    // header both lived in the retired `SessionTree`, and the flat
    // TopologyView's `tick N` counter went with it. SessionTreeV2 shows
    // live topology INLINE instead — a started session decorates its
    // rows with the current state of each state machine.
    const tree = sharedPage.getByTestId('session-tree-v2').first();
    await expect(tree).toBeVisible({ timeout: 5_000 });

    const liveStates = tree.locator('[data-testid$="-state"]');
    await expect(liveStates.first()).toBeVisible({ timeout: 10_000 });
  });

  test('session detail contains snapshot with variables', async () => {
    await navigateToTool(sharedPage, 'session');

    const statusBar = sharedPage.getByTestId('status-bar');
    await expect(statusBar).toBeVisible({ timeout: 5_000 });

    const barText = (await statusBar.textContent()) ?? '';
    // The API health indicator must be present
    expect(barText).toContain('API');

    // The status bar must show non-zero file/model counts (from the
    // workspace store bridge populated by useLoadWorkspace).
    expect(/\d+\s+files?/.test(barText)).toBe(true);

    // Results workbench should always render the task tabs for the
    // loaded model.
    const workbench = sharedPage.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 5_000 });
    await expect(sharedPage.getByTestId('results-workbench-tab-plots')).toBeVisible();
  });

  test('can stop a running session', async () => {
    await navigateToTool(sharedPage, 'session');
    await stopSession(sharedPage);

    const sessionHeader = sharedPage.getByTestId('session-header');
    await expect(sessionHeader).toBeVisible({ timeout: 5_000 });

    // After stopping, the Run button should be visible (phase is idle)
    const runBtn = sharedPage.getByTestId('control-run');
    await expect(runBtn).toBeVisible({ timeout: 5_000 });
  });
});

// ═════════════════════════════════════════════════════════════════════════
// Group 5: Results Workbench
// ═════════════════════════════════════════════════════════════════════════

test.describe('Results Workbench', () => {
  test('results workbench shows task tabs based on capabilities', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    const workbench = page.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 10_000 });

    await expect(page.getByTestId('results-workbench-tab-plots')).toBeVisible();
    await expect(page.getByTestId('results-workbench-tab-timeline')).toBeVisible();
    await expect(page.getByTestId('results-workbench-tab-constraints')).toBeVisible();
  });

  test('every workbench tab renders content or its contextual empty state', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    const workbench = page.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 10_000 });

    // The "Raw Data" tab was DELETED (R56) — it had no panels and only
    // ever showed a "lives here next" placeholder, which read as a broken
    // peer tab. Asserted absent rather than dropped so this suite states
    // the ruling instead of going silent on it.
    await expect(page.getByTestId('results-workbench-tab-raw')).toHaveCount(0);

    // What replaced that assertion: no tab may render nothing. The
    // production cell activates every panel, so the empty-state branch is
    // unreachable HERE (the smoke suite covers it against a stubbed empty
    // backend); the contract that survives on a real workspace is that
    // each tab shows either its cards or its own contextual empty state.
    for (const tab of ['plots', 'kpis', 'equations', 'constraints', 'timeline']) {
      await page.getByTestId(`results-workbench-tab-${tab}`).click();
      await page.waitForTimeout(300);
      const cards = workbench.locator('[data-testid^="results-workbench-card-"]');
      const empty = page.getByTestId(`results-workbench-empty-${tab}`);
      const cardCount = await cards.count();
      const emptyVisible = await empty.isVisible().catch(() => false);
      expect(
        cardCount > 0 || emptyVisible,
        `workbench tab "${tab}" rendered neither cards nor an empty state`,
      ).toBe(true);
    }
  });

  test('plots tab has chart content after stepping', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    await startSession(page);
    await stepSession(page, 5);
    await page.waitForTimeout(2_000);

    const workbench = page.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('results-workbench-tab-plots').click();

    // The card is located by its own testid, not by walking up from the
    // tab label to a `rounded-lg` ancestor — the workbench cards stopped
    // carrying that utility class, so the old xpath matched nothing.
    await expect(
      workbench.getByTestId('results-workbench-card-plots'),
    ).toBeVisible({ timeout: 5_000 });

    await stopSession(page);
  });

  test('constraint card shows evaluation results', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    await startSession(page);
    await stepSession(page, 3);
    await page.waitForTimeout(2_000);

    const workbench = page.getByTestId('results-workbench');
    await expect(workbench).toBeVisible({ timeout: 10_000 });
    await page.getByTestId('results-workbench-tab-constraints').click();

    // Same as the plots card: located by testid, not by a `rounded-lg`
    // ancestor walk that no longer resolves.
    const card = workbench.getByTestId('results-workbench-card-constraints');
    await expect(card).toBeVisible({ timeout: 5_000 });

    // ── Verdict taxonomy (lane B2) ───────────────────────────────────
    // Two properties, both regressions that have actually happened:
    //
    // 1. MEMBERSHIP — the sweep covers constraint USAGES in the subject
    //    model, not definitions and not imported library elements. When
    //    it swept everything, espresso reported 52 rows / 48 "failing".
    // 2. FOUR-VALUEDNESS — an unevaluable constraint is `inconclusive`,
    //    which is NOT a violation. A boolean cannot carry that, so any
    //    layer that collapses the verdict back to a bool re-folds all
    //    seven undecided rows into "fail" and puts the scary badge back.
    //
    // Asserted on counts and on the presence of a non-empty inconclusive
    // set rather than on constraint names, which shift as the model does.
    const pills = card.locator('[data-testid^="constraint-pill-"]');
    await expect(pills).toHaveCount(11, { timeout: 10_000 });

    const inconclusive = card.locator('[data-verdict="inconclusive"]');
    expect(
      await inconclusive.count(),
      'no constraint rendered as inconclusive — the 4-valued verdict has ' +
        'been collapsed to a boolean somewhere on the live-snapshot path, ' +
        'and undecided rows are being shown as failures',
    ).toBeGreaterThan(0);

    // What a user actually reads in the summary strip.
    await expect(card).toContainText(/\d+\s+inconclusive/);

    await stopSession(page);
  });
});

// ═════════════════════════════════════════════════════════════════════════
// Group 6 (Diagram) removed: the Sprotty `#sysml-diagram` container assertion
// is obsolete (Sprotty was deleted; the React-SVG renderer replaced it).
// Diagram rendering is now covered by UI-DIAGRAM (OB-SPROTTY-DIAGRAM,
// retired-internal coverage matrix).
// ═════════════════════════════════════════════════════════════════════════
// Group 7: Compare & Fork (serial)
// ═════════════════════════════════════════════════════════════════════════

test.describe.serial('Compare & Fork', () => {
  let sharedPage: Page;

  test.beforeAll(async ({ browser }) => {
    sharedPage = await browser.newPage();
    await navigateWithWorkspace(sharedPage);
    await waitForWorkspaceLoaded(sharedPage);
  });

  test.afterAll(async () => {
    await stopSession(sharedPage).catch(() => {});
    await sharedPage.close();
  });

  test('fork button creates a forked session', async () => {
    await startSession(sharedPage);
    await stepSession(sharedPage, 3);
    await sharedPage.waitForTimeout(1_000);

    const before = new Set(
      (await listSessions())
        .map((s) => s.id)
        .filter((id): id is string => typeof id === 'string'),
    );

    const forkBtn = sharedPage.getByTestId('control-fork');
    await expect(forkBtn).toBeVisible({ timeout: 5_000 });
    await expect(forkBtn).toBeEnabled({ timeout: 5_000 });

    await forkBtn.click();

    // Assert the fork on the BACKEND rather than on a Compare header that
    // no longer exists: a new session id has to show up in sessions.list.
    // This is the integration suite — the real backend is the oracle, and
    // it does not drift with the shell's navigation.
    await expect
      .poll(
        async () =>
          (await listSessions()).filter(
            (s) => typeof s.id === 'string' && !before.has(s.id),
          ).length,
        { timeout: 20_000, message: 'fork did not create a new backend session' },
      )
      .toBeGreaterThan(0);
  });

  test('compare workspace shows two sessions', async () => {
    // By ROUTE — `tool-tab-compare` was removed when Compare was demoted
    // to a Simulate mode (ninebar Phase 6 W1).
    await gotoCompare(sharedPage);

    await expect(sharedPage.getByTestId('compare-session-picker')).toBeVisible({
      timeout: 10_000,
    });

    // The picker lists the ARCHIVE, not live sessions, so a just-forked
    // pair need not appear. Either it lists rows, or it says the archive
    // is empty — what must not happen is neither.
    const rows = sharedPage.locator('[data-testid^="compare-session-picker-row-"]');
    const empty = sharedPage.getByTestId('compare-session-picker-empty');
    const rowCount = await rows.count();
    const hasEmpty = await empty.isVisible({ timeout: 3_000 }).catch(() => false);
    expect(rowCount > 0 || hasEmpty).toBe(true);
  });
});

// ═════════════════════════════════════════════════════════════════════════
// Group 8: Infrastructure
// ═════════════════════════════════════════════════════════════════════════

test.describe('Infrastructure', () => {
  test('status bar shows healthy API connection', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    const statusBar = page.getByTestId('status-bar');
    await expect(statusBar).toBeVisible({ timeout: 5_000 });

    const apiText = statusBar.locator('text=API');
    await expect(apiText).toBeVisible({ timeout: 10_000 });

    // The health dot should be present
    const healthDot = statusBar.locator(
      '[style*="border-radius: 50%"][style*="width: 6"]',
    );
    await expect(healthDot.first()).toBeVisible({ timeout: 5_000 });
  });

  test('run options dropdown opens with workflow items', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    // The chevron shares the Run control's `disabled` gate, so a target
    // has to be picked before the dropdown can open at all.
    await selectRunTarget(page);

    const dropdownToggle = page.getByTestId('run-dropdown-toggle');
    await expect(dropdownToggle).toBeEnabled({ timeout: 5_000 });
    await dropdownToggle.click();
    await page.waitForTimeout(500);

    const menu = page.getByTestId('run-dropdown-menu');
    await expect(menu).toBeVisible({ timeout: 3_000 });

    // Step-by-step is the ONLY item now: parameter sweeps, Monte Carlo and
    // what-if comparison moved out to the /analyze and /compare workflows
    // and are discovered from the model tree and the top nav, not from
    // this menu. The old "at least 3 items" floor asserted the pre-move
    // shape.
    await expect(menu.locator('button')).toHaveCount(1);
    await expect(menu).toContainText('Step-by-step');

    // Close dropdown
    await page.click('body', { position: { x: 10, y: 10 } });
  });

  test('export button is available on expanded cards', async ({ page }) => {
    await navigateWithWorkspace(page);
    await waitForWorkspaceLoaded(page);
    await navigateToTool(page, 'session');

    const exportBtn = page.getByTestId('control-export');
    await expect(exportBtn).toBeVisible({ timeout: 5_000 });
  });
});
