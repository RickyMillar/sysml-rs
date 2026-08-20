/**
 * Rail-context registration test (ninebar Phase 1, task "Re-home the
 * always-on panels").
 *
 * Proves the five re-homed contexts (variables / breakpoints /
 * diagnostics / views / archive — plus the pre-existing stream-status
 * proof context) resolve via `getRailContext` and render without
 * crashing under a mocked backend + a mocked/default rail store. Each
 * wraps the EXACT component its legacy panel descriptor renders (see
 * `shared/panels/*.ts`), so this is also a light reuse-not-fork check:
 * import side effects alone (via `../contexts`) must be enough to
 * populate the registry.
 *
 * Network-backed hooks are neutralised the same way
 * `VariablesPane.test.tsx` does it: mock `@/shared/api/http` so every
 * query sits in "loading" forever, which every one of these panels'
 * own empty/loading states already handle without touching real data
 * shapes. No real session is active (`useSessionStore`'s default
 * `activeSessionId: null`), so this also exercises each panel's
 * idle/empty path — the one every rail mount starts in.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';

vi.mock('@/shared/api/http', () => ({
  httpGet: vi.fn(() => new Promise(() => {})),
  httpPost: vi.fn(() => new Promise(() => {})),
  httpPostText: vi.fn(() => new Promise(() => {})),
  httpDelete: vi.fn(() => new Promise(() => {})),
  ApiError: class ApiError extends Error {},
}));

// Side-effect import — registers every built-in rail context, exactly
// as `RightRail.tsx` does.
import '../contexts';
import { getRailContext } from '../railRegistry';
import { useSessionStore } from '@/features/sessions/store';

function makeClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
}

function Wrap({ children }: { children: ReactNode }) {
  const client = makeClient();
  return (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/run']}>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}

afterEach(() => {
  cleanup();
  useSessionStore.setState({ activeSessionId: null, phase: 'idle', selectedScope: [] });
});

// `archive` left this list in Phase 6 — its permanent home is the
// history-browser modal (plan §1 row 24), not a rail context.
const REHOMED_CONTEXT_IDS = ['variables', 'breakpoints', 'diagnostics', 'views'];

describe('rail context registration', () => {
  it('registers at least three re-homed contexts (Phase 1 DoD)', () => {
    const resolved = REHOMED_CONTEXT_IDS.map((id) => getRailContext(id));
    expect(resolved.filter(Boolean).length).toBeGreaterThanOrEqual(3);
  });

  it.each(REHOMED_CONTEXT_IDS)('context %s resolves via getRailContext', (id) => {
    const descriptor = getRailContext(id);
    expect(descriptor).toBeDefined();
    expect(descriptor?.title).toBeTruthy();
  });

  it.each(REHOMED_CONTEXT_IDS)('context %s renders without crashing', (id) => {
    const descriptor = getRailContext(id);
    expect(descriptor).toBeDefined();
    if (!descriptor) return;

    render(<Wrap>{descriptor.render()}</Wrap>);

    // Each context wrapper carries its own `rail-context-<id>` testid
    // (see the individual context files) so a rail host could assert
    // per-context mount without depending on the panel's internal DOM.
    expect(screen.getByTestId(`rail-context-${id}`)).toBeInTheDocument();
  });

  it('stream-status (the pre-existing Phase 1 context) still resolves', () => {
    expect(getRailContext('stream-status')).toBeDefined();
  });
});
