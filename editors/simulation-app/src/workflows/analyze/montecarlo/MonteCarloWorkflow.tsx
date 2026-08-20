/**
 * MonteCarloWorkflow — route /analyze/montecarlo (R5.6).
 *
 * Two-column layout:
 *
 *   ┌──────────────────┬──────────────────────────────┐
 *   │ MonteCarloConfig │ MonteCarloResultsShell       │
 *   │  • param picker  │  (filled by R5.7 viewers)    │
 *   │  • dist editors  │                              │
 *   │  • count + seed  │                              │
 *   │  • Run button    │                              │
 *   └──────────────────┴──────────────────────────────┘
 *
 * This file owns:
 *   - Discovery of the candidate parameter list (AttributeUsage names
 *     surfaced from the loaded workspace).
 *   - Config state via `useMonteCarloConfig`.
 *   - The batch runner via `useMonteCarloRunner`.
 *
 * Viewers (histograms, percentile bands, pass-rate dashboard, CSV
 * export) are FF/FF2's territory and land in R5.7 — they read from
 * `{ batchId, children }` on the results shell.
 */

import { useCallback, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { findElements } from '@/shared/api/model';
import { EmbeddedDiagram } from '@/components/diagram/EmbeddedDiagram';
import { MonteCarloConfig, type McParameterCandidate } from './MonteCarloConfig';
import { MonteCarloResultsShell } from './MonteCarloResultsShell';
import { MonteCarloWorkflowNinebar } from './MonteCarloWorkflowNinebar';
import { useMonteCarloConfig } from './useMonteCarloConfig';
import { useMonteCarloRunner } from './useMonteCarloRunner';
import { isFlagEnabled } from '@/featureFlags';

// ── Parameter discovery ─────────────────────────────────────────────

/**
 * Walk every loaded URI in the workspace and collect AttributeUsage
 * elements. Each one is a candidate override parameter for the Monte
 * Carlo batch.
 *
 * Failure mode: if `findElements` throws for any uri we skip it and
 * keep going — a broken file shouldn't nuke the whole parameter list.
 */
async function discoverParameters(
  uris: readonly string[],
): Promise<McParameterCandidate[]> {
  if (uris.length === 0) return [];

  const seen = new Set<string>();
  const out: McParameterCandidate[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name) continue;
      if (seen.has(name)) continue;
      seen.add(name);
      out.push({
        id: `${uri}::${name}`,
        name,
        qualifier: shortenUri(uri),
      });
    }
  }
  // Stable alphabetical — parameters rarely have an inherent order and
  // alphabetic is the least-surprising listing for a multi-select.
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

function shortenUri(uri: string): string {
  // Strip everything up to the last "/" for a compact display qualifier.
  const idx = uri.lastIndexOf('/');
  return idx >= 0 ? uri.slice(idx + 1) : uri;
}

// ── Workflow ────────────────────────────────────────────────────────

/**
 * Route entry for /analyze/montecarlo. Under the (default-on) `ninebar`
 * flag the surface is the re-composed viewer-hero body
 * (`MonteCarloWorkflowNinebar`); flag-off keeps the legacy two-column
 * body verbatim (deleted in Phase 8 per F17).
 */
export function MonteCarloWorkflow() {
  if (isFlagEnabled('ninebar')) return <MonteCarloWorkflowNinebar />;
  return <MonteCarloWorkflowLegacy />;
}

function MonteCarloWorkflowLegacy() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);

  const { data: availableParameters = [], isLoading: isLoadingParameters } =
    useQuery<McParameterCandidate[]>({
      queryKey: ['montecarlo-parameters', workspaceRoot, loadedUris],
      queryFn: () => discoverParameters(loadedUris),
      enabled: !!workspaceRoot && loadedUris.length > 0,
    });

  const config = useMonteCarloConfig();
  const runner = useMonteCarloRunner();

  const handleRun = useCallback(() => {
    if (!config.isValid) return;
    if (loadedUris.length === 0) return;
    // Backend `sysml.batch.create` requires a concrete `uri` (not
    // optional). Prefer the URI that owns the first selected parameter
    // so overrides land in the right graph; fall back to the first
    // loaded workspace URI.
    const firstSelectedKey = Object.keys(config.distributions)[0];
    const ownerUri =
      firstSelectedKey && firstSelectedKey.includes('::')
        ? firstSelectedKey.split('::')[0]
        : loadedUris[0];
    void runner.run({
      workspaceRoot,
      uri: ownerUri,
      distributions: config.distributions,
      count: config.sampleCount,
      seed: config.seed,
    });
  }, [config.distributions, config.isValid, config.sampleCount, config.seed, loadedUris, runner, workspaceRoot]);

  const isRunning =
    runner.state === 'creating' || runner.state === 'running';

  return (
    <div
      data-testid="montecarlo-workflow"
      className="flex flex-row h-full w-full overflow-hidden"
    >
      <MonteCarloConfig
        availableParameters={availableParameters}
        config={config}
        isRunning={isRunning}
        hasWorkspace={!!workspaceRoot}
        isLoadingParameters={isLoadingParameters}
        onRun={handleRun}
      />
      <main
        data-testid="montecarlo-results"
        className="flex-1 overflow-hidden"
        style={{ background: 'var(--surface)' }}
      >
        <MonteCarloResultsShell
          batchId={runner.batchId}
          children={runner.children}
          state={runner.state}
          completed={runner.completed}
          total={runner.total}
          error={runner.error}
        />
      </main>
      {/* Phase 6 — diagram on every workflow tab. */}
      <EmbeddedDiagram label="Model" />
    </div>
  );
}
