/**
 * ninebar Compare — Phase 6 diff canvas smoke (`--project=ninebar`).
 *
 * Offline: every backend call is stubbed at the route layer, with
 * `/api/command` dispatched per command so the surface exercises the
 * real data plumbing (sessions.list → picks → diff_timeline +
 * decimated series → canvas/banners/markers/fork affordances).
 *
 * Covers the Phase 6 mandates end-to-end in the browser:
 *   - /compare deep-link redirects under the Simulate door and the
 *     Run tab lights (demotion, W1);
 *   - one shared playhead, history_truncated banner, fork-anchor +
 *     first-divergence markers (W3);
 *   - fork affordance exists ONLY at forkable_ticks (W4);
 *   - mode switch incl. the golden picker + history-browser modal
 *     opener (W5/W6).
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

const SESSION_A = {
  id: 'aaaaaaaa-0000-0000-0000-000000000000',
  kind: 'orchestrator',
  uri: '__workspace__',
  subsystem_name: null,
  label: 'baseline',
  created_at_ms: 0,
  elapsed_ms: 1000,
  tick: 4,
  time_ms: 4,
  current_state: null,
  completed: true,
  is_expired: false,
  history_len: 5,
  subsystem_count: 1,
  fork_point_tick: null,
  forkable_ticks: [0, 4],
  paused: false,
  ticks_advanced: 0,
};
const SESSION_B = {
  ...SESSION_A,
  id: 'bbbbbbbb-0000-0000-0000-000000000000',
  label: 'branch',
  fork_point_tick: 2,
  forkable_ticks: [0, 2, 4],
};

function seriesFor(sessionId: string) {
  // A: v = t. B: diverges from tick 2 (v = t + 1).
  const isB = sessionId === SESSION_B.id;
  return {
    var: 'current',
    points: [0, 1, 2, 3, 4].map((t) => ({
      time_ms: t,
      value: isB && t >= 2 ? t + 1 : t,
    })),
  };
}

const TIMELINE_DIFF = {
  a_id: SESSION_A.id,
  b_id: SESSION_B.id,
  shared_start_tick: 0,
  shared_end_tick: 4,
  first_divergence_tick: 2,
  tick_diffs: [2, 3, 4].map((tick) => ({
    tick,
    subsystem_diffs: [],
    variable_diffs: [{ name: 'current', a_value: tick, b_value: tick + 1 }],
  })),
  history_truncated: true,
};

const GOLDEN_ENTRY = {
  id: 'golden-arch-1',
  label: 'nightly reference',
  origin: 'run',
  workspace_uri: '__workspace__',
  created_at: 1,
  ended_at: 2,
  ticks: 5,
  is_golden: true,
  golden_label: 'v1.0',
};

async function stubBackend(page: Page) {
  await page.route('**/health', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{"status":"ok"}' }),
  );
  for (const path of ['sessions', 'workspace', 'sources', 'models', 'files']) {
    await page.route(`http://localhost:3010/${path}**`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: path === 'workspace' ? '{"uris":[]}' : path === 'models' ? '{}' : '[]',
      }),
    );
  }
  await page.route('http://localhost:3010/api/**', (route) => {
    const req = route.request();
    let command = '';
    let params: Record<string, unknown> = {};
    try {
      const body = req.postDataJSON() as { command?: string; params?: Record<string, unknown> };
      command = body?.command ?? '';
      params = body?.params ?? {};
    } catch {
      // GET /api/* — fall through to the default stub.
    }
    const json = (payload: unknown) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(payload),
      });
    switch (command) {
      case 'sysml.sessions.list':
        return json([SESSION_A, SESSION_B]);
      case 'sysml.sessions.diff_timeline':
        return json(TIMELINE_DIFF);
      case 'sysml.sessions.timeseries_names':
        return json({ names: ['current'], len: 5, capacity: 1024 });
      case 'sysml.sessions.timeseries_decimated':
        return json(seriesFor(String(params.session_id ?? '')));
      case 'sysml.sessions.archive.list':
        return json({ entries: [GOLDEN_ENTRY] });
      case 'sysml.sessions.archive.get':
        return json({
          entry: {
            ...GOLDEN_ENTRY,
            overrides: {},
            verdicts: [],
            snapshots: [0, 1, 2, 3, 4].map((t) => ({
              tick: t,
              variables: { current: t },
            })),
          },
        });
      case 'sysml.sessions.quota':
        return json({});
      default:
        return json([]);
    }
  });
}

