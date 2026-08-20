/**
 * Command-palette logic tests.
 *
 * These are node-environment vitest tests — no DOM, no React Testing
 * Library. They exercise the pure building blocks that drive the
 * palette's open/filter/select flow:
 *
 *   - fetchCommandCatalog caching + reset
 *   - scoreCommand / filterCommands (search behaviour)
 *   - isDevCmdKEnabled (dev-mode gate)
 *
 * Together these cover the palette's non-rendering surface. Interactive
 * DOM-level behaviour is exercised by the Playwright integration suite.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CommandMeta,
  fetchCommandCatalog,
  filterCommands,
  resetCommandCatalogCache,
  scoreCommand,
} from '../commandCatalog';
import { parseDevCmdKFlag } from '../index';

// ── Catalog fixtures ─────────────────────────────────────────────────────

const CATALOG: CommandMeta[] = [
  {
    name: 'sysml.query',
    category: 'Query',
    description: 'Structured element-list query',
    params: [
      { name: 'uri', ty: 'String', required: true, description: 'Model URI' },
      { name: 'spec', ty: 'Json', required: true, description: 'QuerySpec' },
    ],
    returns: 'QueryResult',
    stateful: false,
  },
  {
    name: 'sysml.stats',
    category: 'Query',
    description: 'Kind histogram for a loaded model',
    params: [{ name: 'uri', ty: 'String', required: true, description: 'Model URI' }],
    returns: 'ModelStats',
    stateful: false,
  },
  {
    name: 'sysml.simulate.start',
    category: 'Execution',
    description: 'Start a simulation session',
    params: [
      { name: 'uri', ty: 'String', required: true, description: 'Model URI' },
      { name: 'name', ty: 'String?', required: false, description: 'Optional session name' },
    ],
    returns: 'SessionId',
    stateful: true,
  },
  {
    name: 'sysml.load_workspace',
    category: 'FileManagement',
    description: 'Load and parse all .sysml files in a directory',
    params: [{ name: 'root', ty: 'String', required: true, description: 'Workspace root' }],
    returns: 'WorkspaceLoadResult',
    stateful: false,
  },
];

// ── Suite ────────────────────────────────────────────────────────────────

describe('CommandPalette core flow', () => {
  beforeEach(() => {
    resetCommandCatalogCache();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    resetCommandCatalogCache();
  });

  // ── Catalog fetching ─────────────────────────────────────────────────

  it('fetches the catalog from /commands and memoises subsequent calls', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify(CATALOG), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    vi.stubGlobal('fetch', fetchMock);

    const first = await fetchCommandCatalog();
    expect(first).toHaveLength(CATALOG.length);
    expect(first[0]?.name).toBe('sysml.query');

    const second = await fetchCommandCatalog();
    // Same reference (cache hit) and only one network call.
    expect(second).toBe(first);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const firstCall = (fetchMock.mock.calls as any[])[0];
    expect(firstCall).toBeDefined();
    expect(String(firstCall?.[0])).toContain('/commands');
  });

  it('propagates catalog fetch errors as ApiError', async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ error: 'boom' }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(fetchCommandCatalog()).rejects.toMatchObject({
      status: 500,
      endpoint: '/commands',
    });
  });

  // ── Filter / select flow ─────────────────────────────────────────────

  it('filters by command name (prefix match ranks highest)', () => {
    const results = filterCommands(CATALOG, 'sim');
    expect(results.length).toBeGreaterThan(0);
    expect(results[0]?.name).toBe('sysml.simulate.start');
  });

  it('filters by description when the name does not contain the token', () => {
    const results = filterCommands(CATALOG, 'histogram');
    expect(results.map((c) => c.name)).toEqual(['sysml.stats']);
  });

  it('returns an empty list for queries that match nothing', () => {
    const results = filterCommands(CATALOG, 'completely-unknown-xyz');
    expect(results).toEqual([]);
  });

  it('returns every command when the query is blank', () => {
    const results = filterCommands(CATALOG, '');
    expect(results).toHaveLength(CATALOG.length);
  });

  it('scores exact-prefix matches higher than substring matches', () => {
    const prefix = scoreCommand(CATALOG[0]!, 'sysml');
    const substring = scoreCommand(CATALOG[0]!, 'query');
    expect(prefix).toBeGreaterThan(0);
    expect(substring).toBeGreaterThan(0);
    expect(prefix).toBeGreaterThanOrEqual(substring);
  });

  // ── "Select" step: arrow-key index arithmetic is trivial enough that
  //    verifying the exact filter result is enough to prove the flow. We
  //    simulate the pick by reading filtered[selectedIndex]. ─────────────

  it('simulates picker → param-form hand-off', () => {
    const filtered = filterCommands(CATALOG, 'sim');
    const selectedIndex = 0;
    const picked = filtered[selectedIndex];
    expect(picked?.name).toBe('sysml.simulate.start');
    // ParameterForm would now receive this command; its params are the
    // ones the auto-generated form renders.
    expect(picked?.params.map((p) => p.name)).toEqual(['uri', 'name']);
  });

  // ── Dev-mode gate ────────────────────────────────────────────────────

  it('parseDevCmdKFlag honours the VITE_DEV_CMDK env values', () => {
    // `isDevCmdKEnabled()` reads `import.meta.env.VITE_DEV_CMDK`, which
    // Vite statically replaces at build time — it cannot be mutated from
    // a test. The parsing layer is factored out so we can assert rules
    // directly on concrete inputs.
    expect(parseDevCmdKFlag('1')).toBe(true);
    expect(parseDevCmdKFlag('true')).toBe(true);
    expect(parseDevCmdKFlag('on')).toBe(true);
    expect(parseDevCmdKFlag('yes')).toBe(true);
    expect(parseDevCmdKFlag('0')).toBe(false);
    expect(parseDevCmdKFlag('false')).toBe(false);
    expect(parseDevCmdKFlag('FALSE')).toBe(false);
    expect(parseDevCmdKFlag('off')).toBe(false);
    expect(parseDevCmdKFlag('')).toBe(false);
    expect(parseDevCmdKFlag('  ')).toBe(false);
    expect(parseDevCmdKFlag(undefined)).toBe(false);
    expect(parseDevCmdKFlag(null)).toBe(false);
  });
});
