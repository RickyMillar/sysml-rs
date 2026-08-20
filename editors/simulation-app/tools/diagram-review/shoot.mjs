/**
 * Diagram-review contact-sheet harness (Phase 1 of
 *
 * For every (example × declared view): load the workspace on /run, select
 * the view via the documented Playwright hook
 * (`__workspaceStoreForTests.setSelectedViewId`), wait for the render
 * family to settle, screenshot the primary surface. Emits PNGs + a
 * manifest.json + gallery.html into the output dir.
 *
 * Views are enumerated live via `sysml.query` (never a hardcoded list),
 * mirroring `useViewsList`: filter type=view, drop stdlib views
 * (source file under `/libraries/`).
 *
 * Run via run.sh — api + vite + this script must share one shell
 * invocation (sandbox network namespaces are per-command).
 */
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, '..', '..', '..', '..');
const APP = 'http://127.0.0.1:3010';
const API = 'http://127.0.0.1:8080';

const EXAMPLES = [
  'examples/view-showcase',
  'examples/espresso-production-cell',
  'examples/espresso-pump-hybrid',
];

const OUT = process.argv[2] ?? join(HERE, '..', '..', 'test-results', 'diagram-review');
mkdirSync(OUT, { recursive: true });

async function command(name, params) {
  const res = await fetch(`${API}/api/command`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command: name, params }),
  });
  if (!res.ok) throw new Error(`${name} → HTTP ${res.status}: ${await res.text()}`);
  return res.json();
}

/** Mirror useViewsList (all workspace views minus stdlib), narrowed to
 *  views declared under `root` — the workspace switch is async and the
 *  backend can briefly answer with the PREVIOUS example's views. */
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
    // Spans carry file:// URIs, not bare paths.
    const file = (r.source_span?.file ?? '').replace(/^file:\/\//, '');
    return !file.includes('/libraries/') && file.startsWith(root);
  });
}

/** Which render family actually appeared for the selected view. */
const FAMILY_SELECTORS = {
  graph: '[data-testid="svg-canvas"]',
  empty: '[data-testid="svg-canvas-empty"]',
  table: '[data-testid="table-view-root"]',
  tree: '[data-testid="browser-view-root"]',
  geometry: '[data-testid="geometry-view-root"]',
};

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1600, height: 1000 },
  deviceScaleFactor: 2,
});
const consoleErrors = [];
page.on('console', (m) => {
  if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 300));
});

