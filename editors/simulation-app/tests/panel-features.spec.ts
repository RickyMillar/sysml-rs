/**
 * Panel feature validation for simulation-app.
 *
 * Tests the three new placeholder panels:
 * 1. Swimlane Timeline (Simulate mode) — orchestrator + SM sessions
 * 2. Sensitivity Sweep (Verify mode) — parameter sweep visualization
 * 3. Element Inspector (Model mode) — click-to-inspect
 *
 * Requires:
 *   - API server: cargo run -p sysml-api (port 8080)
 *   - Simulation app: vite on port 3010
 */
import { test, expect, Page } from '@playwright/test';
import { EXAMPLES } from './helpers';

// Every test here drives the pre-ninebar shell and cannot pass as written.
// It loads models from `editors/diagram/examples` (EXAMPLES), deleted with
// every fixture it names — sim-state-machine, sim-constraints, demo-solver,
// demo-continuous-time; it waits on `#sysml-diagram svg`, the Sprotty mount
// point, which no element in src/ renders any more; and it navigates by a
// mode icon rail (model/simulate/verify/study/montecarlo) that the shell no
// longer has. This file only loads — unlike the specs excluded in
// playwright.config.ts — because it declares those dead helpers inline
// instead of importing them. Porting it to the ninebar shell is a rewrite.
test.skip(true, 'targets the deleted pre-ninebar shell and the deleted editors/diagram/examples workspace');

const APP = 'http://localhost:3010';
const API = 'http://localhost:8080';
const SCREENSHOT_DIR = '/tmp/sim-app-panels';

import * as fs from 'fs';
try { fs.mkdirSync(SCREENSHOT_DIR, { recursive: true }); } catch {}

// ── Helpers ──────────────────────────────────────────────────────────────

async function loadFile(page: Page, filename: string) {
  // Load file via API directly
  const res = await page.evaluate(async (args) => {
    const r = await fetch('/files', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: `${args.examples}/${args.filename}` }),
    });
    return r.json();
  }, { examples: EXAMPLES, filename });
  return res;
}

async function waitForDiagram(page: Page) {
  // Wait for the Sprotty SVG to appear
  await page.waitForSelector('#sysml-diagram svg', { timeout: 15000 }).catch(() => null);
  await page.waitForTimeout(2000); // Let ELK layout settle
}

async function clickSidebarMode(page: Page, mode: 'model' | 'simulate' | 'verify' | 'study' | 'montecarlo') {
  const icons: Record<string, string> = {
    model: 'layers',
    simulate: 'play_arrow',
    verify: 'verified_user',
    study: 'balance',
    montecarlo: 'casino',
  };
  const icon = icons[mode];
  await page.locator(`text=${icon}`).first().click();
  await page.waitForTimeout(500);
}

// ── Pre-check: API is alive ──────────────────────────────────────────────

test.beforeAll(async () => {
  const res = await fetch(`${API}/health`);
  expect(res.ok).toBeTruthy();
});

// ── 1. Element Inspector (Model mode) ────────────────────────────────────

test.describe('Element Inspector', () => {
  test('loads file and shows diagram', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-state-machine.sysml');
    await waitForDiagram(page);

    // Should see the diagram SVG
    const svg = page.locator('#sysml-diagram svg');
    await expect(svg).toBeVisible();
    await page.screenshot({ path: `${SCREENSHOT_DIR}/model-diagram-loaded.png` });
  });

  test('clicking diagram element shows inspector', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-state-machine.sysml');
    await waitForDiagram(page);

    // We're in Model mode by default — click on an SVG element
    const svgElements = page.locator('#sysml-diagram svg [id]:not([id^="root-v"])');
    const count = await svgElements.count();
    if (count > 0) {
      await svgElements.first().click();
      await page.waitForTimeout(1000);

      // Inspector should show something other than default placeholder
      const rightPanel = page.locator('text=ELEMENT INSPECTOR');
      await expect(rightPanel).toBeVisible();
      await page.screenshot({ path: `${SCREENSHOT_DIR}/model-inspector-clicked.png` });
    }
  });
});

// ── 2. Simulate Mode — Swimlane Timeline ─────────────────────────────────

