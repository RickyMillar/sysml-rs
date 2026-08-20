/**
 * Laid-out geometry dump for ONE view — the post-elk node rectangles + fit box,
 * so a "why is this wide/tall" question is answered from real coordinates rather
 * than from the render. Reads `window.__diagramGeometryForTests`.
 *
 * Usage (needs api :8080 + vite :3010 in the SAME shell invocation):
 *   node tools/diagram-review/dump-geometry.mjs <workspace-root> <ViewName>
 */
import { chromium } from 'playwright';

const APP = 'http://127.0.0.1:3010';
const API = 'http://127.0.0.1:8080';
const ROOT = process.argv[2];
const WANT = process.argv[3];

const command = async (name, params) => {
  const r = await fetch(`${API}/api/command`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command: name, params }),
  });
  const j = await r.json();
  if (j.error) throw new Error(`${name}: ${JSON.stringify(j.error)}`);
  return j;
};

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(`${APP}/run?workspace=${encodeURIComponent(ROOT)}`);
await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, { timeout: 30000 });

let views = [];
for (let i = 0; i < 30; i++) {
  const res = await command('sysml.query', {
    uri: '__workspace__',
    spec: { filter: { type: 'view', viewpoint_id: null }, projection: 'summary_expand', limit: 1000 },
  });
  views = res.rows ?? [];
  if (views.length) break;
  await page.waitForTimeout(1000);
}
const v = views.find((r) => r.name === WANT);
if (!v) {
  console.error(`view ${WANT} not found; have: ${views.map((r) => r.name).join(', ')}`);
  process.exit(1);
}

for (let attempt = 0; attempt < 4; attempt++) {
  await page.evaluate((id) => {
    window.__workspaceStoreForTests.setSelectedViewId(null);
    window.__workspaceStoreForTests.setSelectedViewId(id);
  }, v.id);
  await page.waitForTimeout(700);
  const cur = await page.evaluate(() => window.__workspaceStoreForTests.getSelectedViewId?.() ?? null);
  if (cur === v.id) break;
}
await page.waitForSelector('[data-testid="svg-canvas"]', { timeout: 8000 });
await page.waitForTimeout(1500);

const dump = await page.evaluate((id) => {
  const d = window.__diagramGeometryForTests;
  if (!d || d.viewId !== id) return null;
  return d;
}, v.id);

if (!dump) {
  console.error('no geometry dump');
  process.exit(1);
}

// Only nodes whose parent is the scene root (parentId null/root) — the top-level
// layout that drives the overall aspect.
const roots = dump.nodes.filter((n) => !n.parentId || n.parentId === 'root');
console.log(`fit.contentBox: ${JSON.stringify(dump.fit.contentBox)}`);
console.log(`fit.viewport:   ${JSON.stringify(dump.fit.viewport)}  scale=${dump.fit.scale?.toFixed(3)}`);
console.log(`aspect: content ${(dump.fit.contentBox.width / dump.fit.contentBox.height).toFixed(2)} vs viewport ${(dump.fit.viewport.width / dump.fit.viewport.height).toFixed(2)}`);
console.log(`total nodes: ${dump.nodes.length}; top-level: ${roots.length}`);
console.log('── top-level nodes (x, y, w, h, container) sorted by x ──');
for (const n of roots.sort((a, b) => a.rect.x - b.rect.x)) {
  const r = n.rect;
  console.log(
    `  ${(n.id ?? '?').slice(0, 20).padEnd(20)} x=${r.x.toFixed(0).padStart(6)} y=${r.y.toFixed(0).padStart(6)} w=${r.width.toFixed(0).padStart(5)} h=${r.height.toFixed(0).padStart(5)} ${n.container ? 'CONTAINER' : ''}`,
  );
}

await browser.close();