const shots = [];
for (const example of EXAMPLES) {
  const root = join(REPO, example);
  console.log(`── ${example}`);
  await page.goto(`${APP}/run?workspace=${encodeURIComponent(root)}`);
  await page.waitForFunction(() => Boolean(window.__workspaceStoreForTests), null, {
    timeout: 30000,
  });
  // Workspace load is async — poll the backend until this example's views
  // (or at least the merged graph) answer.
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
  if (views.length === 0) {
    console.log('   (no declared views — recorded)');
    shots.push({ example, view: null, family: 'no-views', png: null });
    continue;
  }
  for (const v of views) {
    const name = v.name ?? v.id;
    const errBefore = consoleErrors.length;
    // Set-and-verify: a workspace load completing AFTER the set clears the
    // selection (races the first view of each example) — retry until it sticks.
    for (let attempt = 0; attempt < 4; attempt++) {
      await page.evaluate((id) => {
        window.__workspaceStoreForTests.setSelectedViewId(null);
        window.__workspaceStoreForTests.setSelectedViewId(id);
      }, v.id);
      await page.waitForTimeout(700);
      const cur = await page.evaluate(
        () => window.__workspaceStoreForTests.getSelectedViewId?.() ?? null,
      );
      if (cur === v.id) break;
    }
    // Wait for whichever family renders. `empty` is also the pre-render
    // placeholder, so real families win: only record `empty` if nothing
    // else appeared within the window.
    let family = 'none';
    const start = Date.now();
    while (Date.now() - start < 12000) {
      for (const [fam, sel] of Object.entries(FAMILY_SELECTORS)) {
        if (fam === 'empty') continue;
        if (await page.locator(sel).first().isVisible().catch(() => false)) {
          family = fam;
          break;
        }
      }
      if (family !== 'none') break;
      await page.waitForTimeout(250);
    }
    // Layout settle (elk pass + fit + any stale non-graph payload being
    // replaced) — then RE-detect so the recorded family is what the
    // screenshot actually shows, not the first thing that flashed by.
    await page.waitForTimeout(1500);
    family = 'none';
    for (const [fam, sel] of Object.entries(FAMILY_SELECTORS)) {
      if (fam === 'empty') continue;
      if (await page.locator(sel).first().isVisible().catch(() => false)) {
        family = fam;
        break;
      }
    }
    if (family === 'none') {
      family = (await page
        .locator(FAMILY_SELECTORS.empty)
        .first()
        .isVisible()
        .catch(() => false))
        ? 'empty'
        : 'none';
    }
    const slug = `${example.split('/').pop()}__${name.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
    const png = `${slug}.png`;
    const target = page.locator('[data-testid="primary-outlet"]');
    await target.screenshot({ path: join(OUT, png) });
    shots.push({
      example,
      view: name,
      qualified_name: v.qualified_name ?? null,
      kind: v.kind ?? null,
      id: v.id,
      family,
      png,
      console_errors: consoleErrors.slice(errBefore),
      source_file: v.source_span?.file ?? null,
    });
    console.log(`   ${name} → ${family}`);
  }
}
await browser.close();

writeFileSync(join(OUT, 'manifest.json'), JSON.stringify(shots, null, 2));

// ── gallery.html — the contact sheet ─────────────────────────────────
const byExample = {};
for (const s of shots) (byExample[s.example] ??= []).push(s);
// Images are EMBEDDED as data URIs — the gallery must render standalone
// (sandboxed browsers / side-panel viewers can't always resolve relative
// file paths). Dark diagram PNGs compress well; the whole sheet is ~3MB.
const tile = (s) =>
  s.png && existsSync(join(OUT, s.png))
    ? `<figure class="tile" data-family="${s.family}">
        <img src="data:image/png;base64,${readFileSync(join(OUT, s.png)).toString('base64')}" loading="lazy"
             onclick="this.closest('figure').classList.toggle('zoom')">
        <figcaption><b>${s.view}</b> <span class="fam ${s.family}">${s.family}</span>
        ${s.console_errors?.length ? `<span class="err">⚠ ${s.console_errors.length} console error(s)</span>` : ''}
        <br><small>${s.qualified_name ?? ''} · ${s.kind ?? ''}</small></figcaption>
      </figure>`
    : `<figure class="tile"><figcaption><b>(no declared views)</b></figcaption></figure>`;
const html = `<!-- diagram-review contact sheet — generated by shoot.mjs -->
<meta charset="utf-8"><title>diagram review — contact sheet</title>
<style>
  body{background:#1A140F;color:#EFE8DC;font:13px 'IBM Plex Sans',system-ui,sans-serif;margin:24px}
  h1{font-size:18px} h2{font-size:14px;color:#A99C87;border-bottom:1px solid #3A3126;padding-bottom:4px;margin-top:32px}
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(420px,1fr));gap:16px}
  .tile{margin:0;border:1px solid #3A3126;border-radius:8px;overflow:hidden;background:#201A13}
  .tile img{width:100%;display:block;background:#14100C;cursor:zoom-in}
  .tile.zoom{grid-column:1/-1} .tile.zoom img{cursor:zoom-out}
  figcaption{padding:8px 10px;line-height:1.5}
  .fam{font-family:'IBM Plex Mono',monospace;font-size:10px;border:1px solid #4A4033;border-radius:999px;padding:1px 7px;margin-left:6px}
  .fam.none{color:#C75845;border-color:#C75845} .fam.empty{color:#B0A94E}
  .err{color:#C75845;font-size:11px;margin-left:6px}
  small{color:#8A7D68;font-family:'IBM Plex Mono',monospace}
</style>
<h1>Diagram review — contact sheet <small>(${new Date().toISOString().slice(0, 10)})</small></h1>
${Object.entries(byExample)
  .map(([ex, list]) => `<h2>${ex} — ${list.filter((s) => s.png).length} views</h2>
    <div class="grid">${list.map(tile).join('\n')}</div>`)
  .join('\n')}`;
writeFileSync(join(OUT, 'gallery.html'), html);
console.log(`\n${shots.filter((s) => s.png).length} shots → ${OUT}/gallery.html`);
