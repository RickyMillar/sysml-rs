/**
 * Client for the backend `sysml.get_source` service command (S4.T2).
 *
 * Wire shape mirrors `sysml-service/src/types.rs::GetSourceResult` —
 * snake_case, drops `Span.file` because the caller already knows the URI.
 */

import { httpPost } from '@/shared/api/http';

export interface GetSourceResult {
  /** Source slice covered by the element's first span. */
  text: string;
  /** Byte offset start (inclusive) in the file. */
  start: number;
  /** Byte offset end (exclusive) in the file. */
  end: number;
  /** 1-based line number, if computable from the file's position map. */
  line?: number;
  /** 1-based column number, if computable from the file's position map. */
  col?: number;
}

/**
 * Fetch the source slice for an element. Returns `null` when the element
 * has no first-span / name-span (backend returns JSON `null` in that case).
 *
 * Throws `ApiError` for transport / status failures — callers wrap in
 * react-query for retry / status surfacing.
 */
export async function getSource(params: {
  uri: string;
  id: string;
}): Promise<GetSourceResult | null> {
  const result = await httpPost<GetSourceResult | null>('/api/command', {
    command: 'sysml.get_source',
    params: { uri: params.uri, id: params.id },
  });
  return result;
}
