/**
 * React-query hook for `sysml.get_source`.
 *
 * Disabled until both `uri` and `id` are non-null so a freshly-mounted
 * editor doesn't fire a request before the user has picked anything.
 */

import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { getSource, type GetSourceResult } from './getSource';

export function useGetSource(
  uri: string | null,
  id: string | null,
): UseQueryResult<GetSourceResult | null> {
  return useQuery<GetSourceResult | null>({
    queryKey: ['sysml.get_source', uri, id],
    queryFn: () => {
      if (!uri || !id) return Promise.resolve(null);
      return getSource({ uri, id });
    },
    enabled: !!uri && !!id,
    staleTime: 5_000,
  });
}
