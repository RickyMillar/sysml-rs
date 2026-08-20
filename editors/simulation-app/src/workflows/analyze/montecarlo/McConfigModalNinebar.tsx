/**
 * McConfigModalNinebar — the "Configure Monte Carlo" modal (ninebar
 * Phase 5).
 *
 * Plan §1 row 8c: "MC distribution editor = modal". Same shape as
 * `SweepConfigModalNinebar`: registered by id, every field writes the
 * shared `useMcStudyStore`, closing is enough. The per-parameter
 * distribution editing REUSES the existing `DistributionEditor`
 * (normal/uniform/triangular/custom-CDF + validation display) rather
 * than rebuilding it — it is fully props-driven.
 */

import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { registerModal } from '@/shared/overlays/modalStore';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { findElements } from '@/shared/api/model';
import { DistributionEditor } from './DistributionEditor';
import { useMcStudyStore, mcValidityByName, MC_MAX_SAMPLE_COUNT } from './useMcStudyStore';

export const MC_CONFIG_MODAL_ID = 'analyze-montecarlo-config';

const FIELD_LABEL: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  letterSpacing: '0.03em',
  textTransform: 'uppercase',
};

const FIELD_HINT: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-muted)',
};

const INPUT_STYLE: React.CSSProperties = {
  background: 'var(--surface-sunken)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 'var(--radius-sm)',
  padding: '4px 8px',
  fontSize: 'var(--text-sm)',
};

interface McCandidate {
  id: string;
  name: string;
  qualifier?: string;
}

/**
 * Same discovery as the legacy `MonteCarloWorkflow.discoverParameters`
 * (AttributeUsage names, name-deduped) — kept name-keyed because MC
 * overrides are applied by parameter NAME, matching the legacy body's
 * `distributions` map keys.
 */
async function discoverMcParameters(uris: readonly string[]): Promise<McCandidate[]> {
  if (uris.length === 0) return [];
  const seen = new Set<string>();
  const out: McCandidate[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name || seen.has(name)) continue;
      seen.add(name);
      const idx = uri.lastIndexOf('/');
      out.push({ id: `${uri}::${name}`, name, qualifier: idx >= 0 ? uri.slice(idx + 1) : uri });
    }
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

export function McConfigModalNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: availableParameters = [], isLoading } = useQuery<McCandidate[]>({
    queryKey: ['montecarlo-parameters', workspaceRoot, loadedUris],
    queryFn: () => discoverMcParameters(loadedUris),
    enabled: !!workspaceRoot && loadedUris.length > 0,
  });

  const distributions = useMcStudyStore((s) => s.distributions);
  const addParameter = useMcStudyStore((s) => s.addParameter);
  const removeParameter = useMcStudyStore((s) => s.removeParameter);
  const setDistributionKind = useMcStudyStore((s) => s.setDistributionKind);
  const setDistribution = useMcStudyStore((s) => s.setDistribution);
  const sampleCount = useMcStudyStore((s) => s.sampleCount);
  const setSampleCount = useMcStudyStore((s) => s.setSampleCount);
  const seed = useMcStudyStore((s) => s.seed);
  const setSeed = useMcStudyStore((s) => s.setSeed);

  const [pickerQuery, setPickerQuery] = useState('');
  const paramNames = Object.keys(distributions);
  const validity = useMemo(() => mcValidityByName(distributions), [distributions]);

  const candidates = useMemo<FuzzyCandidate[]>(
    () =>
      availableParameters
        .filter((c) => !distributions[c.name])
        .map((c) => ({ value: c.name, detail: c.qualifier })),
    [availableParameters, distributions],
  );

  const addPicked = () => {
    const name = pickerQuery.trim();
    if (!name) return;
    addParameter(name);
    setPickerQuery('');
  };

  return (
    <div data-testid="mc-config-modal" className="flex flex-col gap-4" style={{ minWidth: 420 }}>
      {/* ── Parameters ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Add parameter</span>
        <div className="flex gap-2">
          <div style={{ flex: 1 }}>
            <FuzzyCombobox
              value={pickerQuery}
              onChange={setPickerQuery}
              candidates={candidates}
              placeholder={isLoading ? 'Discovering parameters…' : 'Search attributes…'}
              testId="mc-modal-parameter-search"
              inputStyle={INPUT_STYLE}
            />
          </div>
          <button
            type="button"
            data-testid="mc-modal-parameter-add"
            onClick={addPicked}
            disabled={pickerQuery.trim().length === 0}
            style={{
              ...INPUT_STYLE,
              cursor: pickerQuery.trim().length === 0 ? 'not-allowed' : 'pointer',
              color: pickerQuery.trim().length === 0 ? 'var(--text-muted)' : 'var(--accent-fg)',
            }}
          >
            Add
          </button>
        </div>
        <span style={FIELD_HINT}>
          Each parameter gets a sampling distribution below (normal, uniform, triangular, or a
          custom CDF).
        </span>
      </div>

      {/* ── Distributions (reused editor) ── */}
      <div className="flex flex-col gap-2">
        <span style={FIELD_LABEL}>Distributions</span>
        {paramNames.length === 0 ? (
          <span data-testid="mc-modal-distributions-empty" style={FIELD_HINT}>
            No parameters yet — add one above to define its distribution.
          </span>
        ) : (
          <div className="flex flex-col gap-2" data-testid="mc-modal-distributions">
            {paramNames.map((name) => (
              <DistributionEditor
                key={name}
                paramName={name}
                distribution={distributions[name]!}
                isValid={validity[name] ?? false}
                onKindChange={(kind) => setDistributionKind(name, kind)}
                onChange={(next) => setDistribution(name, next)}
                onRemove={() => removeParameter(name)}
              />
            ))}
          </div>
        )}
      </div>

      {/* ── Samples + seed ── */}
      <div className="flex items-end gap-3">
        <label className="flex flex-col gap-1.5" style={{ width: 120 }}>
          <span style={FIELD_LABEL}>Samples</span>
          <input
            type="number"
            min={1}
            max={MC_MAX_SAMPLE_COUNT}
            data-testid="mc-modal-sample-count"
            value={sampleCount}
            onChange={(e) => setSampleCount(Number(e.target.value))}
            style={INPUT_STYLE}
          />
        </label>
        <label className="flex flex-col gap-1.5" style={{ width: 160 }}>
          <span style={FIELD_LABEL}>Seed (optional)</span>
          <input
            type="number"
            data-testid="mc-modal-seed"
            value={seed ?? ''}
            placeholder="backend picks"
            onChange={(e) => {
              const raw = e.target.value.trim();
              if (raw === '') {
                setSeed(null);
                return;
              }
              const n = Number(raw);
              if (Number.isFinite(n)) setSeed(n);
            }}
            style={INPUT_STYLE}
          />
        </label>
        <span style={{ ...FIELD_HINT, marginLeft: 'auto' }}>
          Sample cap {MC_MAX_SAMPLE_COUNT} (session quota guard).
        </span>
      </div>
      <span style={FIELD_HINT}>
        Changes apply immediately — close the modal and run from the rail.
      </span>
    </div>
  );
}

registerModal({
  id: MC_CONFIG_MODAL_ID,
  title: 'Configure Monte Carlo',
  component: McConfigModalNinebar,
});
