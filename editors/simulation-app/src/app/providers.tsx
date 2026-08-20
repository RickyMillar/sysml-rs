/**
 * App-level providers — wraps the component tree in shared context.
 *
 * Currently provides:
 * - QueryClientProvider (react-query, configured per ADR-004)
 */

import type { ReactNode } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import { queryClient } from '@/shared/query/client';

export function Providers({ children }: { children: ReactNode }) {
  return (
    <QueryClientProvider client={queryClient}>
      {children}
    </QueryClientProvider>
  );
}