async function gotoCompare(page: Page) {
  await stubBackend(page);
  // Deliberately the LEGACY path — the redirect is part of the contract.
  await page.goto(`${APP}/compare?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
  await expect(page.getByTestId('compare-workflow-ninebar')).toBeVisible();
}

async function pickBoth(page: Page) {
  await page
    .getByTestId(`compare-session-rail-row-${SESSION_A.id}`)
    .locator('input[type=checkbox]')
    .check();
  await page
    .getByTestId(`compare-session-rail-row-${SESSION_B.id}`)
    .locator('input[type=checkbox]')
    .check();
  await expect(page.getByTestId('compare-diff-canvas')).toBeVisible();
}

test('demotion: /compare redirects under the Simulate door and lights the Run tab', async ({ page }) => {
  await gotoCompare(page);
  await expect(page).toHaveURL(/\/run\/compare/);
  await expect(page.getByTestId('tool-tab-session')).toHaveAttribute('data-active', 'true');
  // No Compare tab in the nav anymore.
  await expect(page.getByTestId('tool-tab-compare')).toHaveCount(0);
  // Teaching state until 2 sessions are picked.
  await expect(page.getByTestId('compare-teaching')).toBeVisible();
});

test('pair diff: canvas + envelope + exact gutter + honesty banner + markers', async ({ page }) => {
  await gotoCompare(page);
  await pickBoth(page);

  await expect(page.getByTestId('compare-variable-row-current')).toBeVisible();
  await expect(page.getByTestId('compare-envelope-current')).toBeAttached();
  await expect(page.getByTestId('compare-gutter-current')).toBeAttached();

  // Contract mandates: the truncation banner + both playhead markers.
  await expect(page.getByTestId('compare-history-truncated')).toBeVisible();
  await expect(page.getByTestId('compare-marker-fork-anchor')).toBeAttached();
  await expect(page.getByTestId('compare-marker-first-divergence')).toBeAttached();

  // The fork anchor SNAPPED the shared playhead to fork_point_tick (2).
  await expect(page.getByTestId('compare-playhead-tick')).toHaveText(/tick 2 \/ 4/);

  // One playhead drives the value chips: at tick 2 the pair reads 2 vs 3.
  await expect(page.getByTestId(`compare-value-current-${SESSION_A.id}`)).toContainText('2.000');
  await expect(page.getByTestId(`compare-value-current-${SESSION_B.id}`)).toContainText('3.000');
});

test('fork affordance exists only at forkable_ticks (F8)', async ({ page }) => {
  await gotoCompare(page);
  await pickBoth(page);

  // Playhead sits at the fork anchor (tick 2) — forkable for B only.
  await expect(page.getByTestId(`compare-fork-here-${SESSION_B.id}`)).toBeVisible();
  await expect(page.getByTestId(`compare-fork-here-${SESSION_A.id}`)).toHaveCount(0);

  // Step to tick 3 — archived nowhere; every affordance disappears.
  await page.getByTestId('compare-playhead-step-forward').click();
  await expect(page.getByTestId(`compare-fork-here-${SESSION_B.id}`)).toHaveCount(0);
  await expect(page.getByTestId(`compare-fork-here-${SESSION_A.id}`)).toHaveCount(0);
});

test('mode switch: golden picker lists pinned runs and opens the history browser', async ({ page }) => {
  await gotoCompare(page);
  await pickBoth(page);

  await page.getByTestId('compare-mode-tab-golden').click();
  const picker = page.getByTestId('compare-golden-picker');
  await expect(picker).toBeVisible();
  await picker.selectOption(GOLDEN_ENTRY.id);
  // Golden series equals A exactly → pass; B diverges +1 → fail.
  await expect(
    page.getByTestId(`compare-golden-verdict-current-${SESSION_A.id}`),
  ).toHaveText('pass');
  await expect(
    page.getByTestId(`compare-golden-verdict-current-${SESSION_B.id}`),
  ).toHaveText('fail');

  // "manage archive…" opens the history-browser modal (plan row 24).
  await page.getByTestId('compare-golden-manage').click();
  await expect(page.getByTestId('history-browser-modal')).toBeVisible();
  await expect(page.getByTestId('history-browser-archived-runs')).toBeVisible();
});
