/** Focused diagnostics: (a) does drag move the node LIVE (before drop
 *  reflow)? (b) does wheel reach d3-zoom (__zoom transform)? */
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '..', '..', '..', '..');
const APP = 'http://127.0.0.1:3010';
const WORKSPACE = join(REPO, 'examples/view-showcase');

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(`${APP}/run?workspace=${encodeURIComponent(WORKSPACE)}`);
await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, { timeout: 30000 });
const viewId = await page.evaluate(async () => {
  for (let i = 0; i < 30; i++) {
    const res = await fetch('/api/command', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ command: 'sysml.query', params: { uri: '__workspace__', spec: { filter: { type: 'view', viewpoint_id: null }, projection: 'summary_expand', limit: 1000 } } }),
    });
    if (res.ok) {
      const { rows } = await res.json();
      const hit = (rows ?? []).find((r) => r.name === 'OverviewView');
      if (hit) return hit.id;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  return null;
});
await page.evaluate((id) => window.__workspaceStoreForTests.setSelectedViewId(id), viewId);
await page.waitForSelector('[data-testid="svg-canvas"]', { timeout: 20000 });
await page.waitForTimeout(1500);

const engine = page.locator('[data-testid="svg-canvas"] g[data-element-id]', { hasText: 'engine : Engine' }).last();
const b0 = await engine.boundingBox();

// Drag and sample DURING the drag (before pointerup).
await page.mouse.move(b0.x + b0.width / 2, b0.y + 12);
await page.mouse.down();
await page.mouse.move(b0.x + b0.width / 2 + 150, b0.y + 102, { steps: 10 });
const during = await engine.boundingBox();
console.log('drag live Δ:', Math.round(during.x - b0.x), Math.round(during.y - b0.y));
await page.mouse.up();
await page.waitForTimeout(300);
const justDropped = await engine.boundingBox();
console.log('right after drop Δ:', Math.round(justDropped.x - b0.x), Math.round(justDropped.y - b0.y));
await page.waitForTimeout(2000);
const afterReflow = await engine.boundingBox();
console.log('after reflow Δ:', Math.round(afterReflow.x - b0.x), Math.round(afterReflow.y - b0.y));

// Wheel → read the svg's d3 __zoom state directly.
const zoomState = () =>
  page.evaluate(() => {
    const svg = document.querySelector('[data-testid="svg-canvas"]');
    return svg && svg.__zoom ? { k: svg.__zoom.k, x: Math.round(svg.__zoom.x), y: Math.round(svg.__zoom.y) } : null;
  });
console.log('zoom before wheel:', await zoomState());
await page.mouse.move(900, 500);
await page.mouse.wheel(0, -400);
await page.waitForTimeout(300);
console.log('zoom after wheel(-400):', await zoomState());
// Also try dispatching a wheel event directly on the svg.
await page.evaluate(() => {
  const svg = document.querySelector('[data-testid="svg-canvas"]');
  svg.dispatchEvent(new WheelEvent('wheel', { deltaY: -400, clientX: 900, clientY: 500, bubbles: true, cancelable: true }));
});
await page.waitForTimeout(300);
console.log('zoom after synthetic wheel:', await zoomState());
// Drag on empty canvas = pan?
const pan0 = await zoomState();
await page.mouse.move(500, 850);
await page.mouse.down();
await page.mouse.move(700, 750, { steps: 8 });
await page.mouse.up();
console.log('pan: ', pan0, '→', await zoomState());
await browser.close();
