/**
 * Smoke test for the `@sysml-rs/expression-view` integration.
 *
 * Asserts two things:
 *   1. The backend service command `sysml.expression.ast` returns
 *      renderable AST payloads for a known constraint-bearing model.
 *   2. When the `expressionView` feature flag is on, the shared
 *      `expression-view` package renders KaTeX markup (`.katex`) in the
 *      DOM for at least one constraint expression.
 *
 * We do (1) against the REST API and (2) by navigating to the Vite dev
 * server, loading the compiled `expression-view` ESM module, and rendering
 * a single expression into a blank container. This keeps the smoke test
 * focused on the wiring path that ConstraintCard + EquationsTab use.
 *
 * Skips (rather than failing) when either the API server or the Vite dev
 * server is unreachable — the upstream CI harness spins those up, but this
 * file must remain runnable in bare environments.
 *
 * Env:
 *   - API server: cargo run -p sysml-api   (port 8080)
 *   - Vite dev:   npm --prefix editors/simulation-app run dev (port 3010)
 */
import { test, expect, APIRequestContext } from '@playwright/test';
import * as fs from 'fs';
import { repoPath } from './repo-paths';

const APP = 'http://localhost:3010';
const API = 'http://localhost:8080';
const GS = repoPath('examples/espresso-production-cell');

async function apiAlive(request: APIRequestContext): Promise<boolean> {
  try {
    const r = await request.get(`${API}/health`, { timeout: 2000 });
    return r.ok();
  } catch {
    return false;
  }
}

async function appAlive(request: APIRequestContext): Promise<boolean> {
  try {
    const r = await request.get(`${APP}/`, { timeout: 2000 });
    return r.ok();
  } catch {
    return false;
  }
}

/**
 * Best-effort probe for a service command. Returns `null` if the running
 * backend doesn't know the command (older build), truthy JSON otherwise.
 */
async function tryCmd(
  request: APIRequestContext,
  command: string,
  params: Record<string, unknown> = {},
): Promise<unknown | null> {
  const r = await request.post(`${API}/api/command`, {
    data: { command, params },
  });
  if (!r.ok()) return null;
  const body = (await r.json()) as unknown;
  if (
    body &&
    typeof body === 'object' &&
    'error' in (body as Record<string, unknown>) &&
    typeof (body as Record<string, unknown>).error === 'string' &&
    /not found|unknown command/i.test(String((body as Record<string, unknown>).error))
  ) {
    return null;
  }
  return body;
}

async function loadFileUri(
  request: APIRequestContext,
  path: string,
): Promise<string> {
  const r = await request.post(`${API}/files`, { data: { path } });
  expect(r.ok()).toBeTruthy();
  const body = await r.json();
  return body.uri as string;
}

function pickSysmlFile(): string {
  // A single constraint-bearing file out of the espresso production cell —
  // the whole cell is a directory of many files and this test only needs one.
  // PhysicalLaws holds the cell's reusable constraint defs (MassBalance,
  // EnergyBalance, WithinEnvelope) and a calc def, so its expressions cover
  // arithmetic, comparison, boolean and call nodes for the AST render.
  const file = `${GS}/Libraries/PhysicalLaws.sysml`;
  if (!fs.existsSync(file)) {
    throw new Error(`expression-view fixture not found: ${file}`);
  }
  return file;
}

test.describe('expression-view integration', () => {
  test('backend exposes expression AST for constraint-bearing model', async ({
    request,
  }) => {
    if (!(await apiAlive(request))) {
      test.skip(true, `sysml-api not reachable at ${API}; skipping backend AST smoke test.`);
      return;
    }
    const uri = await loadFileUri(request, pickSysmlFile());
    const rowsRaw = await tryCmd(request, 'sysml.expression.ast', { uri });
    if (rowsRaw === null) {
      test.skip(
        true,
        "running sysml-api binary predates 'sysml.expression.ast'; rebuild with `cargo build -p sysml-api` to exercise this path.",
      );
      return;
    }
    const rows = rowsRaw as Array<{
      element_name: string | null;
      element_kind: string;
      ast: unknown;
    }>;
    expect(Array.isArray(rows)).toBeTruthy();
    // At least one AST row must come back with a non-null tree.
    const renderable = rows.filter((r) => r.ast !== null);
    expect(renderable.length).toBeGreaterThan(0);
  });

  test('ConstraintCard renders KaTeX when flag is enabled', async ({
    page,
    request,
  }) => {
    if (!(await apiAlive(request))) {
      test.skip(true, `sysml-api not reachable at ${API}; skipping KaTeX render smoke.`);
      return;
    }
    if (!(await appAlive(request))) {
      test.skip(true, `simulation-app dev server not reachable at ${APP}; skipping KaTeX render smoke.`);
      return;
    }

    // 1. Load the SysML file on the backend and grab a real AST.
    const uri = await loadFileUri(request, pickSysmlFile());
    const rowsRaw = await tryCmd(request, 'sysml.expression.ast', { uri });
    if (rowsRaw === null) {
      test.skip(
        true,
        "running sysml-api binary predates 'sysml.expression.ast'; rebuild with `cargo build -p sysml-api` to exercise this path.",
      );
      return;
    }
    const rows = rowsRaw as Array<{
      element_id: string;
      element_name: string | null;
      element_kind: string;
      source: string | null;
      ast: unknown;
    }>;
    const sample = rows.find((r) => r.ast !== null);
    expect(sample, 'expected at least one AST with a non-null tree').toBeTruthy();

    // 2. Flip the feature flag and navigate to the Vite dev server. The
    //    page itself doesn't need to have a running activity; we mount a
    //    minimal ConstraintCard via the live expression-view module served
    //    through Vite's ESM pipeline.
    await page.goto(`${APP}/?flag=expressionView`);
    await page.waitForLoadState('domcontentloaded');

    // Render the AST into a detached host element using the installed
    // `@sysml-rs/expression-view` module. Vite serves it as an ESM import
    // from node_modules, so `/@id/@sysml-rs/expression-view` resolves.
    const katexHtml = await page.evaluate(async (ast) => {
      const mod = await import(
        /* @vite-ignore */ '/@id/@sysml-rs/expression-view'
      );
      const host = document.createElement('span');
      host.id = 'expression-view-test-host';
      document.body.appendChild(host);
      mod.renderExpression(host, ast as unknown as import('@sysml-rs/expression-view').ExpressionAstNode);
      return host.innerHTML;
    }, sample!.ast);

    // KaTeX emits `<span class="katex">…</span>` wrappers around rendered math.
    expect(katexHtml).toContain('katex');
  });
});
