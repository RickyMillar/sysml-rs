/**
 * ninebar Browse floor — /browse route smoke test (Phase 1.5).
 *
 * Runs only under the `ninebar` Playwright project (see
 * `playwright.config.ts`; matched by the `ninebar*.spec.ts` glob,
 * excluded from `default`) — non-blocking, same as
 * `tests/ninebar-shell.spec.ts`. Run explicitly:
 * `npx playwright test --project=ninebar`.
 *
 * Follows the offline-stub pattern from `tests/ninebar-shell.spec.ts` /
 * `tests/smoke.spec.ts`: every backend call is intercepted so the spec
 * runs without a live `sysml-api` process. No workspace is loaded and
 * no session is ever created — this asserts the plan's "must work with
 * zero sessions" contract for the Browse floor directly: the route
 * renders its tree + empty-state reading surface with nothing but a
 * stubbed, empty backend behind it.
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

async function gotoBrowse(page: Page) {
  await stubBackend(page);
  await page.goto(`${APP}/browse?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
}

test('Browse tab is first in the workflow switcher', async ({ page }) => {
  await gotoBrowse(page);

  const nav = page.getByTestId('tool-nav');
  await expect(nav).toBeVisible();
  const tabs = nav.locator('[data-workflow-id]');
  await expect(tabs.first()).toHaveAttribute('data-workflow-id', 'browse');
  await expect(page.getByTestId('tool-tab-browse')).toHaveAttribute('data-workflow-id', 'browse');
});

test('left rail hosts the portaled Browse tree, replacing the workspace bar', async ({ page }) => {
  await gotoBrowse(page);

  const leftRail = page.getByTestId('left-rail');
  await expect(leftRail).toBeVisible();

  // The tree is portaled into the shell's left-rail slot — attached
  // under `left-rail`, not rendered by BrowseWorkflow's own DOM
  // subtree (see `src/app/slots.tsx`).
  const tree = page.getByTestId('browse-tree');
  await expect(tree).toBeAttached();
  await expect(leftRail.getByTestId('browse-tree')).toHaveCount(1);

  // Mounting `<LeftRailContent>` replaces the interim WorkspaceBar
  // section (AppShell.tsx) — it must not still be present once Browse
  // has portaled its own content in.
  await expect(page.getByTestId('workspace-bar')).toHaveCount(0);
});

test('primary surface shows the empty-state hint with no selection and no session', async ({ page }) => {
  await gotoBrowse(page);

  await expect(page.getByTestId('primary-outlet')).toBeVisible();
  await expect(page.getByTestId('browse-workflow')).toBeVisible();

  // Source is the default view; nothing is selected yet, so the quiet
  // centered hint renders — no editor, no error, no session prompt.
  await expect(page.getByTestId('browse-reading-empty')).toBeVisible();
  await expect(page.getByTestId('browse-view-switch-source')).toHaveAttribute('data-active', 'true');

  // No session was created to reach this state.
  const sessionChip = page.getByTestId('session-switcher-chip');
  if (await sessionChip.count()) {
    await expect(sessionChip).not.toContainText(/active/i);
  }
});

test('segmented control switches between source and trace matrix', async ({ page }) => {
  await gotoBrowse(page);

  await expect(page.getByTestId('browse-reading-empty')).toBeVisible();

  await page.getByTestId('browse-view-switch-trace').click();
  await expect(page.getByTestId('browse-view-switch-trace')).toHaveAttribute('data-active', 'true');
  await expect(page.getByTestId('trace-matrix-panel-no-workspace')).toBeVisible();

  await page.getByTestId('browse-view-switch-source').click();
  await expect(page.getByTestId('browse-reading-empty')).toBeVisible();
});