test.describe('Simulate Mode — SM Session', () => {
  test('start SM simulation and see sequence tab', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-state-machine.sysml');
    await waitForDiagram(page);

    // Switch to Simulate mode
    await clickSidebarMode(page, 'simulate');
    await page.waitForTimeout(500);

    // Click Run button
    const runBtn = page.locator('button', { hasText: /Run/ });
    await expect(runBtn).toBeVisible({ timeout: 5000 });
    await runBtn.click();
    await page.waitForTimeout(1500);

    // Should see step counter
    await expect(page.locator('text=/STEP \\d/')).toBeVisible({ timeout: 5000 });

    // Step a few times
    const stepBtn = page.locator('button', { hasText: /Step/ });
    for (let i = 0; i < 3; i++) {
      await stepBtn.click();
      await page.waitForTimeout(500);
    }

    // Bottom panel should appear with SEQUENCE tab
    const seqTab = page.locator('button', { hasText: 'SEQUENCE' });
    if (await seqTab.isVisible()) {
      await seqTab.click();
      await page.waitForTimeout(300);
    }

    await page.screenshot({ path: `${SCREENSHOT_DIR}/simulate-sm-sequence.png` });

    // Stop
    const stopBtn = page.locator('button', { hasText: /Stop/ });
    if (await stopBtn.isVisible()) await stopBtn.click();
  });
});

test.describe('Simulate Mode — Orchestrator Session', () => {
  test('start orchestrator and see timeline tab', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'demo-continuous-time.sysml');
    await waitForDiagram(page);

    // Switch to Simulate mode
    await clickSidebarMode(page, 'simulate');
    await page.waitForTimeout(500);

    // Run button should say "(orchestrator)"
    const runBtn = page.locator('button', { hasText: /Run/ });
    await expect(runBtn).toBeVisible({ timeout: 5000 });
    await runBtn.click();
    await page.waitForTimeout(2000);

    // Step several times to accumulate timeline data
    const stepBtn = page.locator('button', { hasText: /Step/ });
    if (await stepBtn.isVisible()) {
      for (let i = 0; i < 5; i++) {
        await stepBtn.click();
        await page.waitForTimeout(500);
      }
    }

    // Check for TIMELINE tab in bottom panel
    const timelineTab = page.locator('button', { hasText: 'TIMELINE' });
    if (await timelineTab.isVisible()) {
      await timelineTab.click();
      await page.waitForTimeout(500);

      // Should see SVG swimlane timeline
      const timelineSvg = page.locator('.flex-1.overflow-auto svg');
      const hasSvg = await timelineSvg.count();
      if (hasSvg > 0) {
        await expect(timelineSvg.first()).toBeVisible();
      }
    }

    // Also check CHARTS tab
    const chartsTab = page.locator('button', { hasText: 'CHARTS' });
    if (await chartsTab.isVisible()) {
      await chartsTab.click();
      await page.waitForTimeout(500);
    }

    await page.screenshot({ path: `${SCREENSHOT_DIR}/simulate-orchestrator-timeline.png` });

    // Stop
    const stopBtn = page.locator('button', { hasText: /Stop/ });
    if (await stopBtn.isVisible()) await stopBtn.click();
  });

  test('speed dial changes autoplay rate', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-state-machine.sysml');
    await waitForDiagram(page);

    await clickSidebarMode(page, 'simulate');
    await page.waitForTimeout(500);

    const runBtn = page.locator('button', { hasText: /Run/ });
    await runBtn.click();
    await page.waitForTimeout(1500);

    // Speed dial buttons should be visible
    const speed2x = page.locator('button', { hasText: '2x' });
    const speed5x = page.locator('button', { hasText: '5x' });
    await expect(speed2x).toBeVisible();
    await expect(speed5x).toBeVisible();

    // Click 5x
    await speed5x.click();
    await page.waitForTimeout(300);

    await page.screenshot({ path: `${SCREENSHOT_DIR}/simulate-speed-dial.png` });

    // Stop
    const stopBtn = page.locator('button', { hasText: /Stop/ });
    if (await stopBtn.isVisible()) await stopBtn.click();
  });
});

// ── 3. Verify Mode — Sensitivity Sweep ───────────────────────────────────

