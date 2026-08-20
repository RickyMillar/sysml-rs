import { afterEach, describe, expect, it, vi } from 'vitest';
import { getSource } from '../getSource';

const ORIGINAL_FETCH = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = ORIGINAL_FETCH;
  vi.restoreAllMocks();
});

function mockJsonResponse(body: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: 'OK',
    json: () => Promise.resolve(body),
  } as Response;
}

describe('getSource', () => {
  it('POSTs to /api/command with the get_source envelope', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      mockJsonResponse({ text: 'package P {}', start: 0, end: 12, line: 1, col: 1 }),
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    const result = await getSource({ uri: 'file:///a.sysml', id: 'el-1' });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [path, init] = fetchMock.mock.calls[0]!;
    expect(path).toBe('/api/command');
    expect((init as RequestInit).method).toBe('POST');
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body).toEqual({
      command: 'sysml.get_source',
      params: { uri: 'file:///a.sysml', id: 'el-1' },
    });
    expect(result).toEqual({ text: 'package P {}', start: 0, end: 12, line: 1, col: 1 });
  });

  it('returns null when the backend returns JSON null', async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(mockJsonResponse(null)) as unknown as typeof fetch;
    const result = await getSource({ uri: 'file:///a.sysml', id: 'missing' });
    expect(result).toBeNull();
  });

  it('throws on non-2xx responses', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(mockJsonResponse({ error: 'no such uri' }, 404)) as unknown as typeof fetch;
    await expect(
      getSource({ uri: 'file:///nope.sysml', id: 'x' }),
    ).rejects.toThrow(/no such uri/);
  });
});
