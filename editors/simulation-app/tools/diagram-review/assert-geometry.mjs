/**
 * Geometry gates for the diagram-review harness (layout-quality brief §6).
 *
 * Loads every (example × declared view) exactly as shoot.mjs does, pulls the
 * `window.__diagramGeometryForTests` dump published by SvgCanvas after layout
 * + label placement, and asserts the G1–G7 gates:
 *
 *   G1  no edge label overlaps a node body (frame boxes count as nodes)
 *   G2  no label-label overlap; degraded count = 0
 *   G3  port labels disjoint from node bodies / other port labels / edge labels
 *   G4  ported edges anchor on port glyph centers (±1px) — incl. after a
 *       scripted drag on one tile (probe2 drag machinery)
 *   G5  content bbox aspect within 2.5× of the viewport aspect (no ribbon)
 *   G6  fit floor: all tiles ≥ 0.25; corePhysics ≥ 0.5 (StartFlowView exempt
 *       from the 0.5 floor until D-B1 — not exempt from G5)
 *   G7  content that fits both axes is centered within 8px of viewport center
 *
 * (G8 is the vitest suite — not this script's job.)
 *
 * Failures are REPORTED per (tile, gate) but do not fail the run unless
 * `--strict` is passed (brief §7 step 0: gates wired non-blocking first; step 6
 * flips them). Servers: reuses a running api+vite pair when one is up (the
 * `run.sh --assert` path shares run.sh's servers in one shell invocation);
 * otherwise spawns its own, so `node assert-geometry.mjs` works standalone.
 *
 * Note on G1 and container bodies: a container's body area is legitimate
 * routing space (a transition label INSIDE its state-machine container is
 * correct, not an overlap), so G1/G3 test against non-container node rects
 * plus the frame heading/corner boxes. Container HEADERS are covered by the
 * child-node rects elk keeps out of the padding band.
 */
import { writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { chromium } from 'playwright-core';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '..', '..', '..', '..');
const APP_DIR = join(HERE, '..', '..');
const APP = 'http://127.0.0.1:3010';
const API = 'http://127.0.0.1:8080';
const STRICT = process.argv.includes('--strict');
const OUT =
  process.argv.slice(2).find((a) => !a.startsWith('--')) ??
  join(APP_DIR, 'test-results', 'diagram-review');

const EXAMPLES = [
  'examples/view-showcase',
  'examples/espresso-production-cell',
  'examples/espresso-pump-hybrid',
];

// ── server management ────────────────────────────────────────────────
const reachable = async (url) => {
  try {
    await fetch(url, { signal: AbortSignal.timeout(1000) });
    return true;
  } catch {
    return false;
  }
};
const spawned = [];
if (!(await reachable(APP))) {
  console.log('(servers not up — spawning api + vite for this run)');
  spawned.push(spawn(join(REPO, 'target', 'release', 'sysml-api'), ['127.0.0.1:8080'], { stdio: 'ignore' }));
  spawned.push(
    spawn(join(APP_DIR, 'node_modules', '.bin', 'vite'), ['--port', '3010', '--strictPort'], {
      cwd: APP_DIR,
      stdio: 'ignore',
    }),
  );
  let ok = false;
  for (let i = 0; i < 30 && !ok; i++) {
    ok = await reachable(APP);
    if (!ok) await new Promise((r) => setTimeout(r, 1000));
  }
  if (!ok) {
    for (const p of spawned) p.kill();
    throw new Error('vite did not come up on :3010');
  }
}
const shutdown = () => {
  for (const p of spawned) p.kill();
};
process.on('exit', shutdown);

// ── geometry helpers ─────────────────────────────────────────────────
const EPS = 0.5; // sub-pixel touches don't count as overlap
const overlaps = (a, b) =>
  a.x + EPS < b.x + b.width && b.x + EPS < a.x + a.width && a.y + EPS < b.y + b.height && b.y + EPS < a.y + a.height;

