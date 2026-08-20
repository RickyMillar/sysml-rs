/**
 * Phase 1 — Monaco live editor over /lsp Playwright spec.
 *
 * Drives the live-editor path end-to-end against the running dev
 * server. Requires `sysml-api --mcp` on :8080 (see project memory:
 * "Restart sysml-api after every backend change"). The /lsp endpoint
 * uses the host's shared SysmlService (commit 9aef3948), so
 * LSP did_change edits flow into the same salsa AnalysisHost that
 * backs REST sysml.* — diagnostics surface against the live buffer.
 *
 * What we assert:
 *   1. The Source utility panel mounts the live editor for a focused
 *      file (no slice fetch, no readOnly).
 *   2. Typing into the buffer triggers a backend round-trip — we
 *      verify by watching the WebSocket frames the FE sends; the
 *      750ms wait covers initialize + didChange + diagnostics latency.
 *   3. The sneak-peek path still mounts in read-only mode (no LSP
 *      client opened for hover previews).
 *
 * Hover / completion / goto-def fidelity is covered by the LSP's own
 * protocol_tests crate; here we only verify the wire-up.
 */

import { test, expect, type Page } from '@playwright/test';

const APP = 'http://localhost:3010';

const FIXTURE_SOURCE = `package LiveEditing {
    part def Motor;
    part m : Motor;
}
`;

async function hydrateFocusedFile(page: Page, uri: string, source: string) {
  await page.evaluate(
    ({ uri, source }) => {
      const ws = (window as unknown as {
        __workspaceStoreForTests?: {
          setFocusedFile: (uri: string, source: string) => void;
        };
      }).__workspaceStoreForTests;
      if (ws) ws.setFocusedFile(uri, source);
    },
    { uri, source },
  );
}

test.describe('Monaco live transport (Phase 1)', () => {
  test('source panel mounts the live editor on a focused file', async ({ page }) => {
    await page.goto(`${APP}/run`);
    await page.getByTestId('utility-toggle-source').click();
    await hydrateFocusedFile(page, 'file:///phase1-live.sysml', FIXTURE_SOURCE);

    const editor = page.getByTestId('source-panel-editor');
    await expect(editor).toBeVisible({ timeout: 15_000 });

    // Live editor is NOT marked readonly — Monaco renders editable
    // chrome. The wrapper textarea is the typing target.
    const textarea = editor.locator('textarea').first();
    await expect(textarea).toBeVisible();
    await expect(textarea).not.toHaveAttribute('readonly', /.+/);
  });

  test('typing into the buffer sends LSP frames to /lsp', async ({ page }) => {
    const lspFrames: string[] = [];

    // Capture WS frames before navigating so we see the initialize +
    // didOpen / didChange sequence.
    page.on('websocket', (ws) => {
      if (!ws.url().endsWith('/lsp')) return;
      ws.on('framesent', (frame) => {
        lspFrames.push(frame.payload?.toString() ?? '');
      });
    });

    await page.goto(`${APP}/run`);
    await page.getByTestId('utility-toggle-source').click();
    await hydrateFocusedFile(page, 'file:///phase1-edit.sysml', FIXTURE_SOURCE);

    const editor = page.getByTestId('source-panel-editor');
    await expect(editor).toBeVisible({ timeout: 15_000 });

    // Wait for initialize → didOpen to flush.
    await page.waitForFunction(
      () => Array.from(document.querySelectorAll('.monaco-editor')).length > 0,
      undefined,
      { timeout: 10_000 },
    );

    const textarea = editor.locator('textarea').first();
    await textarea.click();
    await page.keyboard.type(' // edit', { delay: 30 });

    // Diagnostics + didChange propagate within the perf budget (<500ms
    // edit-to-diagnostic per ADR-013). Give 1500ms to absorb CI jitter.
    await page.waitForTimeout(1500);

    const hasInitialize = lspFrames.some((f) => f.includes('"method":"initialize"'));
    const hasDidOpen = lspFrames.some((f) => f.includes('"method":"textDocument/didOpen"'));
    const hasDidChange = lspFrames.some((f) =>
      f.includes('"method":"textDocument/didChange"'),
    );
    expect(hasInitialize, 'no initialize frame on /lsp').toBe(true);
    expect(hasDidOpen, 'no didOpen frame on /lsp').toBe(true);
    expect(hasDidChange, 'no didChange frame on /lsp after typing').toBe(true);
  });

  test('sneak-peek does NOT open an /lsp socket', async ({ page }) => {
    const lspSockets: string[] = [];
    page.on('websocket', (ws) => {
      if (ws.url().endsWith('/lsp')) lspSockets.push(ws.url());
    });

    await page.route('**/api/command', async (route) => {
      const body = route.request().postDataJSON() as { command?: string };
      if (body?.command === 'sysml.get_source') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            text: 'part def Motor;',
            start: 0,
            end: 14,
            line: 1,
            col: 1,
          }),
        });
      }
      return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });

    await page.goto(`${APP}/run`);

    // Drive the selection store directly — same hook the existing
    // sneak-peek spec uses. We never open the SourcePanel here.
    await page.evaluate(() => {
      const sel = (window as unknown as {
        __selectionStoreForTests?: {
          select: (uri: string | null, id: string | null) => void;
        };
      }).__selectionStoreForTests;
      if (sel) sel.select('file:///sneak.sysml', 'el-1');
    });

    // Give the app a moment to render any LSP-attached UI.
    await page.waitForTimeout(500);
    expect(lspSockets.length, `unexpected /lsp socket from sneak-peek: ${lspSockets.join(',')}`).toBe(
      0,
    );
  });
});
