/**
 * Phase 3 — source-preview popover Playwright spec.
 *
 * Verifies that hovering a click-an-element row arms the shared
 * `SourcePreviewPopover` against `sysml.get_source`, and that clicking
 * the popover promotes selection into the Source utility drawer.
 *
 * Backend-independent — `/api/command` traffic is stubbed so the spec
 * runs without `sysml-api` on :8080. Monaco bundle still loads from
 * the dev server, so we assert on our wrapping
 * `[data-testid="sneak-peek"]` (inside the popover) rather than poking
 * Monaco's own DOM. The `monaco-live.spec.ts` precedent — also stubbed
 * — is the model.
 *
 * Why ViewsPanel rather than DiagnosticsPanel:
 *   The diagnostics panel's preview popover gates on the panel's
 *   `extractElementId` test hook, which is `() => null` by default and
 *   only set via React-level props. Stubbing it from Playwright would
 *   require an injection seam the production code doesn't need.
 *   ViewsPanel rows carry their own `view.id` element id, so the
 *   plumbing is exercised end-to-end with a single backend stub.
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

async function stubBackend(page: Page) {
  await page.route('**/health', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' }),
  );
  await page.route('**/workspace**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"uris":[]}' }),
  );
  await page.route('**/sessions**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('**/files**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('**/models/**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ id: 'el-1', kind: 'PartDefinition', name: 'Thing' }),
    }),
  );
}

test.describe('Source preview popovers (Phase 3)', () => {
  test.beforeEach(async ({ page }) => {
    await stubBackend(page);
  });

  test('hovering a views-panel row arms a source-preview popover, click promotes to Source', async ({ page }) => {
    await page.route('**/api/command', async (route) => {
      const body = route.request().postDataJSON() as { command?: string };
      if (body?.command === 'sysml.views.list') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([
            {
              id: 'view-1',
              kind: 'ViewDefinition',
              name: 'Overview',
              exposed: [],
            },
          ]),
        });
      }
      if (body?.command === 'sysml.get_source') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            text: 'view def Overview {\n}\n',
            start: 0,
            end: 22,
            line: 1,
            col: 1,
          }),
        });
      }
      return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });

    await page.goto(`${APP}/run`);

    await page.getByTestId('utility-toggle-views').click();
    await expect(page.getByTestId('utility-drawer')).toBeVisible({ timeout: 5_000 });

    const row = page.getByTestId('views-panel-row-view-1');
    await expect(row).toBeVisible({ timeout: 10_000 });

    // Hover the row and wait past the 180ms debounce — popover arms.
    await row.hover();
    const popover = page.getByTestId('views-panel-preview-view-1');
    await expect(popover).toBeVisible({ timeout: 5_000 });

    // The popover wraps a SneakPeek which renders Monaco; assert the
    // SneakPeek wrapper is present rather than poking Monaco internals.
    await expect(popover.getByTestId('sneak-peek')).toBeVisible({ timeout: 10_000 });

    // Click the popover → Source utility drawer opens.
    await popover.click();
    await expect(page.getByTestId('utility-toggle-source')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  test('mouse leaves the row before the debounce → popover stays closed', async ({ page }) => {
    await page.route('**/api/command', async (route) => {
      const body = route.request().postDataJSON() as { command?: string };
      if (body?.command === 'sysml.views.list') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([
            { id: 'flash', kind: 'ViewDefinition', name: 'Flash', exposed: [] },
          ]),
        });
      }
      return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });

    await page.goto(`${APP}/run`);
    await page.getByTestId('utility-toggle-views').click();
    const row = page.getByTestId('views-panel-row-flash');
    await expect(row).toBeVisible({ timeout: 10_000 });

    // Hover, leave well before the debounce elapses.
    await row.hover();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(220);
    await expect(page.getByTestId('views-panel-preview-flash')).toHaveCount(0);
  });
});