/** Run G1–G7 over one tile's dump. Returns { gates: {G1: {pass, detail}, …} }. */
function runGates(dump, tileName) {
  const gates = {};
  const bodyRects = dump.nodes.filter((n) => !n.container).map((n) => n.rect);
  const labelRects = dump.labels.map((l) => l.rect);
  const portLabelRects = dump.ports.filter((p) => p.labelRect).map((p) => p.labelRect);

  // G1 — edge labels vs node bodies.
  let g1 = 0;
  for (const l of dump.labels) for (const r of bodyRects) if (overlaps(l.rect, r)) g1++;
  gates.G1 = { pass: g1 === 0, detail: `${g1} label∩node` };

  // G2 — label-label + degraded count.
  let g2 = 0;
  for (let i = 0; i < labelRects.length; i++)
    for (let j = i + 1; j < labelRects.length; j++) if (overlaps(labelRects[i], labelRects[j])) g2++;
  const degraded = dump.labels.filter((l) => l.degraded).length;
  gates.G2 = { pass: g2 === 0 && degraded === 0, detail: `${g2} label∩label, ${degraded} degraded` };

  // G3 — port labels vs node bodies / other port labels / edge labels.
  let g3 = 0;
  for (let i = 0; i < portLabelRects.length; i++) {
    for (const r of bodyRects) if (overlaps(portLabelRects[i], r)) g3++;
    for (let j = i + 1; j < portLabelRects.length; j++) if (overlaps(portLabelRects[i], portLabelRects[j])) g3++;
    for (const l of labelRects) if (overlaps(portLabelRects[i], l)) g3++;
  }
  gates.G3 = { pass: g3 === 0, detail: `${g3} port-label clashes (${portLabelRects.length} labels)` };

  // G4 — ported edges anchored on port centers.
  const ported = dump.edges.filter((e) => e.portAnchored !== null);
  const unanchored = ported.filter((e) => e.portAnchored === false).length;
  gates.G4 = { pass: unanchored === 0, detail: `${unanchored}/${ported.length} ported edges off-center` };

  // G8 — edge-route detour ratio. An orthogonal route is expected to be longer
  // than the straight-line (Manhattan) distance between its endpoints, but a
  // route that is MANY times longer means elk sent the edge around the graph
  // instead of between its endpoints — the "down, all the way up and over, back
  // down" shape. That is what root-level layer wrapping used to produce:
  // MixedExposeView's two Engine→Gearbox edges measured 1126px of route for a
  // 270px gap (4.17×) before the fix, and 1.00×/1.29× after.
  //
  // Threshold 3.0× is calibrated from real measurements: straight runs sit at
  // 1.0–1.3×, and legitimate state-machine back-transitions (which must route
  // around their own source node) top out near 1.9×. Short edges are skipped —
  // a few px of jog around a port dominates the ratio and says nothing.
  //
  // A ratio ALONE is not enough (D-L9). Two nodes 78px apart joined by a pair
  // of parallel transitions force elk to offset the second one around the
  // first: 251px of route, 3.22× — flagged, but only 173px longer than direct
  // and visually a small jog. Meanwhile the real defect this gate exists for
  // was 856px longer than direct. So a route must be BOTH disproportionate and
  // absolutely long to count. `MIN_EXCESS` is the floor that separates them.
  const MIN_DIRECT = 60;
  const MAX_DETOUR = 3.0;
  const MIN_EXCESS = 250;
  const detours = [];
  for (const e of dump.edges) {
    const p = e.points ?? [];
    if (p.length < 2) continue;
    let len = 0;
    for (let i = 1; i < p.length; i++) len += Math.abs(p[i].x - p[i - 1].x) + Math.abs(p[i].y - p[i - 1].y);
    const a0 = p[0];
    const z0 = p[p.length - 1];
    const direct = Math.abs(z0.x - a0.x) + Math.abs(z0.y - a0.y);
    if (direct < MIN_DIRECT) continue;
    const r = len / direct;
    if (r > MAX_DETOUR && len - direct > MIN_EXCESS)
      detours.push(`${e.id.slice(0, 8)}@${r.toFixed(2)}×(+${Math.round(len - direct)}px)`);
  }
  gates.G8 = {
    pass: detours.length === 0,
    detail: detours.length
      ? `${detours.length} pathological route(s): ${detours.join(', ')}`
      : `no route over ${MAX_DETOUR}× and +${MIN_EXCESS}px`,
  };

  // G5 / G6 / G7 — fit geometry.
  const fit = dump.fit;
  if (!fit) {
    gates.G5 = gates.G6 = gates.G7 = { pass: false, detail: 'no fit recorded' };
    return { gates };
  }
  const a = fit.contentBox.width / fit.contentBox.height;
  const v = fit.viewport.width / fit.viewport.height;
  const ratio = Math.max(a / v, v / a);
  gates.G5 = { pass: ratio <= 2.5, detail: `aspect ${ratio.toFixed(2)}× viewport` };

  const isCore = tileName.includes('corePhysics');
  const isStartFlow = tileName.includes('StartFlowView');
  const floor = isCore && !isStartFlow ? 0.5 : 0.25;
  gates.G6 = { pass: fit.scale >= floor, detail: `fit ${(fit.scale * 100).toFixed(0)}% (floor ${floor * 100}%)` };

  const fitsBoth =
    fit.contentBox.width * fit.scale <= fit.viewport.width - 1 &&
    fit.contentBox.height * fit.scale <= fit.viewport.height - 1;
  if (fitsBoth) {
    const cx = fit.tx + (fit.contentBox.x + fit.contentBox.width / 2) * fit.scale;
    const cy = fit.ty + (fit.contentBox.y + fit.contentBox.height / 2) * fit.scale;
    const dx = Math.abs(cx - fit.viewport.width / 2);
    const dy = Math.abs(cy - fit.viewport.height / 2);
    gates.G7 = { pass: dx <= 8 && dy <= 8, detail: `center off by ${dx.toFixed(1)},${dy.toFixed(1)}px` };
  } else {
    gates.G7 = { pass: true, detail: 'overflows viewport (n/a)' };
  }
  return { gates };
}

