/**
 * S4.T5 — Hover-popup sneak-peek Playwright spec.
 *
 * Verifies that hovering a diagram node fetches `sysml.get_source` and
 * mounts a read-only Monaco preview inside the hover popup. The popup
 * already covers kind / span / docs / value / constraint sections (R6.3);
 * this spec asserts the sneak-peek wrapper renders alongside them.
 *
 * Backend-independent — `/api/command` traffic is stubbed so the spec
 * runs without sysml-api on :8080. We don't render a real Sprotty diagram
 * either; instead we inject a fake `<g id="sysml-diagram_<id>">` into the
 * `#sysml-diagram` root (HoverPopupProvider's DOM-delegation target) and
 * dispatch a `mouseover`. The provider's 300ms debounce is honored by
 * the test (an extra wait), which mirrors real-user timing.
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
      body: JSON.stringify({
        id: 'el-99',
        kind: 'PartDefinition',
        name: 'Thing',
        qname: 'Pkg::Thing',
        spans: [],
        props: {},
      }),
    }),
  );
  await page.route('**/api/command', async (route) => {
    const body = route.request().postDataJSON() as { command?: string };
    if (body?.command === 'sysml.get_source') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          text: 'package Hover {\n    part def Thing;\n}',
          start: 0,
          end: 36,
          line: 1,
          col: 1,
        }),
      });
    }
    return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
  });
}

async function pinFocusedUri(page: Page, uri: string) {
  await page.evaluate((u) => {
    const ws = (window as unknown as {
      __workspaceStoreForTests?: { setFocusedUri: (uri: string | null) => void };
    }).__workspaceStoreForTests;
    if (ws) ws.setFocusedUri(u);
  }, uri);
}

async function injectHoverNode(page: Page, elementId: string) {
  // Inject a fake Sprotty-shaped <g> as a child of the diagram root so
  // HoverPopupProvider's mouseover delegation picks it up. The id prefix
  // and `<g>` tag are the two predicates the provider relies on.
  await page.evaluate((id) => {
    const root = document.querySelector('#sysml-diagram');
    if (!root) throw new Error('diagram root #sysml-diagram is not mounted');
    const ns = 'http://www.w3.org/2000/svg';
    let svg = root.querySelector('svg');
    if (!svg) {
      svg = document.createElementNS(ns, 'svg');
      svg.setAttribute('width', '200');
      svg.setAttribute('height', '120');
      root.appendChild(svg);
    }
    const g = document.createElementNS(ns, 'g');
    g.setAttribute('id', `sysml-diagram_${id}`);
    g.setAttribute('class', 'sprotty-node');
    const rect = document.createElementNS(ns, 'rect');
    rect.setAttribute('x', '10');
    rect.setAttribute('y', '10');
    rect.setAttribute('width', '80');
    rect.setAttribute('height', '40');
    g.appendChild(rect);
    svg.appendChild(g);
  }, elementId);
}

async function dispatchHover(page: Page, elementId: string) {
  await page.evaluate((id) => {
    const g = document.getElementById(`sysml-diagram_${id}`);
    if (!g) throw new Error('hover target not present');
    const ev = new MouseEvent('mouseover', { bubbles: true, clientX: 50, clientY: 30 });
    g.dispatchEvent(ev);
  }, elementId);
}

test.describe('Hover sneak-peek (T5)', () => {
  test.beforeEach(async ({ page }) => {
    await stubBackend(page);
  });

  test('mounts the sneak-peek Monaco inside the hover popup', async ({ page }) => {
    await page.goto(`${APP}/run`);
    await expect(page.locator('#sysml-diagram')).toBeVisible({ timeout: 10_000 });

    await pinFocusedUri(page, 'file:///stubbed.sysml');
    await injectHoverNode(page, 'el-99');
    await dispatchHover(page, 'el-99');

    // Popup itself appears after the 300ms debounce + element fetch.
    await expect(page.getByTestId('hover-popup')).toBeVisible({ timeout: 5_000 });
    // Sneak-peek wrapper + Monaco mount land once `sysml.get_source` resolves.
    await expect(page.getByTestId('sneak-peek')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('sneak-peek-editor')).toBeVisible({ timeout: 15_000 });
  });
});
