/**
 * S4.T4 — Source utility panel Playwright spec.
 *
 * Verifies the minimum Monaco mount path:
 *   1. The Source utility toggle is present in every workflow.
 *   2. Opening it with no selection shows the empty hint, not the editor.
 *   3. Stubbing a selection + `sysml.get_source` response renders the
 *      Monaco editor wrapper.
 *
 * Backend-independent — all `/api/command` traffic is intercepted so
 * the spec runs without sysml-api on :8080. The actual Monaco bundle
 * still loads from the dev server, so the test asserts on our wrapping
 * `[data-testid="source-panel-editor"]` div rather than poking inside
 * Monaco's own DOM.
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
  // Default: any sysml.get_source call returns the canned package source.
  await page.route('**/api/command', async (route) => {
    const body = route.request().postDataJSON() as { command?: string };
    if (body?.command === 'sysml.get_source') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          text: 'package Stubbed {\n    part def Thing;\n}',
          start: 0,
          end: 36,
          line: 1,
          col: 1,
        }),
      });
    }
    // Generic empty success for other commands.
    return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
  });
}

test.describe('Source utility panel (T4)', () => {
  test.beforeEach(async ({ page }) => {
    await stubBackend(page);
  });

  test('toggle is present and starts closed', async ({ page }) => {
    await page.goto(`${APP}/run`);
    const toggle = page.getByTestId('utility-toggle-source');
    await expect(toggle).toBeVisible({ timeout: 10_000 });
    // No drawer yet — Source isn't auto-open.
    await expect(page.getByTestId('utility-drawer')).toHaveCount(0);
  });

  test('opens to the empty hint when no file is focused', async ({ page }) => {
    await page.goto(`${APP}/run`);
    await page.getByTestId('utility-toggle-source').click();
    await expect(page.getByTestId('utility-drawer')).toBeVisible();
    await expect(page.getByTestId('source-panel-empty')).toBeVisible();
  });

  test('renders the live Monaco editor once a file is focused', async ({ page }) => {
    await page.goto(`${APP}/run`);
    await page.getByTestId('utility-toggle-source').click();

    // Phase 1 — the panel renders the FOCUSED FILE, not a per-element
    // slice. Drive the workspace store directly so the spec doesn't
    // need a real backend file-load round-trip.
    await page.evaluate(() => {
      const ws = (window as unknown as {
        __workspaceStoreForTests?: {
          setFocusedFile: (uri: string, source: string) => void;
        };
      }).__workspaceStoreForTests;
      if (ws) ws.setFocusedFile('file:///stubbed.sysml', 'package Stubbed {\n    part def Thing;\n}');
    });

    await expect(page.getByTestId('source-panel-editor')).toBeVisible({ timeout: 15_000 });
  });
});