test.describe('Verify Mode', () => {
  test('run constraint check and see results', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-constraints.sysml');
    await waitForDiagram(page);

    // Switch to Verify mode
    await clickSidebarMode(page, 'verify');
    await page.waitForTimeout(500);

    // Click CHECK ALL
    const checkBtn = page.locator('button', { hasText: /CHECK ALL/ });
    await expect(checkBtn).toBeVisible({ timeout: 5000 });
    await checkBtn.click();
    await page.waitForTimeout(2000);

    // Should see pass/fail counts
    const passText = page.locator('text=/\\d+ PASS/');
    await expect(passText).toBeVisible({ timeout: 5000 });

    // Should see constraint rows
    const constraintRows = page.locator('[style*="border-left: 2px solid"]');
    const rowCount = await constraintRows.count();
    expect(rowCount).toBeGreaterThan(0);

    await page.screenshot({ path: `${SCREENSHOT_DIR}/verify-constraints.png` });
  });

  test('sensitivity tab shows sweep controls', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-constraints.sysml');
    await waitForDiagram(page);

    await clickSidebarMode(page, 'verify');
    await page.waitForTimeout(500);

    // Run check first
    const checkBtn = page.locator('button', { hasText: /CHECK ALL/ });
    await checkBtn.click();
    await page.waitForTimeout(2000);

    // Click SENSITIVITY tab
    const sensTab = page.locator('button', { hasText: 'SENSITIVITY' });
    await expect(sensTab).toBeVisible();
    await sensTab.click();
    await page.waitForTimeout(500);

    // Should see input fields
    const paramInput = page.locator('input[placeholder*="speed"]');
    await expect(paramInput).toBeVisible();

    // Should see Sweep button
    const sweepBtn = page.locator('button', { hasText: 'Sweep' });
    await expect(sweepBtn).toBeVisible();

    await page.screenshot({ path: `${SCREENSHOT_DIR}/verify-sensitivity-controls.png` });
  });

  test('run sensitivity sweep with parameter', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'demo-solver.sysml');
    await waitForDiagram(page);

    await clickSidebarMode(page, 'verify');
    await page.waitForTimeout(500);

    // Run check first
    const checkBtn = page.locator('button', { hasText: /CHECK ALL/ });
    await checkBtn.click();
    await page.waitForTimeout(2000);

    // Click SENSITIVITY tab
    const sensTab = page.locator('button', { hasText: 'SENSITIVITY' });
    await sensTab.click();
    await page.waitForTimeout(500);

    // Enter parameter name
    const paramInput = page.locator('input[placeholder*="speed"]');
    await paramInput.fill('speed');

    // Set range
    const rangeInputs = page.locator('input').filter({ has: page.locator('[style*="width: 60px"]') });
    // Use more specific selectors
    const inputs = page.locator('.flex.gap-1.items-center input');
    const loInput = inputs.first();
    const hiInput = inputs.last();
    if (await loInput.isVisible()) {
      await loInput.fill('0');
      await hiInput.fill('200');
    }

    // Click Sweep
    const sweepBtn = page.locator('button', { hasText: 'Sweep' });
    await sweepBtn.click();
    await page.waitForTimeout(3000);

    await page.screenshot({ path: `${SCREENSHOT_DIR}/verify-sensitivity-results.png` });
  });
});

// ── 4. Coverage Matrix ───────────────────────────────────────────────────

test.describe('Verify — Coverage Matrix', () => {
  test('coverage grid shows after check', async ({ page }) => {
    await page.goto(`${APP}?workspace=${EXAMPLES}`);
    await page.waitForTimeout(1000);

    await loadFile(page, 'sim-constraints.sysml');
    await waitForDiagram(page);

    await clickSidebarMode(page, 'verify');
    await page.waitForTimeout(500);

    const checkBtn = page.locator('button', { hasText: /CHECK ALL/ });
    await checkBtn.click();
    await page.waitForTimeout(2000);

    // COVERAGE MATRIX tab should be active by default
    const coverageTab = page.locator('button', { hasText: 'COVERAGE MATRIX' });
    await expect(coverageTab).toBeVisible();

    // Should see percentage
    const pctText = page.locator('text=/%/');
    await expect(pctText).toBeVisible({ timeout: 5000 });

    await page.screenshot({ path: `${SCREENSHOT_DIR}/verify-coverage-matrix.png` });
  });
});
