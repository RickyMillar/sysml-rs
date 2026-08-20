/**
 * ninebar AppShell — five-slot skeleton smoke test (Phase 1).
 *
 * Runs only under the `ninebar` Playwright project (see
 * `playwright.config.ts`; matched by filename, excluded from `default`)
 * — non-blocking, not part of the legacy CI gate
 * (`.github/workflows/simulation-app.yml` runs `tests/smoke.spec.ts`
 * only). Run explicitly: `npx playwright test --project=ninebar`.
 *
 * Loads the app with `?flag=ninebar` (see `src/featureFlags.ts`) so the
 * `LayoutGate` in `src/App.tsx` mounts `AppShell` instead of the legacy
 * `AppLayout`, then asserts the testid contract documented on
 * `src/app/AppShell.tsx`: `app-shell` on the root plus one testid per
 * slot (`frame` / `left-rail` / `primary-outlet` / `right-rail` /
 * `bottom-strip`). `right-rail` and `bottom-strip` are closed shells —
 * zero size by design — so we assert attachment, not visibility.
 */
import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

// ── Route interception: stub every backend API call so this spec runs
// fully offline, same pattern as tests/smoke.spec.ts. ─────────────────

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

/** Navigate to the ninebar-flagged app and wait for the shell to attach. */
async function gotoNinebarApp(page: Page) {
  await stubBackend(page);
  await page.goto(`${APP}/?flag=ninebar`, { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('app-shell')).toBeAttached({ timeout: 15_000 });
}

test('AppShell mounts with the five-slot testid contract', async ({ page }) => {
  await gotoNinebarApp(page);

  // Root + frame + primary-outlet + left-rail are visible by default.
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(page.getByTestId('frame')).toBeVisible();
  await expect(page.getByTestId('left-rail')).toBeVisible();
  await expect(page.getByTestId('primary-outlet')).toBeVisible();
  await expect(page.getByTestId('left-rail-toggle')).toBeVisible();

  // right-rail is a closed shell (zero size, data-state "closed") by
  // design — no rail context opens on a fresh load. Assert attachment,
  // not visibility.
  const rightRail = page.getByTestId('right-rail');
  await expect(rightRail).toBeAttached();
  await expect(rightRail).toHaveAttribute('data-state', 'closed');

  // bottom-strip: the root route redirects to `/run`, and ninebar Run
  // (Phase 3, "Run, re-composed") portals its waveform card into the
  // strip unconditionally — so it's OPEN by default here, not closed.
  // (Phase 1's "closed on fresh load" applied only while no workflow
  // had a bottom-strip composition yet; see tests/ninebar-run.spec.ts
  // for the Run-specific strip contract.)
  const bottomStrip = page.getByTestId('bottom-strip');
  await expect(bottomStrip).toBeAttached();
  await expect(bottomStrip).toHaveAttribute('data-state', 'open');
});

test('left-rail toggle collapses the rail', async ({ page }) => {
  await gotoNinebarApp(page);

  const leftRail = page.getByTestId('left-rail');
  const toggle = page.getByTestId('left-rail-toggle');

  // Open by default.
  await expect(leftRail).toHaveAttribute('data-state', 'open');

  await toggle.click();
  await expect(leftRail).toHaveAttribute('data-state', 'closed');

  // Toggling back reopens it — round-trip, not a one-way collapse.
  await toggle.click();
  await expect(leftRail).toHaveAttribute('data-state', 'open');
});
