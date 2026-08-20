/**
 * ninebar Run, re-composed — /run route smoke test (Phase 3).
 *
 * Runs only under the `ninebar` Playwright project (see
 * `playwright.config.ts`; matched by the `ninebar*.spec.ts` glob,
 * excluded from `default`) — non-blocking, same as
 * `tests/ninebar-shell.spec.ts` / `tests/ninebar-browse.spec.ts`. Run
 * explicitly: `npx playwright test --project=ninebar`.
 *
 * Follows the offline-stub pattern from the other ninebar specs: every
 * backend call is intercepted so the spec runs without a live
 * `sysml-api` process — no workspace is loaded and no session is ever
 * created. This asserts the Phase 3 five-slot contract for Run directly
 * (plan `ninebar-implementation-plan.md` §3 DoD): the left rail hosts
 * the session tree, the primary surface is the diagram (no inline
 * 300px tree column — the "no double-rail collision" contract, audit
 * F17), the bottom strip is OPEN with the waveform card (not a tabbed
 * results workbench), and `ResultsWorkbench`/`SessionStatusBar` are not
 * mounted at all under the flag.
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

async function stubBackend(page: Page) {
  await page.route('**/health', (route) => {
    if (route.request().url().includes('localhost')) {
      return route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' });
    }
    return route.continue();
  });
  await page.route('http://localhost:3010/sessions**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/workspace**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"uris":[]}' }),
  );
  await page.route('http://localhost:3010/sources**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/models**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }),
  );
  await page.route('http://localhost:3010/files**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
  await page.route('http://localhost:3010/api/**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  );
}

async function gotoRun(page: Page) {
  await stubBackend(page);
  await page.goto(`${APP}/run?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
}

test('left rail hosts the portaled session tree, not an inline column', async ({ page }) => {
  await gotoRun(page);

  const leftRail = page.getByTestId('left-rail');
  await expect(leftRail).toBeVisible();

  // SessionTreeV2 is portaled into the shell's left-rail slot (see
  // src/app/slots.tsx) — attached under `left-rail`, not rendered by
  // RunWorkflow's own DOM subtree as an inline 300px column any more.
  // `session-tree-v2` legitimately appears twice within the tree
  // (SessionTreeV2's own container + the nested ModelTreeView root
  // that reuses the same testIdPrefix) — assert the outer one is
  // inside the rail, not that the testid is globally unique.
  const treeInRail = leftRail.getByTestId('session-tree-v2').first();
  await expect(treeInRail).toBeVisible();

  // Mounting `<LeftRailContent>` replaces the interim WorkspaceBar
  // section (AppShell.tsx).
  await expect(page.getByTestId('workspace-bar')).toHaveCount(0);
});

test('primary surface is the diagram, full-bleed with no side column', async ({ page }) => {
  await gotoRun(page);

  const primary = page.getByTestId('primary-outlet');
  await expect(primary).toBeVisible();
  await expect(page.getByTestId('session-workspace')).toBeVisible();

  // The diagram surface renders inside the primary outlet. With no
  // table/geometry/tree payload and no view selected, DiagramHost mounts the
  // first-class view-less landing (`viewless-state`, Phase 2 W5) — offline
  // with no workspace loaded it shows the "load a workspace" state. Either
  // way it's the diagram surface, not a table/tree/geometry renderer or a
  // second tree column.
  await expect(primary.getByTestId('viewless-state')).toBeVisible();

  // No inline 300px tree column beside the diagram (F17 double-rail
  // guard) — the only `session-tree-v2` in the document lives in the
  // left rail, not inside the primary outlet.
  await expect(primary.getByTestId('session-tree-v2')).toHaveCount(0);
});

test('bottom strip is open with the waveform card, no tabbed workbench', async ({ page }) => {
  await gotoRun(page);

  const strip = page.getByTestId('bottom-strip');
  await expect(strip).toHaveAttribute('data-state', 'open');

  await expect(strip.getByTestId('waveform-card')).toBeVisible();

  // No tabbed results workbench remains flag-on (plan §3 DoD) — the
  // testid must not be attached anywhere on the page, not merely
  // hidden.
  await expect(page.getByTestId('results-workbench')).toHaveCount(0);

  // SessionStatusBar is not mounted flag-on either — its tick/time
  // readout moved into the waveform card's own footer (see
  // src/features/results/WaveformCard.tsx doc comment).
  await expect(page.getByTestId('status-bar')).toHaveCount(0);
});
