/**
 * Shared test helpers for simulation-app Playwright tests.
 */
import { Page, expect } from '@playwright/test';
import { repoPath } from './repo-paths';

export const APP = 'http://localhost:3010';
export const API = 'http://localhost:8080';
export const FIXTURES = repoPath('crates/tooling/sysml-cli/fixtures');
/**
 * Legacy demo-example workspace. `editors/diagram/` was deleted with the
 * directory no longer exists and nothing under it can be loaded.
 *
 * Its one remaining reader is `panel-features.spec.ts`, which collects but
 * is skipped in full (it targets the same deleted shell). The other readers
 * were the nine Sprotty-era specs retired under OS-D2 #6. Delete this
 * constant when `panel-features.spec.ts` is ported or retired in turn.
 */
export const EXAMPLES = repoPath('editors/diagram/examples');

// ── File Loading ──────────────────────────────────────────────────────

/** Load a file by clicking it in the file explorer. Falls back to event dispatch. */
export async function loadFileViaAPI(page: Page, filePath: string) {
  const fileName = filePath.split('/').pop() ?? filePath;

  // Try to click the file in the file explorer
  const fileBtn = page.locator(`button:has-text("${fileName}")`).first();
  if (await fileBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
    await fileBtn.click();
    await page.waitForTimeout(4000);
    return;
  }

  // Fallback: dispatch event (Shell handler calls workspaceStore.loadFile)
  await page.evaluate(async (path) => {
    window.dispatchEvent(new CustomEvent('sysml-file-loaded', { detail: { path, filePath: path } }));
  }, filePath);
  await page.waitForTimeout(5000);
}

/** Wait for the model tree to render with element nodes. */
export async function waitForModelTree(page: Page) {
  const tree = page.locator('[data-testid="model-tree"]');
  await expect(tree).toBeVisible({ timeout: 8000 });
  // Wait for at least one tree node item to render inside
  await expect(tree.locator('.truncate').first()).toBeVisible({ timeout: 8000 });
}

// ── Model Tree Interaction ────────────────────────────────────────────

/**
 * The model tree container, identified by data-testid.
 */
function modelTree(page: Page) {
  return page.locator('[data-testid="model-tree"]');
}

/**
 * Find an element by name in the model tree. Expands tree path by
 * clicking chevrons that are NOT already expanded (rotation style check).
 */
export async function expandToElement(page: Page, elementName: string, timeout = 10000) {
  const tree = modelTree(page);
  await expect(tree).toBeVisible({ timeout: 5000 });

  const target = tree.getByText(elementName, { exact: true }).first();

  // Repeatedly click collapsed chevrons only (transform: rotate(0deg))
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await target.isVisible({ timeout: 500 }).catch(() => false)) {
      return target;
    }

    // Find chevron buttons that are NOT rotated (collapsed state)
    const collapsed = tree.locator('.material-symbols-outlined:text("chevron_right")').filter({
      has: page.locator(':scope'),
    });
    const allChevronSpans = tree.locator('.material-symbols-outlined:text("chevron_right")');
    const count = await allChevronSpans.count();
    let clickedAny = false;

    for (let i = 0; i < count; i++) {
      const span = allChevronSpans.nth(i);
      if (!await span.isVisible()) continue;

      // Check if this chevron is NOT rotated (collapsed)
      const transform = await span.evaluate(el =>
        window.getComputedStyle(el).transform || window.getComputedStyle(el).getPropertyValue('transform')
      ).catch(() => 'none');

      // rotate(90deg) = matrix(0, 1, -1, 0, 0, 0); unrotated = "none" or matrix(1,0,0,1,0,0)
      const isCollapsed = transform === 'none' || transform.includes('matrix(1');

      if (isCollapsed) {
        // Click the parent button of this span
        const btn = span.locator('xpath=ancestor::button').first();
        if (await btn.isVisible()) {
          await btn.click();
          clickedAny = true;
          await page.waitForTimeout(200);
        }
      }
    }
    if (!clickedAny) break;
    await page.waitForTimeout(300);
  }

  await expect(target).toBeVisible({ timeout: 3000 });
  return target;
}

/**
 * Right-click an element in the model tree. Expands tree if needed.
 */
export async function rightClickElement(page: Page, elementName: string) {
  const el = await expandToElement(page, elementName);
  await el.click({ button: 'right' });
  await page.waitForTimeout(400);
}

