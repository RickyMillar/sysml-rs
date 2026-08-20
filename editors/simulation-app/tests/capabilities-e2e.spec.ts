/**
 * Test capabilities detection for multi-file espresso-production-cell.
 * Verifies that loading multiple files with SMs + flows → orchestrator mode.
 */
import { test, expect } from '@playwright/test';
import { APP } from './helpers';
import { repoPath } from './repo-paths';

const GS = repoPath('examples/espresso-production-cell');
const GS_FILES = [
  `${GS}/Behaviour/ProtectionRace.sysml`,
  `${GS}/Behaviour/BreakerRelayCoordinator.sysml`,
  `${GS}/Structure/ProductionCell.sysml`,
];

test('multi-file espresso-production-cell uses orchestrator mode', async ({ page }) => {
  await page.goto(`${APP}/?workspace=${GS}`);
  await page.waitForTimeout(2000);

  // Load files via event dispatch
  for (const file of GS_FILES) {
    await page.evaluate((path) => {
      window.dispatchEvent(new CustomEvent('sysml-file-loaded', { detail: { path } }));
    }, file);
    await page.waitForTimeout(4000);
  }

  // Wait for model tree to appear
  const tree = page.locator('[data-testid="model-tree"]');
  await expect(tree).toBeVisible({ timeout: 8000 });

  // Status bar should show multi-file info
  const fileCount = page.getByText(/\d+ file/);
  await expect(fileCount.first()).toBeVisible({ timeout: 5000 });

  // Right-click first visible element in tree and click Analyze
  const elements = tree.locator('.truncate.mono-text').filter({ hasNotText: /\.sysml/ });
  const firstElement = elements.first();
  await expect(firstElement).toBeVisible({ timeout: 5000 });
  await firstElement.click({ button: 'right' });
  await page.waitForTimeout(500);

  const analyzeBtn = page.locator('.rounded-lg.shadow-lg button', { hasText: 'Analyze' });
  await expect(analyzeBtn).toBeVisible({ timeout: 3000 });
  await analyzeBtn.click();
  await page.waitForTimeout(1000);

  // The Run button should show (orchestrator) for multi-file with SMs
  const runBtn = page.locator('button', { hasText: /Run/ }).first();
  await expect(runBtn).toBeVisible({ timeout: 5000 });
  const runText = await runBtn.textContent();
  expect(runText).toContain('orchestrator');
});
