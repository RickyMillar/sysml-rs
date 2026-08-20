/**
 * ninebar Analyze, re-composed — /analyze route smoke test (Phase 5).
 *
 * Runs under the `ninebar` Playwright project (matched by the
 * `ninebar*.spec.ts` glob). Offline-stub pattern (mirrors
 * ninebar-run/verify): every backend call is intercepted so the spec
 * runs without a live `sysml-api` — no workspace is loaded, so every
 * method body is in its teaching empty state. Asserts the Phase 5
 * five-slot contract (plan §3 DoD): each method's viewer is the whole
 * primary surface, configuration lives in a MODAL (no resident config
 * column mounts flag-on), the rail carries the study summary + Run, and
 * the bottom strip carries the batch lifecycle.
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

async function stubBackend(page: Page) {
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

async function gotoAnalyze(page: Page, sub = '') {
  await stubBackend(page);
  await page.goto(`${APP}/analyze${sub}?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
  await expect(page.getByTestId('analyze-workflow-ninebar')).toBeVisible();
}

test('the Analyze shell is the ninebar method-tab row, not the legacy mode bar', async ({ page }) => {
  await gotoAnalyze(page);

  await expect(page.getByTestId('analyze-method-tabs')).toBeVisible();
  await expect(page.getByTestId('analyze-method-sweep')).toBeVisible();

  // Legacy shell chrome must NOT mount under the flag.
  await expect(page.getByTestId('analyze-workflow')).toHaveCount(0);
  await expect(page.getByTestId('analyze-mode-tabs')).toHaveCount(0);
});

test('Sweep: viewer hero + rail config + strip — no resident config column', async ({ page }) => {
  await gotoAnalyze(page, '/sweep');

  // The hero is the primary surface, in its teaching empty state offline.
  const primary = page.getByTestId('primary-outlet');
  await expect(primary.getByTestId('sweep-hero')).toBeVisible();
  await expect(primary.getByTestId('sweep-hero-empty')).toBeVisible();

  // The legacy two-column body must NOT be mounted.
  await expect(page.getByTestId('sweep-config')).toHaveCount(0);
  await expect(page.getByTestId('sweep-results')).toHaveCount(0);

  // Rail carries the factor summary + Run; mounting it replaces the
  // interim WorkspaceBar.
  const leftRail = page.getByTestId('left-rail');
  await expect(leftRail.getByTestId('sweep-rail')).toBeVisible();
  await expect(leftRail.getByTestId('sweep-rail-run')).toBeVisible();
  await expect(leftRail.getByTestId('sweep-rail-run')).toBeDisabled();
  await expect(page.getByTestId('workspace-bar')).toHaveCount(0);

  // Bottom strip carries the batch lifecycle.
  const strip = page.getByTestId('bottom-strip');
  await expect(strip).toHaveAttribute('data-state', 'open');
  await expect(strip.getByTestId('analyze-batch-strip')).toBeVisible();
});

test('Sweep: configuration lives in the modal; edits survive close (no apply step)', async ({ page }) => {
  await gotoAnalyze(page, '/sweep');

  await page.getByTestId('sweep-hero-configure').click();
  await expect(page.getByTestId('sweep-config-modal')).toBeVisible();
  await expect(page.getByTestId('modal-title')).toHaveText('Configure sweep');

  // Free-form parameter add (offline → no discovered candidates).
  await page.getByTestId('sweep-modal-parameter-search').fill('I_residual');
  await page.getByTestId('sweep-modal-parameter-add').click();
  await expect(page.getByTestId('sweep-modal-range-I_residual')).toBeVisible();
  // Default grid 0→1 step 0.25 = 5 combinations.
  await expect(page.getByTestId('sweep-modal-child-count')).toContainText('5');

  // Close — the rail keeps the study (store-backed, no apply step).
  await page.getByTestId('modal-close').click();
  await expect(page.getByTestId('sweep-config-modal')).toHaveCount(0);
  await expect(page.getByTestId('sweep-rail-factor-I_residual')).toBeVisible();
  await expect(page.getByTestId('sweep-rail-summary')).toContainText('5');
});

test('Monte Carlo: viewer hero + modal config with reused distribution editor', async ({ page }) => {
  await gotoAnalyze(page, '/montecarlo');

  await expect(page.getByTestId('mc-hero-empty')).toBeVisible();
  await expect(page.getByTestId('montecarlo-config')).toHaveCount(0);
  await expect(page.getByTestId('left-rail').getByTestId('mc-rail')).toBeVisible();

  await page.getByTestId('mc-hero-configure').click();
  await expect(page.getByTestId('mc-config-modal')).toBeVisible();
  await page.getByTestId('mc-modal-parameter-search').fill('I_residual');
  await page.getByTestId('mc-modal-parameter-add').click();
  await expect(page.getByTestId('mc-modal-distributions')).toBeVisible();
  await page.getByTestId('modal-close').click();
  await expect(page.getByTestId('mc-rail-distribution-I_residual')).toBeVisible();
});

test('Trade Study + Sensitivity: hero empty states, rails, no resident config', async ({ page }) => {
  await gotoAnalyze(page, '/trade-study');
  await expect(page.getByTestId('tradestudy-hero-empty')).toBeVisible();
  await expect(page.getByTestId('tradestudy-config')).toHaveCount(0);
  await expect(page.getByTestId('left-rail').getByTestId('tradestudy-rail')).toBeVisible();

  await page.getByTestId('analyze-method-sensitivity').click();
  await expect(page.getByTestId('sensitivity-hero-empty')).toBeVisible();
  await expect(page.getByTestId('sensitivity-config')).toHaveCount(0);
  await expect(page.getByTestId('left-rail').getByTestId('sensitivity-rail')).toBeVisible();

  // Sensitivity config modal: method pills flip the sampler knobs.
  await page.getByTestId('sensitivity-hero-configure').click();
  await expect(page.getByTestId('sensitivity-config-modal')).toBeVisible();
  await expect(page.getByTestId('sensitivity-modal-morris-r')).toBeVisible();
  await page.getByTestId('sensitivity-modal-method-sobol').click();
  await expect(page.getByTestId('sensitivity-modal-sobol-n')).toBeVisible();
});
