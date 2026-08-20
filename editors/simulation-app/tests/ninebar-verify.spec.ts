/**
 * ninebar Verify, re-composed — /verify route smoke test (Phase 4).
 *
 * Runs only under the `ninebar` Playwright project (matched by the
 * `ninebar*.spec.ts` glob, excluded from `default`) — non-blocking, same
 * as the other ninebar specs. Run explicitly:
 * `npx playwright test --project=ninebar`.
 *
 * Offline-stub pattern (mirrors ninebar-run/browse): every backend call
 * is intercepted so the spec runs without a live `sysml-api` — no
 * workspace is loaded, so the matrix is in its empty state. Asserts the
 * Phase 4 five-slot contract (plan §4 DoD): the verdict matrix is the
 * whole primary surface, config lives in the left rail (not an inline
 * column), the bottom strip carries the verdict rollup, and the legacy
 * two-column body (`verify-config` aside + `verify-results-workbench`)
 * is not mounted at all under the flag.
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
  // Match the other ninebar specs: a permissive array default. (The
  // verdict-history timeline is never fetched offline — with no workspace
  // loaded its URI is empty and the panel renders its "load a workspace"
  // fallback instead.)
  await page.route('http://localhost:3010/api/**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
}

async function gotoVerify(page: Page) {
  await stubBackend(page);
  await page.goto(`${APP}/verify?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
  await expect(page.getByTestId('verify-workflow-ninebar')).toBeVisible();
}

test('verdict matrix is the primary surface (not the legacy two-column body)', async ({ page }) => {
  await gotoVerify(page);

  const primary = page.getByTestId('primary-outlet');
  await expect(primary).toBeVisible();
  await expect(primary.getByTestId('verdict-matrix')).toBeVisible();

  // Offline, no workspace → the matrix's own empty state.
  await expect(primary.getByTestId('verdict-matrix-empty')).toBeVisible();

  // The legacy flag-off body must NOT be mounted under the flag.
  await expect(page.getByTestId('verify-results-workbench')).toHaveCount(0);
  await expect(page.getByTestId('verify-results')).toHaveCount(0);
});

test('config lives in the left rail, not an inline 320px aside', async ({ page }) => {
  await gotoVerify(page);

  const leftRail = page.getByTestId('left-rail');
  await expect(leftRail).toBeVisible();
  await expect(leftRail.getByTestId('verify-rail-config')).toBeVisible();

  // The run control is the rail's, and it drives verification.
  await expect(leftRail.getByTestId('verify-run')).toBeVisible();

  // Mounting `<LeftRailContent>` replaces the interim WorkspaceBar.
  await expect(page.getByTestId('workspace-bar')).toHaveCount(0);
});

test('bottom strip carries the verdict rollup', async ({ page }) => {
  await gotoVerify(page);

  const strip = page.getByTestId('bottom-strip');
  await expect(strip).toHaveAttribute('data-state', 'open');
  await expect(strip.getByTestId('verify-strip')).toBeVisible();
});

test('filter tabs and the matrix/history sub-view toggle are present', async ({ page }) => {
  await gotoVerify(page);

  // Filter tabs live in the matrix toolbar.
  await expect(page.getByTestId('verdict-matrix-filter-all')).toBeVisible();
  await expect(page.getByTestId('verdict-matrix-filter-failing')).toBeVisible();
  await expect(page.getByTestId('verdict-matrix-filter-not-run')).toBeVisible();

  // Sub-view toggle swaps the matrix for the verdict-history timeline.
  await page.getByTestId('verify-subview-history').click();
  await expect(page.getByTestId('verify-history')).toBeVisible();
  await expect(page.getByTestId('verdict-matrix')).toHaveCount(0);

  // Aggregate RETIRED as a sub-view (`0fc2cb0e` — case-as-document: the
  // rollup lives in the suite header now). Pin its absence so a
  // regression can't quietly resurrect the tab.
  await expect(page.getByTestId('verify-subview-aggregate')).toHaveCount(0);

  // Report sub-view: offline with no verdicts it shows its empty state
  // (the document surface appears once a run has produced verdicts).
  await page.getByTestId('verify-subview-report').click();
  await expect(page.getByTestId('verify-report-empty')).toBeVisible();

  await page.getByTestId('verify-subview-matrix').click();
  await expect(page.getByTestId('verdict-matrix')).toBeVisible();
});
