/**
 * Phase 7 — IntegrationsPanel Playwright spec.
 *
 * Smoke-checks that the Integrations utility drawer can be opened
 * from the toolbar, renders all three sections (MCP / REST / LSP),
 * and the "Test connection" button against a stubbed `/health`
 * surfaces the ok indicator.
 *
 * Note: sandbox env can't bootstrap the dev server (same blocker as
 * source-panel.spec / session-inspector.spec / analysis-workflow.spec).
 * The spec is written for the standard CI env.
 */
import { test, expect, type Page, type Route } from '@playwright/test';

const APP = 'http://localhost:3010';

async function stubBackend(page: Page) {
  await page.route('**/health', (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: '{"status":"ok","version":"0.1.0"}',
    }),
  );
  await page.route('**/commands', (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        { name: 'sysml.workspace.info', category: 'Query', description: '', params: [], returns: '', stateful: false },
        { name: 'sysml.query', category: 'Query', description: '', params: [], returns: '', stateful: false },
      ]),
    }),
  );
  // Catch-all empty for any incidental /api/command traffic.
  await page.route('**/api/command', (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: 'null',
    }),
  );
}

test.describe('IntegrationsPanel (Phase 7)', () => {
  test.beforeEach(async ({ page }) => {
    await stubBackend(page);
  });

  test('opens from the utility toolbar and renders MCP / REST / LSP sections', async ({
    page,
  }) => {
    await page.goto(APP);
    await page.getByTestId('utility-toggle-integrations').click();
    await expect(page.getByTestId('integrations-panel')).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId('integrations-mcp')).toBeVisible();
    await expect(page.getByTestId('integrations-rest')).toBeVisible();
    await expect(page.getByTestId('integrations-lsp')).toBeVisible();
  });

  test('Test connection succeeds against a healthy stubbed backend', async ({
    page,
  }) => {
    await page.goto(APP);
    await page.getByTestId('utility-toggle-integrations').click();
    await expect(page.getByTestId('integrations-panel')).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId('integrations-rest-test').click();
    await expect(page.getByTestId('integrations-rest-health-ok')).toBeVisible();
  });

  test('binary path field rewrites the MCP config snippets live', async ({
    page,
  }) => {
    await page.goto(APP);
    await page.getByTestId('utility-toggle-integrations').click();
    await expect(page.getByTestId('integrations-panel')).toBeVisible({
      timeout: 10_000,
    });
    await page
      .getByTestId('integrations-binary-path')
      .fill('/opt/sysml/bin/sysml-api');
    await expect(page.getByTestId('integrations-mcp-desktop')).toContainText(
      '/opt/sysml/bin/sysml-api',
    );
    await expect(page.getByTestId('integrations-mcp-code')).toContainText(
      '/opt/sysml/bin/sysml-api',
    );
  });
});
