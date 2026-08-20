/**
 * REST transport implementation — thin fetch() wrapper.
 *
 * Consumed by `transport.ts` when running in a browser or Vite dev-server
 * context (i.e. not inside Tauri).  Do not import this module directly from
 * feature code; import from `./transport` (or the backward-compat `./http`).
 */

const BASE_URL =
  (typeof import.meta !== 'undefined' &&
    (import.meta as unknown as Record<string, unknown>).env &&
    ((import.meta as unknown as Record<string, Record<string, unknown>>).env
      .VITE_API_BASE_URL as string | undefined)) ?? '';

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public override readonly message: string,
    public readonly endpoint: string,
  ) {
    super(`API ${status} ${endpoint}: ${message}`);
    this.name = 'ApiError';
  }
}

async function request(path: string, options?: RequestInit): Promise<Response> {
  const url = `${BASE_URL}${path}`;
  const response = await fetch(url, options);
  if (!response.ok) {
    let errorMessage: string;
    try {
      const body = (await response.json()) as Record<string, unknown>;
      errorMessage =
        typeof body.error === 'string' ? body.error : response.statusText;
    } catch {
      errorMessage = response.statusText;
    }
    throw new ApiError(response.status, errorMessage, path);
  }
  return response;
}

export async function httpGetRest<T>(path: string): Promise<T> {
  const response = await request(path);
  return response.json() as Promise<T>;
}

export async function httpPostRest<T>(path: string, body?: unknown): Promise<T> {
  const response = await request(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  return response.json() as Promise<T>;
}

export async function httpPostTextRest(path: string, body?: unknown): Promise<string> {
  const response = await request(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  return response.text();
}

export async function httpDeleteRest(path: string): Promise<void> {
  await request(path, { method: 'DELETE' });
}
