/**
 * Targeted test for the autoplay flow that was crashing.
 * Exercises: load file → right-click → Analyze → Run → Auto → verify no crash.
 */
import { test, expect } from '@playwright/test';
import {
  APP, FIXTURES,
  loadFileViaAPI, waitForModelTree,
  rightClickElement, clickContextAction,
  waitForActivityTab, clickRun,
} from './helpers';

test('autoplay runs without hooks crash', async ({ page }) => {
  // Collect console errors
  const errors: string[] = [];
  page.on('pageerror', (err) => errors.push(err.message));

  await page.goto(`${APP}/?workspace=${FIXTURES}`);
  await loadFileViaAPI(page, `${FIXTURES}/traffic_light.sysml`);
  await waitForModelTree(page);

  // Right-click → Analyze → creates activity
  await rightClickElement(page, 'TrafficLightStates');
  await clickContextAction(page, 'Analyze');
  await waitForActivityTab(page, 'Simulate');

  // Click Run
  await clickRun(page);

  // Click Auto button
  const autoBtn = page.locator('button', { hasText: /Auto|Pause/ }).first();
  await expect(autoBtn).toBeVisible({ timeout: 5000 });
  await autoBtn.click();

  // Let autoplay run for 3 seconds
  await page.waitForTimeout(3000);

  // Should NOT have crashed — toolbar should still be visible
  const toolbar = page.locator('button', { hasText: /Stop|Pause|Step/ }).first();
  await expect(toolbar).toBeVisible({ timeout: 3000 });

  // No "Rendered more hooks" errors
  const hookErrors = errors.filter((e) => e.includes('hooks') || e.includes('Rendered more'));
  expect(hookErrors).toHaveLength(0);
});
