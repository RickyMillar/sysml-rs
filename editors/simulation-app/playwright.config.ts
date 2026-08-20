import { defineConfig } from '@playwright/test';

/**
 * THE FLIP (ninebar Phase 3, 2026-07-14): `ninebar` is default-ON in the
 * app, so the LEGACY projects (smoke/integration/default — 27 spec files
 * written against `AppLayout`) pin it OFF via `storageState` seeding
 * `localStorage['sysml.flag.ninebar']='0'` (an explicit stored opt-out,
 * honoured by `isFlagEnabled`). They are NOT ported — they die with the
 * legacy shell in Phase 8. The `ninebar` project needs no flag at all
 * now but keeps `?flag=ninebar` URLs harmlessly (they store '1').
 */
const LEGACY_SHELL_STATE = {
  cookies: [],
  origins: [
    {
      origin: 'http://localhost:3010',
      localStorage: [{ name: 'sysml.flag.ninebar', value: '0' }],
    },
  ],
};

const CHROMIUM = {
  browserName: 'chromium' as const,
  headless: true,
  video: 'on' as const,
  screenshot: 'on' as const,
  viewport: { width: 1920, height: 1080 },
  launchOptions: { slowMo: 50 },
};

export default defineConfig({
  testDir: './tests',
  timeout: 180_000,
  retries: 0,
  projects: [
    {
      name: 'smoke',
      testMatch: /smoke\.spec\.ts/,
      use: { ...CHROMIUM, storageState: LEGACY_SHELL_STATE },
    },
    {
      name: 'integration',
      testMatch: /integration\.spec\.ts/,
      use: { ...CHROMIUM, storageState: LEGACY_SHELL_STATE },
    },
    {
      // The ninebar shell — BLOCKING since the flip (CI runs it alongside
      // smoke; see .github/workflows/simulation-app.yml).
      name: 'ninebar',
      testMatch: /ninebar.*\.spec\.ts/,
      use: CHROMIUM,
    },
    {
      name: 'default',
      testIgnore: [
        /integration\.spec\.ts/,
        /smoke\.spec\.ts/,
        /ninebar.*\.spec\.ts/,
      ],
      use: { ...CHROMIUM, storageState: LEGACY_SHELL_STATE },
    },
  ],
  // Auto-start the vite dev server.
  // Existing e2e tests that need the backend still manually start sysml-api.
  webServer: {
    command: 'npx vite --port 3010',
    port: 3010,
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