/**
 * Right-click the first element that has a kind badge matching the given abbreviation.
 * Expands tree first, then finds the badge and right-clicks its row.
 */
export async function rightClickFirstOfKind(page: Page, kindAbbrev: string) {
  const tree = modelTree(page);
  await expect(tree).toBeVisible({ timeout: 5000 });

  // Expand all tree nodes
  await expandAllModelTree(page);

  // Find the kind badge text within the model tree
  const badge = tree.locator('.uppercase.tracking-wider', { hasText: kindAbbrev }).first();
  await expect(badge).toBeVisible({ timeout: 5000 });

  // Right-click the badge
  await badge.click({ button: 'right' });
  await page.waitForTimeout(400);
}

/** Expand all collapsed nodes within the model tree. */
async function expandAllModelTree(page: Page) {
  const tree = modelTree(page);
  for (let round = 0; round < 5; round++) {
    const chevrons = tree.locator('button:has(.material-symbols-outlined:text("chevron_right"))');
    const count = await chevrons.count();
    let clickedAny = false;
    for (let i = 0; i < count; i++) {
      const chevron = chevrons.nth(i);
      if (await chevron.isVisible()) {
        await chevron.click();
        clickedAny = true;
        await page.waitForTimeout(150);
      }
    }
    if (!clickedAny) break;
    await page.waitForTimeout(200);
  }
}

// ── Context Menu ──────────────────────────────────────────────────────

/** Click an action in the context menu portal. */
export async function clickContextAction(page: Page, actionLabel: string) {
  // Context menu is a portal with rounded-lg + shadow-lg (distinguishes from sidebar)
  const menu = page.locator('.rounded-lg.shadow-lg');
  await expect(menu).toBeVisible({ timeout: 3000 });
  const action = menu.locator('button', { hasText: actionLabel }).first();
  await expect(action).toBeVisible({ timeout: 2000 });
  await action.click();
  await page.waitForTimeout(500);
}

/** Check if context menu is showing a specific action. */
export async function contextMenuHasAction(page: Page, actionLabel: string): Promise<boolean> {
  const menu = page.locator('.rounded-lg.shadow-lg');
  if (!await menu.isVisible({ timeout: 1000 }).catch(() => false)) return false;
  const action = menu.locator('button', { hasText: actionLabel }).first();
  return action.isVisible({ timeout: 1000 }).catch(() => false);
}

// ── Activity Tabs ─────────────────────────────────────────────────────

/** Wait for an activity tab to appear in the ActivityBar. */
export async function waitForActivityTab(page: Page, labelSubstring: string, timeout = 5000) {
  // Activity tabs are in the bar with height 32px, have min-width 120px
  const tab = page.locator('[style*="min-width: 120px"]', { hasText: labelSubstring }).first();
  await expect(tab).toBeVisible({ timeout });
}

/** Count activity tabs in the ActivityBar. */
export async function getActivityTabCount(page: Page): Promise<number> {
  return page.locator('[style*="min-width: 120px"][style*="max-width: 200px"]').count();
}

/** Click an activity tab by label substring. */
export async function clickActivityTab(page: Page, labelSubstring: string) {
  const tab = page.locator('[style*="min-width: 120px"]', { hasText: labelSubstring }).first();
  await tab.click();
  await page.waitForTimeout(300);
}

// ── Simulation Controls ───────────────────────────────────────────────

/** Click the Run button in the toolbar. */
export async function clickRun(page: Page) {
  const btn = page.locator('button', { hasText: /^\u25B6\s*Run|Run/ }).first();
  await expect(btn).toBeVisible({ timeout: 5000 });
  await btn.click();
  await page.waitForTimeout(1500);
}

/** Click Step button N times with delay. */
export async function stepN(page: Page, n: number, delayMs = 500) {
  for (let i = 0; i < n; i++) {
    const btn = page.locator('button', { hasText: 'Step' }).first();
    if (await btn.isVisible()) {
      await btn.click();
      await page.waitForTimeout(delayMs);
    }
  }
}

/** Click Stop button. */
export async function clickStop(page: Page) {
  const btn = page.locator('button', { hasText: 'Stop' }).first();
  if (await btn.isVisible()) await btn.click();
  await page.waitForTimeout(500);
}
