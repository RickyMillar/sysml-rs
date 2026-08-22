/**
 * Standalone diagram-embed entry.
 *
 * Boots the app's real view pipeline — SelectedViewRenderer (ViewModel fetch +
 * non-graph dispatch) → EmbedDiagramHost (renderer dispatch) → SvgCanvas /
 * BrowserView / TableView / GeometryView — against a ViewModel fixture, with
 * no app shell and no backend. Built by `vite.embed.config.ts` into a
 * self-contained static dist an <iframe> can load from any mount path.
 *
 * URL parameters and the embedding contract (CSS hooks, fixture format) are
 * documented in ./README.md. In short:
 *   - `?src=<url>`       fetch a ViewModel JSON (relative to the page) — the
 *                        primary path for hosts like the book, which serve
 *                        fixtures as plain .json files next to the viewer.
 *   - `?fixture=<name>`  one of the demo fixtures bundled from ./fixtures/
 *                        (basename, no extension). Default: the first one.
 *   - `?theme=light`     set data-theme="light" on <html> (the app's existing
 *                        light-canvas ramp in tokens.css picks it up).
 *
 * Interactions: pan/zoom, drag, hover, and expand/collapse are fully live.
 * Element click / double-click only move the visual selection highlight — the
 * fixture transport answers the selection store's element-detail fetches with
 * an empty stub, and no inspector ever renders.
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { installFixtureFetch, realFetch, setFixtureViewModel } from './fixture-transport';
import { useWorkspaceStore } from '@/store/workspace';
import { WORKSPACE_URI } from '@/shared/api/model';
import { SelectedViewRenderer } from '@/features/views/SelectedViewRenderer';
import { EmbedDiagramHost } from './EmbedDiagramHost';
import '@/styles/global.css';
import './embed-fonts.css';

// Patch fetch before anything renders (module init itself never fetches; the
// REST transport resolves `window.fetch` per call).
installFixtureFetch();

/** Synthetic view id — the fixture transport ignores it and always serves the
 *  loaded fixture, but the store needs a non-null selectedViewId to render. */
const EMBED_VIEW_ID = '__embed_fixture_view__';

/** name → asset URL for every fixture baked into the bundle. */
const bundledFixtures: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob('./fixtures/*.json', { query: '?url', import: 'default', eager: true }),
  ).map(([path, url]) => [path.replace(/^\.\/fixtures\//, '').replace(/\.json$/, ''), url as string]),
);

function fail(message: string): never {
  const el = document.getElementById('root');
  if (el) el.textContent = message;
  throw new Error(message);
}

async function loadFixture(): Promise<unknown> {
  const params = new URLSearchParams(window.location.search);
  const src = params.get('src');
  const name = params.get('fixture');
  let url: string;
  if (src) {
    url = src;
  } else if (name) {
    url = bundledFixtures[name] ?? fail(`Unknown fixture "${name}". Bundled: ${Object.keys(bundledFixtures).join(', ')}`);
  } else {
    const first = Object.keys(bundledFixtures).sort()[0];
    if (!first) fail('No bundled fixtures and no ?src= given.');
    url = bundledFixtures[first];
  }
  const res = await realFetch(url);
  if (!res.ok) fail(`Fixture fetch failed: ${res.status} ${url}`);
  return res.json();
}

async function boot() {
  const params = new URLSearchParams(window.location.search);
  if (params.get('theme') === 'light') {
    document.documentElement.setAttribute('data-theme', 'light');
  }

  setFixtureViewModel(await loadFixture());

  // Seed the two store fields the pipeline keys on. Direct setState — the
  // store's own actions (focusFile/setFocusedUri) guard on loadedFiles, which
  // fixture mode has no reason to populate.
  useWorkspaceStore.setState({ focusedUri: WORKSPACE_URI, selectedViewId: EMBED_VIEW_ID });

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <SelectedViewRenderer />
        <EmbedDiagramHost />
      </QueryClientProvider>
    </StrictMode>,
  );
}

void boot();
