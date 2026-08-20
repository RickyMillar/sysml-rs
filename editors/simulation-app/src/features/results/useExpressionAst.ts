/**
 * useExpressionAst — fetch `sysml.expression.ast` so the Equations card
 * can render rendered math / clickable symbols.
 *
 * `sysml.expression.ast` is workspace-scoped (scope-collapse W2 dropped
 * its uri param): the ASTs cover the whole merged graph regardless of
 * which file the caller is looking at, so the query keys on a constant
 * rather than the uri. The `uri` argument is retained only as an
 * enable-gate — we don't fetch until the caller has a context.
 *
 * Results are structural (calc-def / constraint-def bodies), not
 * per-tick, so staleTime is long.
 */
import { useQuery } from '@tanstack/react-query';
import type { ExpressionAstResult } from '@sysml-rs/expression-view';
import { httpPost } from '../../shared/api/http';

async function fetchExpressionAst(): Promise<ExpressionAstResult[]> {
  return httpPost<ExpressionAstResult[]>('/api/command', {
    command: 'sysml.expression.ast',
    params: {},
  });
}

export function useExpressionAst(uri: string | null) {
  return useQuery({
    queryKey: ['expression-ast'],
    queryFn: () => fetchExpressionAst(),
    enabled: !!uri,
    staleTime: 5 * 60 * 1000, // 5 min — ASTs change only when source reloads
  });
}
