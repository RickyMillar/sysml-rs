/**
 * Shared QueryClient instance — configured per ADR-004.
 *
 * - Model/workspace queries: staleTime Infinity (manually invalidated)
 * - Default retry: 3 with exponential backoff
 */

import { QueryClient } from '@tanstack/react-query';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Infinity,
      retry: 3,
      refetchOnWindowFocus: false,
    },
  },
});
