/**
 * Fixture-mode backend for the standalone diagram embed (spike).
 *
 * The embed page runs the app's real view pipeline (SelectedViewRenderer →
 * DiagramHost → SvgCanvas / BrowserView / …) with NO backend: this module
 * monkey-patches `window.fetch` so every backend-shaped request the bundled
 * app modules make (the REST transport bottoms out in `fetch`) is answered
 * from a baked ViewModel fixture instead of `sysml-api`.
 *
 * Served commands:
 *   - `sysml.diagram.viewmodel`          → the loaded fixture ViewModel
 *   - `sysml.diagram.diagnostic_overlay` → null (no diagnostics in fixture mode)
 *   - `sysml.diagram.sim_overlay` /
 *     `sysml.diagram.verdict_overlay`    → null (never enabled: no session)
 *   - `sysml.query`                      → empty row set
 *   - anything else                      → null + a console.warn (a warn, not an
 *     error — the embed proof asserts a clean error console)
 *
 * Served REST paths:
 *   - `/models/:uri/elements/:id`           → a minimal element stub — clicking
 *     a node makes the selection store fetch detail for a (never-rendered)
 *     inspector; a stub keeps selection purely visual with a quiet console
 *   - `/models/:uri/elements/:id/children`  → empty list
 *   - `/health`                             → ok
 *
 * Non-backend URLs (the fixture JSON itself, hashed assets) pass through to
 * the real fetch untouched.
 */

/** The real fetch, captured before the patch — fixture/asset loads use this. */
export const realFetch: typeof fetch = window.fetch.bind(window);

let fixtureViewModel: unknown = null;

/** Install the baked ViewModel served to every `sysml.diagram.viewmodel` call. */
export function setFixtureViewModel(vm: unknown): void {
  fixtureViewModel = vm;
}

/** Path prefixes the app's REST transport treats as backend endpoints. */
const BACKEND_PREFIXES = [
  '/api',
  '/models',
  '/sources',
  '/sessions',
  '/health',
  '/files',
  '/workspace',
  '/commands',
];

function jsonResponse(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function requestUrl(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input;
  if (input instanceof URL) return input.pathname + input.search;
  return input.url;
}

async function answerCommand(body: string | undefined): Promise<Response> {
  let command = '';
  try {
    command = (JSON.parse(body ?? '{}') as { command?: string }).command ?? '';
  } catch {
    // fall through to the unknown-command arm
  }
  switch (command) {
    case 'sysml.diagram.viewmodel':
      return jsonResponse(fixtureViewModel);
    case 'sysml.diagram.diagnostic_overlay':
    case 'sysml.diagram.sim_overlay':
    case 'sysml.diagram.verdict_overlay':
      return jsonResponse(null);
    case 'sysml.query':
      return jsonResponse({ rows: [] });
    default:
      console.warn(`[embed fixture] unhandled command "${command}" — returning null`);
      return jsonResponse(null);
  }
}

/** Patch `window.fetch` so backend-shaped requests resolve from the fixture. */
export function installFixtureFetch(): void {
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = requestUrl(input);
    const isBackend = BACKEND_PREFIXES.some(
      (p) => url === p || url.startsWith(`${p}/`) || url.startsWith(`${p}?`),
    );
    if (!isBackend) return realFetch(input, init);

    if (url.startsWith('/api/command')) {
      const body =
        typeof init?.body === 'string'
          ? init.body
          : input instanceof Request
            ? await input.clone().text()
            : undefined;
      return answerCommand(body);
    }
    if (url.startsWith('/health')) return jsonResponse({ status: 'ok' });

    // Selection detail (selection store's fetchDetail, fired by node clicks).
    const elementMatch = /^\/models\/[^/]+\/elements\/([^/?]+)(\/children)?(?:\?|$)/.exec(url);
    if (elementMatch) {
      if (elementMatch[2]) return jsonResponse([]);
      return jsonResponse({
        id: decodeURIComponent(elementMatch[1]),
        kind: 'Element',
        name: null,
        props: {},
        spans: [],
      });
    }

    console.warn(`[embed fixture] unhandled backend request "${url}" — returning null`);
    return jsonResponse(null);
  };
}
