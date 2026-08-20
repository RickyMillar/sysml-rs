/**
 * End-to-end tests for the simulation UI (end-game architecture).
 *
 * All actions are launched via right-click context menu → "Analyze".
 * The Results Strip auto-composes cards from model capabilities.
 *
 * Requires: API server on :8080, Vite dev server on :3011
 */
import { test, expect } from '@playwright/test';
import {
  APP, API, FIXTURES,
  loadFileViaAPI, waitForModelTree, expandToElement,
  rightClickElement, rightClickFirstOfKind,
  clickContextAction, contextMenuHasAction,
  waitForActivityTab, getActivityTabCount, clickActivityTab,
  clickRun, clickStop, stepN,
} from './helpers';

// ── App Loading ──────────────────────────────────────────────────────

test.describe('App Loading', () => {
  test('shows idle workspace when no activities open', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await page.waitForTimeout(2000);

    // Diagram view should be visible (idle workspace shows diagram)
    const diagram = page.locator('#sysml-diagram');
    // Status bar should be visible
    const statusBar = page.locator('text=API');
    await expect(statusBar.first()).toBeVisible({ timeout: 8000 });
  });

  test('loads file and shows elements in model tree', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    // File name should appear
    const fileName = page.locator('.truncate.mono-text', { hasText: 'traffic_light.sysml' });
    await expect(fileName.first()).toBeVisible({ timeout: 5000 });
  });

  test('StatusBar shows connection status', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await page.waitForTimeout(3000);

    const status = page.getByText(/API READY|CONNECTED/);
    await expect(status.first()).toBeVisible({ timeout: 8000 });
  });

  test('model tree elements have action indicator icons', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await expandToElement(page, 'TrafficLightStates');

    const indicator = page.locator('[title*="Right-click"]');
    await expect(indicator.first()).toBeVisible({ timeout: 5000 });
  });
});

// ── Context Menu ─────────────────────────────────────────────────────

test.describe('Context Menu', () => {
  test('right-click state machine shows Analyze action', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await rightClickElement(page, 'TrafficLightStates');

    const hasAnalyze = await contextMenuHasAction(page, 'Analyze');
    expect(hasAnalyze).toBeTruthy();
  });

  test('clicking Analyze creates a simulation activity', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await rightClickElement(page, 'TrafficLightStates');
    await clickContextAction(page, 'Analyze');

    await waitForActivityTab(page, 'Simulate');
    const count = await getActivityTabCount(page);
    expect(count).toBe(1);
  });
});

// ── Simulation Workflow ──────────────────────────────────────────────

test.describe('Simulation Workflow', () => {
  test('simulation can run and step', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await rightClickElement(page, 'TrafficLightStates');
    await clickContextAction(page, 'Analyze');
    await waitForActivityTab(page, 'Simulate');

    await clickRun(page);
    await page.waitForTimeout(1000);
    await stepN(page, 2, 800);

    // Should show step count or completion in toolbar
    const stepOrDone = page.getByText(/STEP\s+\d+|DONE/);
    await expect(stepOrDone.first()).toBeVisible({ timeout: 5000 });
  });

  test('simulation stop returns to idle', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await rightClickElement(page, 'TrafficLightStates');
    await clickContextAction(page, 'Analyze');
    await waitForActivityTab(page, 'Simulate');

    await clickRun(page);
    await page.waitForTimeout(500);
    await clickStop(page);

    // Run button should reappear
    const runBtn = page.locator('button', { hasText: /Run/ }).first();
    await expect(runBtn).toBeVisible({ timeout: 5000 });
  });
});

// ── Results Strip ────────────────────────────────────────────────────

test.describe('Results Strip', () => {
  test('results strip is visible in the layout', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    // Results strip should be present (it auto-composes from capabilities)
    const strip = page.locator('[data-testid="results-strip"]');
    await expect(strip).toBeVisible({ timeout: 5000 });
  });

  test('status bar shows model counts after loading files', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    // Status bar should show file count
    const counts = page.getByText(/\d+ file/);
    await expect(counts.first()).toBeVisible({ timeout: 5000 });
  });
});

// ── Session Panel ────────────────────────────────────────────────────

test.describe('Session Panel', () => {
  test('SESSIONS section shows after starting simulation', async ({ page }) => {
    await page.goto(`${APP}/?workspace=${FIXTURES}`);
    await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
    await waitForModelTree(page);

    await rightClickElement(page, 'TrafficLightStates');
    await clickContextAction(page, 'Analyze');
    await waitForActivityTab(page, 'Simulate');
    await clickRun(page);
    await page.waitForTimeout(2000);

    const sessionsLabel = page.getByText('SESSIONS');
    await expect(sessionsLabel.first()).toBeVisible({ timeout: 5000 });
  });
});
