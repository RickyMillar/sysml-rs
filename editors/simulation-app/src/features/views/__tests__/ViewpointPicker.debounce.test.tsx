/**
 * ViewpointPicker — debounce contract (Phase 5).
 *
 * Pins the 250ms debounce on the typeahead input: rapid keystrokes
 * must collapse into a single `sysml.query` request to the backend.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { ViewpointPicker, VIEWPOINT_PICKER_DEBOUNCE_MS } from '@/features/views/ViewpointPicker';

const FETCH_MOCK = vi.fn();

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: false });
  FETCH_MOCK.mockReset();
  vi.stubGlobal('fetch', FETCH_MOCK);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function ok(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function makeWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

function fetchBodies(): Array<{
  command?: string;
  params?: { uri?: string; spec?: Record<string, unknown> };
}> {
  return FETCH_MOCK.mock.calls.map((c) =>
    JSON.parse(String(c[1]?.body ?? '{}')),
  );
}

describe('ViewpointPicker debounce', () => {
  it('collapses a burst of keystrokes into one sysml.query after 250ms', async () => {
    FETCH_MOCK.mockResolvedValue(ok({ rows: [], total_estimate: 0 }));

    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewpointPicker
          uri="file:///x.sysml"
          selectedId={null}
          selectedLabel={null}
          onSelect={() => {}}
          onClear={() => {}}
        />
      </Wrapper>,
    );

    const input = screen.getByTestId('viewpoint-picker-input') as HTMLInputElement;

    // Focus opens the list, which triggers the empty-query fetch.
    act(() => {
      input.focus();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(VIEWPOINT_PICKER_DEBOUNCE_MS);
    });
    const initialCount = FETCH_MOCK.mock.calls.length;

    // Rapid keystrokes — each is well under the debounce window.
    act(() => {
      fireEvent.change(input, { target: { value: 'V' } });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    act(() => {
      fireEvent.change(input, { target: { value: 'Vi' } });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    act(() => {
      fireEvent.change(input, { target: { value: 'Vie' } });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    act(() => {
      fireEvent.change(input, { target: { value: 'View' } });
    });

    // Mid-burst: no extra fetch yet — debounce window has not closed.
    expect(FETCH_MOCK.mock.calls.length).toBe(initialCount);

    // After one more full debounce window, exactly one new fetch fires.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(VIEWPOINT_PICKER_DEBOUNCE_MS);
    });
    expect(FETCH_MOCK.mock.calls.length).toBe(initialCount + 1);

    const bodies = fetchBodies();
    const last = bodies[bodies.length - 1];
    expect(last.command).toBe('sysml.query');
    const filter = last.params?.spec?.filter as
      | { type?: string; filters?: Array<Record<string, unknown>> }
      | undefined;
    expect(filter?.type).toBe('all');
    const nameFilter = filter?.filters?.find((f) => f.type === 'name_match') as
      | { name_match?: { contains?: string; ci?: boolean } }
      | undefined;
    expect(nameFilter?.name_match?.contains).toBe('View');
    expect(nameFilter?.name_match?.ci).toBe(true);
  });
});
