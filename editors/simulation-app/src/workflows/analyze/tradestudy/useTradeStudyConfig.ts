/**
 * useTradeStudyConfig — local UI state for the TradeStudyWorkflow config panel.
 *
 * Three pieces of user intent drive a trade study run:
 *
 *   1. A list of **design alternatives** — each one a named point in
 *      parameter space (`label` + `overrides: Record<string, Value>`).
 *      Array order is preserved end-to-end: HH's results viewer displays
 *      alternatives in submission order, so we never sort or dedupe.
 *   2. A list of **criteria** — MetricDescriptor ids + a per-metric
 *      objective (`'min' | 'max'`, lowercase, matches TradeStudies
 *      stdlib's MinimizeObjective/MaximizeObjective).
 *   3. A list of **weights** — one numeric weight per criterion. Defaults
 *      to equal (1/N); normalised to sum to 1 on submit (see
 *      `normalizeWeights` below). Weights are NOT required to sum to 1
 *      while the user is editing.
 *
 * Pure local state, matches the pattern of `useVerifyConfig`. Lives for
 * the lifetime of the TradeStudyWorkflow page.
 *
 * Consumers: `TradeStudyConfig` (UI) + `TradeStudyWorkflow` (submit
 * wiring). Tested in `__tests__/useTradeStudyConfig.test.ts`.
 */

import { useCallback, useMemo, useState } from 'react';
import type { Value } from '@/engine/types';

// ── Types ───────────────────────────────────────────────────────────

/** Objective direction per the TradeStudies stdlib. Lowercase on the
 *  wire to match Rust serde defaults. */
export type TradeStudyObjective = 'min' | 'max';

/**
 * A single design alternative the user wants to evaluate. The `overrides`
 * map mirrors the `Record<string, Value>` shape used everywhere else in
 * the engine surface (start, fork, whatif).
 */
export interface AlternativeConfig {
  /** Stable client-side id (never sent to backend). */
  id: string;
  /** Human-readable label (e.g. "Design A"). Submitted to backend
   *  as the alternative name. */
  label: string;
  /** Parameter overrides for this alternative. */
  overrides: Record<string, Value>;
}

/**
 * A single criterion: which metric to score on, and the objective
 * direction (minimise vs maximise).
 */
export interface CriterionConfig {
  /** Metric id from the shared MetricRegistry. */
  metricId: string;
  /** Objective direction. */
  objective: TradeStudyObjective;
}

// ── Heuristics ──────────────────────────────────────────────────────

/**
 * Default objective for a new criterion based on metric name heuristic.
 *
 * Metrics whose name contains "cost" / "latency" / "error" / "penalty"
 * default to Min (you want less of them). Everything else defaults to
 * Max (you want more performance / score / throughput).
 *
 * Kept separate from the hook so tests can exercise it in isolation.
 */
export function defaultObjectiveForMetric(name: string): TradeStudyObjective {
  const n = name.toLowerCase();
  if (
    n.includes('cost') ||
    n.includes('latency') ||
    n.includes('error') ||
    n.includes('penalty')
  ) {
    return 'min';
  }
  return 'max';
}

// ── Weight normalisation ────────────────────────────────────────────

/**
 * Normalise a weight vector to sum to 1.
 *
 * Rules:
 *   - Empty input → empty output.
 *   - All-zero / all-negative input → equal weights (1/N).
 *   - Any NaN / non-finite entries are treated as 0.
 *   - Negative entries are clamped to 0 before normalisation.
 *   - Otherwise: scale by 1/sum so the total is exactly 1.
 *
 * Deterministic + pure; tested directly.
 */
export function normalizeWeights(weights: readonly number[]): number[] {
  const n = weights.length;
  if (n === 0) return [];
  const clean = weights.map((w) => (Number.isFinite(w) && w > 0 ? w : 0));
  const sum = clean.reduce((a, b) => a + b, 0);
  if (sum <= 0) {
    const eq = 1 / n;
    return Array.from({ length: n }, () => eq);
  }
  return clean.map((w) => w / sum);
}

/** Validation — what the Run button requires. Exported so the UI and
 *  tests share the contract. */
