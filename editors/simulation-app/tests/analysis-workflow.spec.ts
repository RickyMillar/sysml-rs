/**
 * Phase 6 — Analysis workflow Playwright spec.
 *
 * Smoke-checks the four `/analyze/<mode>` routes: the shell mounts,
 * the right tab is highlighted, and the per-mode workflow root
 * (`<mode>-workflow`) renders. Each route's command surface is
 * stubbed so the test is backend-independent — same harness
 * convention as `tests/session-inspector.spec.ts` / `source-panel.spec.ts`.
 *
 * Note: this sandbox env can't bootstrap the dev server cleanly
 * (same blocker noted for those specs). The spec is written for
 * the standard CI env where `playwright.config.ts` runs `npm run dev`.
 */
import { test, expect, type Page, type Route } from '@playwright/test';

const APP = 'http://localhost:3010';

async function stubBackend(page: Page) {
  await page.route('**/health', (route: Route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' }),
  );
  await page.route('**/api/command', async (route: Route) => {
    const body = route.request().postDataJSON() as { command?: string };
    // The shell + per-mode workflows make a lot of incidental
    // workspace/info / sessions.list / query calls on mount; respond
    // empty so the chrome renders cleanly without filling the result
    // surfaces (which is fine — we only assert the *shell* mounts).
    const empty = jsonForCommand(body?.command ?? '');
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(empty),
    });
  });
}

function jsonForCommand(command: string): unknown {
  switch (command) {
    case 'sysml.workspace.info':
      return { uris: [], root: null };
    case 'sysml.query':
      return { items: [], page: { cursor: null, has_more: false } };
    case 'sysml.evaluate.analysis_cases':
      return [];
    case 'sysml.sessions.list':
      return [];
    case 'sysml.command_catalog':
    case 'sysml.commands':
      return [];
    default:
      return null;
  }
}

test.describe('Analysis workflow (Phase 6)', () => {
  test.beforeEach(async ({ page }) => {
    await stubBackend(page);
  });

  test('mounts the shell + cases landing at /analyze', async ({ page }) => {
    await page.goto(`${APP}/analyze`);
    await expect(page.getByTestId('analyze-workflow')).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId('analyze-mode-tabs')).toBeVisible();
    // Cases tab is the active one (the index route).
    const cases = page.getByTestId('analyze-mode-cases');
    await expect(cases).toBeVisible();
    await expect(cases).toHaveAttribute('style', /primary-container/);
  });

  // Playwright does not have `test.each`; spell out the four routes.
  const MODES = [
    { mode: 'sweep', workflowTestId: 'sweep-workflow' },
    { mode: 'montecarlo', workflowTestId: 'montecarlo-workflow' },
    { mode: 'trade-study', workflowTestId: 'tradestudy-workflow' },
    { mode: 'sensitivity', workflowTestId: 'sensitivity-workflow' },
  ] as const;
  for (const { mode, workflowTestId } of MODES) {
    test(`navigates to /analyze/${mode} and renders ${workflowTestId}`, async ({
      page,
    }) => {
      await page.goto(`${APP}/analyze/${mode}`);
      await expect(page.getByTestId(`analyze-mode-${mode}`)).toBeVisible({
        timeout: 10_000,
      });
      await expect(page.getByTestId(`analyze-mode-${mode}`)).toHaveAttribute(
        'style',
        /primary-container/,
      );
      // The per-mode workflow root is mounted as a sibling of the
      // tab bar via react-router `<Outlet/>`.
      await expect(page.getByTestId(workflowTestId)).toBeVisible({
        timeout: 10_000,
      });
    });
  }

  test('clicking a tab swaps the rendered workflow without a full reload', async ({
    page,
  }) => {
    await page.goto(`${APP}/analyze`);
    await expect(page.getByTestId('analyze-workflow')).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId('analyze-mode-sweep').click();
    await expect(page).toHaveURL(/\/analyze\/sweep/);
    await expect(page.getByTestId('sweep-workflow')).toBeVisible();

    await page.getByTestId('analyze-mode-montecarlo').click();
    await expect(page).toHaveURL(/\/analyze\/montecarlo/);
    await expect(page.getByTestId('montecarlo-workflow')).toBeVisible();
  });
});
