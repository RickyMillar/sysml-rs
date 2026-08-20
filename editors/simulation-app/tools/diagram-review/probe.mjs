/**
 * Interaction probes (Phase 2b of diagram-modeling-review-plan.md).
 *
 * Drives the OverviewView diagram (view-showcase) through the core
 * interactions — drag, subtree carry, persistence, zoom, select, hover,
 * double-click — recording numeric observations + step screenshots.
 * Output: probe-results.json + probes/*.png in the output dir.
 *
 * Run via probes.sh (one shell invocation with api+vite — sandbox netns).
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '..', '..', '..', '..');
const OUT = process.argv[2] ?? join(HERE, '..', '..', 'test-results', 'diagram-review');
const PROBES = join(OUT, 'probes');
mkdirSync(PROBES, { recursive: true });

const APP = 'http://127.0.0.1:3010';
const WORKSPACE = join(REPO, 'examples/view-showcase');

const results = [];
const record = (name, outcome, detail) => {
  results.push({ name, outcome, ...detail });
  console.log(`  ${outcome === 'ok' ? '✓' : outcome === 'finding' ? '⚑' : '✗'} ${name}: ${detail.note ?? ''}`);
};

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 1 });
const errors = [];
page.on('console', (m) => m.type() === 'error' && errors.push(m.text().slice(0, 200)));
page.on('pageerror', (e) => errors.push(String(e).slice(0, 200)));

await page.goto(`${APP}/run?workspace=${encodeURIComponent(WORKSPACE)}`);
await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, { timeout: 30000 });

// Resolve OverviewView's id via the backend and select it.
const viewId = await page.evaluate(async () => {
  for (let i = 0; i < 30; i++) {
    const res = await fetch('/api/command', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        command: 'sysml.query',
        params: {
          uri: '__workspace__',
          spec: { filter: { type: 'view', viewpoint_id: null }, projection: 'summary_expand', limit: 1000 },
        },
      }),
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
if (!viewId) throw new Error('OverviewView not found');
await page.evaluate((id) => window.__workspaceStoreForTests.setSelectedViewId(id), viewId);
await page.waitForSelector('[data-testid="svg-canvas"]', { timeout: 20000 });
await page.waitForTimeout(1500);

const nodeByText = (text) =>
  page.locator(`[data-testid="svg-canvas"] g[data-element-id]`, { hasText: text }).last();
const bbox = async (loc) => await loc.boundingBox();
const shot = (name) => page.screenshot({ path: join(PROBES, `${name}.png`) });

await shot('00-initial');

// ── P1: drag a leaf node ─────────────────────────────────────────────
{
  const engine = nodeByText('engine : Engine');
  const b0 = await bbox(engine);
  await page.mouse.move(b0.x + b0.width / 2, b0.y + 12);
  await page.mouse.down();
  await page.mouse.move(b0.x + b0.width / 2 + 150, b0.y + 12 + 90, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(1500); // elk INTERACTIVE re-run on drop
  const b1 = await bbox(engine);
  await shot('01-after-leaf-drag');
  const moved = Math.abs(b1.x - b0.x) + Math.abs(b1.y - b0.y);
  record('drag-leaf-node', moved > 40 ? 'ok' : 'finding', {
    note: `bbox moved ${Math.round(b1.x - b0.x)},${Math.round(b1.y - b0.y)} (px)`,
    before: b0,
    after: b1,
  });
}

// ── P2: container drag carries subtree ───────────────────────────────
{
  const vehicle = nodeByText('Vehicle');
  const engine = nodeByText('engine : Engine');
  const v0 = await bbox(vehicle);
  const e0 = await bbox(engine);
  // Grab the container by its header strip (top center), not the middle
  // (children overlap the middle).
  await page.mouse.move(v0.x + v0.width / 2, v0.y + 8);
  await page.mouse.down();
  await page.mouse.move(v0.x + v0.width / 2 - 120, v0.y + 8 + 60, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(1500);
  const v1 = await bbox(vehicle);
  const e1 = await bbox(engine);
  await shot('02-after-container-drag');
  const vMoved = Math.abs(v1.x - v0.x) > 40;
  const carried = Math.abs(e1.x - e0.x) > 40;
  record('container-drag-carries-subtree', vMoved && carried ? 'ok' : 'finding', {
    note: `vehicle Δ${Math.round(v1.x - v0.x)},${Math.round(v1.y - v0.y)}; engine Δ${Math.round(e1.x - e0.x)},${Math.round(e1.y - e0.y)}`,
  });
}

// ── P3: does the manual position survive a view switch? ──────────────
{
  const engine = nodeByText('engine : Engine');
  const before = await bbox(engine);
  await page.evaluate((id) => {
    window.__workspaceStoreForTests.setSelectedViewId(null);
    window.__workspaceStoreForTests.setSelectedViewId(id);
  }, viewId);
  await page.waitForSelector('[data-testid="svg-canvas"]', { timeout: 20000 });
  await page.waitForTimeout(1500);
  const after = await bbox(nodeByText('engine : Engine'));
  await shot('03-after-reselect');
  const kept = Math.abs(after.x - before.x) < 10 && Math.abs(after.y - before.y) < 10;
  record('position-survives-reselect', kept ? 'ok' : 'finding', {
    note: kept ? 'manual position retained' : `position reset (Δ${Math.round(after.x - before.x)},${Math.round(after.y - before.y)})`,
  });
}

// ── P4: wheel zoom + LOD pill ────────────────────────────────────────
{
  const pill = () => page.locator('[data-testid="svg-canvas-lod-pill"]').textContent().catch(() => null);
  const p0 = await pill();
  await page.mouse.move(800, 500);
  await page.mouse.wheel(0, -600);
  await page.waitForTimeout(400);
  const p1 = await pill();
  await page.mouse.wheel(0, 1800);
  await page.waitForTimeout(400);
  const p2 = await pill();
  await shot('04-after-zoom-out');
  record('wheel-zoom-updates-lod', p0 !== p1 || p1 !== p2 ? 'ok' : 'finding', {
    note: `pill: "${p0}" → "${p1}" → "${p2}"`,
  });
}

// Restore a workable zoom before the click probes: P4 leaves the canvas at
// glyph LOD (now that wheel-zoom works), where compartment text — the
// locator anchor for P5/P6 — isn't rendered. Wheel back by the inverse of
// P4's net delta (-600 + 1800 → -1200 restores ~fit scale).
await page.mouse.move(800, 500);
await page.mouse.wheel(0, -1200);
await page.waitForTimeout(500);

// ── P5: click-select opens context / highlights ──────────────────────
{
  const gearbox = nodeByText('gearbox : Gearbox');
  await gearbox.click({ position: { x: 30, y: 10 } });
  await page.waitForTimeout(800);
  const railOpen = await page.locator('[data-testid="right-rail"][data-state="open"]').count();
  const railText = railOpen
    ? (await page.locator('[data-testid="right-rail"]').textContent())?.slice(0, 120)
    : null;
  await shot('05-after-click-select');
  record('click-select', railOpen ? 'ok' : 'finding', {
    note: railOpen ? `right rail opened: ${railText?.slice(0, 60)}…` : 'no rail context opened on node select',
  });
}

// ── P6: double-click (go-to-def per SvgCanvas contract) ─────────────
{
  const engine = nodeByText('engine : Engine');
  await engine.dblclick({ position: { x: 30, y: 10 } });
  await page.waitForTimeout(1200);
  const url = page.url();
  await shot('06-after-dblclick');
  record('dblclick-go-to-def', 'observed', { note: `url now ${url.replace(APP, '')}` });
}

writeFileSync(join(OUT, 'probe-results.json'), JSON.stringify({ results, consoleErrors: errors }, null, 2));
console.log(`\nprobe-results.json + ${results.length} probes → ${PROBES}`);
await browser.close();