export interface TradeStudyValidation {
  /** True when the config is runnable. */
  canRun: boolean;
  /** One-line reason it cannot run (null when `canRun`). */
  reason: string | null;
}

export function validateTradeStudyConfig(
  alternatives: readonly AlternativeConfig[],
  criteria: readonly CriterionConfig[],
): TradeStudyValidation {
  if (alternatives.length < 2) {
    return { canRun: false, reason: 'Add at least two alternatives.' };
  }
  if (criteria.length < 1) {
    return { canRun: false, reason: 'Pick at least one criterion.' };
  }
  return { canRun: true, reason: null };
}

// ── Hook ────────────────────────────────────────────────────────────

export interface TradeStudyConfigState {
  alternatives: AlternativeConfig[];
  criteria: CriterionConfig[];
  /** Weight per criterion. Index-aligned with `criteria`. Raw user
   *  input — not guaranteed to sum to 1. */
  weights: number[];

  // alternative actions
  addAlternative: (label?: string) => void;
  removeAlternative: (id: string) => void;
  renameAlternative: (id: string, label: string) => void;
  setOverride: (id: string, key: string, value: Value) => void;
  removeOverride: (id: string, key: string) => void;

  // criterion actions
  addCriterion: (metricId: string, metricName?: string) => void;
  removeCriterion: (metricId: string) => void;
  setObjective: (metricId: string, objective: TradeStudyObjective) => void;
  setWeight: (metricId: string, weight: number) => void;
  resetWeights: () => void;

  // derived
  validation: TradeStudyValidation;
  /** Weights normalised to sum to 1 (for submit + preview). */
  normalizedWeights: number[];
  hasAlternatives: boolean;
  hasCriteria: boolean;
}

export interface UseTradeStudyConfigOptions {
  initialAlternatives?: AlternativeConfig[];
  initialCriteria?: CriterionConfig[];
  initialWeights?: number[];
}

/** Monotonic id counter for alternative rows. Client-only. */
let altIdCounter = 0;
function nextAltId(): string {
  altIdCounter += 1;
  return `alt-${altIdCounter}`;
}

/** Default label for the Nth alternative: "Design A", "Design B", ... */
export function defaultAlternativeLabel(index: number): string {
  // Support >26 by wrapping: Design A..Z, Design AA..AZ, ...
  if (index < 26) {
    return `Design ${String.fromCharCode(65 + index)}`;
  }
  const hi = Math.floor(index / 26) - 1;
  const lo = index % 26;
  return `Design ${String.fromCharCode(65 + hi)}${String.fromCharCode(65 + lo)}`;
}

