import { useMutation } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type { VerdictKind } from '@/engine/types';

export interface EvaluateExpressionInput {
  elementId: string;
  overrides?: Record<string, string>;
}

export interface EvaluateExpressionResult {
  element_id: string;
  element_name?: string | null;
  source?: string | null;
  value?: unknown;
  display?: string;
  value_type?: string;
  verdict: VerdictKind;
  symbols?: string[];
  context?: Record<string, unknown>;
  diagnostics?: string[];
}

function overridesToTuples(overrides: Record<string, string> | undefined): [string, string][] {
  if (!overrides) return [];
  return Object.entries(overrides)
    .map(([key, value]) => [key.trim(), value.trim()] as [string, string])
    .filter(([key, value]) => key.length > 0 && value.length > 0);
}

async function evaluateExpression(input: EvaluateExpressionInput): Promise<EvaluateExpressionResult> {
  return httpPost<EvaluateExpressionResult>('/api/command', {
    command: 'sysml.evaluate.expression',
    params: {
      element_id: input.elementId,
      overrides: overridesToTuples(input.overrides),
    },
  });
}

export function useEvaluateExpression() {
  return useMutation({ mutationFn: evaluateExpression });
}