// ── harness (mirrors shoot.mjs) ──────────────────────────────────────
async function command(name, params) {
  const res = await fetch(`${API}/api/command`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command: name, params }),
  });
  if (!res.ok) throw new Error(`${name} → HTTP ${res.status}: ${await res.text()}`);
  return res.json();
}

async function listViews(root) {
  const result = await command('sysml.query', {
    uri: '__workspace__',
    spec: {
      filter: { type: 'view', viewpoint_id: null },
      projection: 'summary_expand',
      sort: [{ field: 'name', dir: 'asc' }],
      limit: 1000,
    },
  });
  return (result.rows ?? []).filter((r) => {
    const file = (r.source_span?.file ?? '').replace(/^file:\/\//, '');
    return !file.includes('/libraries/') && file.startsWith(root);
  });
}

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });

/** Poll the dump until it reports the requested viewId (or timeout). */
async function readDump(viewId, timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const dump = await page.evaluate(() => window.__diagramGeometryForTests ?? null);
    if (dump && dump.viewId === viewId && dump.fit) return dump;
    await page.waitForTimeout(300);
  }
  return null;
}

const report = [];
for (const example of EXAMPLES) {
  const root = join(REPO, example);
  console.log(`── ${example}`);
  await page.goto(`${APP}/run?workspace=${encodeURIComponent(root)}`);
  await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, { timeout: 30000 });
  let views = [];
  for (let i = 0; i < 30; i++) {
    try {
      views = await listViews(root);
      if (views.length > 0) break;
    } catch {
      /* workspace still loading */
    }
    await page.waitForTimeout(1000);
  }
  for (const v of views) {
    const name = v.name ?? v.id;
    const tile = `${example.split('/').pop()}__${name.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
    for (let attempt = 0; attempt < 4; attempt++) {
      await page.evaluate((id) => {
        window.__workspaceStoreForTests.setSelectedViewId(null);
        window.__workspaceStoreForTests.setSelectedViewId(id);
      }, v.id);
      await page.waitForTimeout(700);
      const cur = await page.evaluate(() => window.__workspaceStoreForTests.getSelectedViewId?.() ?? null);
      if (cur === v.id) break;
    }
    // Graph views publish the dump; non-graph families are out of scope.
    const isGraph = await page
      .waitForSelector('[data-testid="svg-canvas"]', { timeout: 8000 })
      .then(() => true)
      .catch(() => false);
    if (!isGraph) {
      report.push({ tile, family: 'non-graph', gates: null });
      console.log(`   ${name} → (non-graph, skipped)`);
      continue;
    }
    await page.waitForTimeout(1500); // layout + fit settle
    const dump = await readDump(v.id);
    if (!dump) {
      report.push({ tile, family: 'graph', gates: null, error: 'no geometry dump' });
      console.log(`   ${name} → ✗ no geometry dump`);
      continue;
    }
    const { gates } = runGates(dump, tile);
    report.push({
      tile,
      family: 'graph',
      gates,
      metrics: {
        fitScale: dump.fit?.scale ?? null,
        nodes: dump.nodes.length,
        labels: dump.labels.length,
        degraded: dump.labels.filter((l) => l.degraded).length,
      },
    });
    const line = Object.entries(gates)
      // Sort by gate number so display order is G1..G8 regardless of the order
      // the gates are computed in (G8 runs before the G5-G7 fit block so it is
      // still reported when no fit was recorded).
      .sort(([a], [b]) => Number(a.slice(1)) - Number(b.slice(1)))
      .map(([g, r]) => `${g}${r.pass ? '✓' : `✗(${r.detail})`}`)
      .join(' ');
    console.log(`   ${name} → ${line}`);
  }
}

// ── G4 after a scripted drag (probe2 machinery, one tile: OverviewView) ──
// Best-effort — a failure here must NOT abort the run before the summary +
// geometry-report.json are written.
try {
  const root = join(REPO, 'examples/view-showcase');
  await page.goto(`${APP}/run?workspace=${encodeURIComponent(root)}`);
  await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, { timeout: 30000 });
  let hit = null;
  for (let i = 0; i < 30 && !hit; i++) {
    try {
      hit = (await listViews(root)).find((r) => r.name === 'OverviewView') ?? null;
    } catch {
      /* loading */
    }
    if (!hit) await page.waitForTimeout(1000);
  }
  if (hit) {
    // Set-and-verify (workspace load can race the selection clear).
    for (let attempt = 0; attempt < 4; attempt++) {
      await page.evaluate((id) => {
        window.__workspaceStoreForTests.setSelectedViewId(null);
        window.__workspaceStoreForTests.setSelectedViewId(id);
      }, hit.id);
      await page.waitForTimeout(700);
      const cur = await page.evaluate(() => window.__workspaceStoreForTests.getSelectedViewId?.() ?? null);
      if (cur === hit.id) break;
    }
    const ok = await page
      .waitForSelector('[data-testid="svg-canvas"]', { timeout: 15000 })
      .then(() => true)
      .catch(() => false);
    if (ok) {
      await page.waitForTimeout(1500);
      const engine = page
        .locator('[data-testid="svg-canvas"] g[data-element-id]', { hasText: 'engine : Engine' })
        .last();
      const b0 = await engine.boundingBox().catch(() => null);
      if (b0) {
        await page.mouse.move(b0.x + b0.width / 2, b0.y + 12);
        await page.mouse.down();
        await page.mouse.move(b0.x + b0.width / 2 + 150, b0.y + 102, { steps: 10 });
        await page.mouse.up();
        await page.waitForTimeout(800);
        const dump = await page.evaluate(() => window.__diagramGeometryForTests ?? null);
        const ported = (dump?.edges ?? []).filter((e) => e.portAnchored !== null);
        const off = ported.filter((e) => e.portAnchored === false).length;
        const gates = {
          G4: { pass: off === 0, detail: `${off}/${ported.length} ported edges off-center after drag` },
        };
        report.push({ tile: 'view-showcase__OverviewView (post-drag)', family: 'graph', gates });
        console.log(`── post-drag G4 → ${gates.G4.pass ? '✓' : `✗ (${gates.G4.detail})`}`);
      }
    }
  }
} catch (e) {
  console.log(`── post-drag G4 → (skipped: ${String(e).slice(0, 80)})`);
}

await browser.close();
shutdown();

// ── summary ──────────────────────────────────────────────────────────
const graphTiles = report.filter((r) => r.gates);
const failures = [];
for (const r of graphTiles)
  for (const [g, res] of Object.entries(r.gates)) if (!res.pass) failures.push(`${r.tile} ${g}: ${res.detail}`);
console.log(`\n${graphTiles.length} graph tiles gated; ${failures.length} (tile, gate) failures`);
for (const f of failures) console.log(`  ✗ ${f}`);
writeFileSync(join(OUT, 'geometry-report.json'), JSON.stringify(report, null, 2));
console.log(`geometry-report.json → ${OUT}`);

if (STRICT && failures.length > 0) process.exit(1);