export function useTradeStudyConfig(
  opts: UseTradeStudyConfigOptions = {},
): TradeStudyConfigState {
  const {
    initialAlternatives = [],
    initialCriteria = [],
    initialWeights,
  } = opts;

  const [alternatives, setAlternatives] = useState<AlternativeConfig[]>(
    () => initialAlternatives.map((a) => ({ ...a, overrides: { ...a.overrides } })),
  );
  const [criteria, setCriteria] = useState<CriterionConfig[]>(
    () => initialCriteria.map((c) => ({ ...c })),
  );
  const [weights, setWeights] = useState<number[]>(() => {
    if (initialWeights && initialWeights.length === initialCriteria.length) {
      return [...initialWeights];
    }
    if (initialCriteria.length === 0) return [];
    const eq = 1 / initialCriteria.length;
    return initialCriteria.map(() => eq);
  });

  // ── alternatives ──────────────────────────────────────────────────

  const addAlternative = useCallback((label?: string) => {
    setAlternatives((prev) => {
      const id = nextAltId();
      const nextLabel = label ?? defaultAlternativeLabel(prev.length);
      return [...prev, { id, label: nextLabel, overrides: {} }];
    });
  }, []);

  const removeAlternative = useCallback((id: string) => {
    setAlternatives((prev) => prev.filter((a) => a.id !== id));
  }, []);

  const renameAlternative = useCallback((id: string, label: string) => {
    setAlternatives((prev) =>
      prev.map((a) => (a.id === id ? { ...a, label } : a)),
    );
  }, []);

  const setOverride = useCallback(
    (id: string, key: string, value: Value) => {
      setAlternatives((prev) =>
        prev.map((a) =>
          a.id === id ? { ...a, overrides: { ...a.overrides, [key]: value } } : a,
        ),
      );
    },
    [],
  );

  const removeOverride = useCallback((id: string, key: string) => {
    setAlternatives((prev) =>
      prev.map((a) => {
        if (a.id !== id) return a;
        if (!(key in a.overrides)) return a;
        const next = { ...a.overrides };
        delete next[key];
        return { ...a, overrides: next };
      }),
    );
  }, []);

  // ── criteria ──────────────────────────────────────────────────────

  const addCriterion = useCallback(
    (metricId: string, metricName?: string) => {
      setCriteria((prev) => {
        if (prev.some((c) => c.metricId === metricId)) return prev;
        const objective = defaultObjectiveForMetric(metricName ?? metricId);
        const next = [...prev, { metricId, objective }];
        // Extend weights to match: give the new criterion an equal share
        // and scale the rest so all old ones retain their relative sizes.
        setWeights((prevW) => {
          const nLen = next.length;
          // Simple rule: reset to equal when adding. Keeps semantics
          // obvious to users ("added a criterion, now all weights are
          // equal"). Expert users can fine-tune after.
          return Array.from({ length: nLen }, () => 1 / nLen);
        });
        return next;
      });
    },
    [],
  );

  const removeCriterion = useCallback((metricId: string) => {
    setCriteria((prev) => {
      const idx = prev.findIndex((c) => c.metricId === metricId);
      if (idx < 0) return prev;
      const next = prev.filter((_, i) => i !== idx);
      setWeights((prevW) => {
        const filtered = prevW.filter((_, i) => i !== idx);
        if (filtered.length === 0) return [];
        // Renormalise leftover weights so they sum to 1 in the UI too.
        return normalizeWeights(filtered);
      });
      return next;
    });
  }, []);

  const setObjective = useCallback(
    (metricId: string, objective: TradeStudyObjective) => {
      setCriteria((prev) =>
        prev.map((c) => (c.metricId === metricId ? { ...c, objective } : c)),
      );
    },
    [],
  );

  const setWeight = useCallback((metricId: string, weight: number) => {
    setCriteria((prev) => {
      const idx = prev.findIndex((c) => c.metricId === metricId);
      if (idx < 0) return prev;
      setWeights((prevW) => {
        const next = [...prevW];
        // Pad with zero if weights is shorter than criteria (shouldn't
        // happen but defensive).
        while (next.length < prev.length) next.push(0);
        next[idx] = Number.isFinite(weight) ? weight : 0;
        return next;
      });
      return prev;
    });
  }, []);

  const resetWeights = useCallback(() => {
    // Re-seed to equal based on the current criteria length. Reading
    // criteria via the functional updater keeps this tear-safe under
    // React 18 batched state.
    setCriteria((prev) => {
      const n = prev.length;
      setWeights(n === 0 ? [] : Array.from({ length: n }, () => 1 / n));
      return prev;
    });
  }, []);

  // ── derived ───────────────────────────────────────────────────────

  const validation = useMemo(
    () => validateTradeStudyConfig(alternatives, criteria),
    [alternatives, criteria],
  );

  const normalizedWeights = useMemo(() => {
    if (weights.length !== criteria.length) {
      // Fallback: equal weights when out of sync.
      if (criteria.length === 0) return [];
      const eq = 1 / criteria.length;
      return criteria.map(() => eq);
    }
    return normalizeWeights(weights);
  }, [criteria, weights]);

  return {
    alternatives,
    criteria,
    weights,

    addAlternative,
    removeAlternative,
    renameAlternative,
    setOverride,
    removeOverride,

    addCriterion,
    removeCriterion,
    setObjective,
    setWeight,
    resetWeights,

    validation,
    normalizedWeights,
    hasAlternatives: alternatives.length > 0,
    hasCriteria: criteria.length > 0,
  };
}
