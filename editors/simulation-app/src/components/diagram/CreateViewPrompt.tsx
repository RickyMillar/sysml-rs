/**
 * CreateViewPrompt — the guided create-view flow, v2: PROJECTION-FIRST
 * (it previously asked for the target first).
 *
 * Users think in projections ("I want a state diagram") before targets,
 * so the flow asks:
 *   1. AS   — one of the renderer's REAL 8 kinds (cards with live
 *             availability badges: "2 state machines"; empty kinds dim);
 *   2. OF   — a picker SPECIALIZED to that kind: only valid targets,
 *             shown in their model hierarchy (ineligible ancestors are
 *             muted group headers), multi-select — each selection
 *             becomes one `expose` line. StateTransition rows list
 *             their states inline; Interconnection rows badge their
 *             port/connection counts.
 *   3. NAMED — identifier-validated, defaulted from the first pick.
 *
 * Mechanics are v1's, unchanged: element ids go to `POST /views/scratch`
 * (the backend owns spec-correct expose shaping), only the name/
 * supertype tokens are rewritten (loud failure otherwise), the def is
 * appended to the owning file's source BUFFER — dirty until saved in
 * the editor (§6 authoring loop). Filters and the live preview pane are
 * staged (v2c / v2b — preview waits on the `sysml.views.preview`
 * billet; see the design doc §4-5). No implicit default view, ever.
 */
import { useMemo, useState } from 'react';
import { useCreateScratchView } from '@/features/views/queries';
import { useSessionModelTree } from '@/features/sessions/tree/useSessionModelTree';
import { useWorkspaceStore } from '@/store/workspace';
import {
  VIEW_TYPES,
  buildScopeRows,
  defaultViewName,
  isValidViewName,
  kindAvailability,
  rewriteScratchSnippet,
  sourceTokenFor,
} from './createViewFlow';

const FIELD_LABEL: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  textTransform: 'uppercase',
  letterSpacing: '0.03em',
};

export function CreateViewPrompt({
  targetId,
  targetName,
  context = 'run',
}: {
  targetId: string;
  targetName?: string | null;
  /** 'run' = a live session's target has no view (the original 3.14 case);
   *  'browse' = no session — the workspace declares no views (W5) —
   *  the prefill target comes from the tree selection;
   *  'modal' = opened deliberately (Views rail "+ New" / Cmd-K) in a
   *  workspace that may already declare views. */
  context?: 'run' | 'browse' | 'modal';
}) {
  const create = useCreateScratchView();
  const updateSource = useWorkspaceStore((s) => s.updateSource);
  const focusFile = useWorkspaceStore((s) => s.focusFile);
  const loadedFiles = useWorkspaceStore((s) => s.loadedFiles);

  // Session-free model read — the flow works in Browse with no session.
  const { tree } = useSessionModelTree({ groupByPackage: false, expectedSessionId: null });

  const availability = useMemo(() => kindAvailability(tree), [tree]);

  // ── 1: projection ──
  const [chosenType, setChosenType] = useState('General');
  const scopeRows = useMemo(() => buildScopeRows(tree, chosenType), [tree, chosenType]);

  // ── 2: scope (multi-expose). Prefilled with the incoming target when
  // it is eligible under the chosen kind. Keyed by elementId. ──
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set());
  const prefillApplied = useMemo(() => {
    const eligibleIds = new Set(scopeRows.filter((r) => r.eligible).map((r) => r.node.elementId));
    if (selected.size > 0) return selected;
    if (targetId && eligibleIds.has(targetId)) return new Set([targetId]);
    // Name-based fallback (run context passes target ids that may not
    // match tree elementIds 1:1 across builds).
    if (targetName) {
      const byName = scopeRows.find((r) => r.eligible && r.node.name === targetName);
      if (byName) return new Set([byName.node.elementId]);
    }
    return selected;
  }, [selected, scopeRows, targetId, targetName]);

  const toggle = (elementId: string) => {
    const next = new Set(prefillApplied);
    if (next.has(elementId)) next.delete(elementId);
    else next.add(elementId);
    setSelected(next);
  };

  const pickedRows = scopeRows.filter((r) => prefillApplied.has(r.node.elementId));

  // ── 3: name ──
  const [nameInput, setNameInput] = useState<string | null>(null);
  const effectiveName =
    nameInput ?? defaultViewName(pickedRows[0]?.node.name ?? chosenType);
  const nameValid = isValidViewName(effectiveName);

  const [done, setDone] = useState<{ file: string; name: string } | null>(null);
  const [flowError, setFlowError] = useState<string | null>(null);

  const targetUri = pickedRows[0]?.node.uri ?? null;
  const crossFile = pickedRows.some((r) => r.node.uri !== targetUri);
  const canCreate = pickedRows.length > 0 && nameValid && !create.isPending;

  const onCreate = () => {
    if (!targetUri) return;
    setFlowError(null);
    create.mutate(pickedRows.map((r) => r.node.elementId), {
      onSuccess: async (snippet) => {
        const rewritten = rewriteScratchSnippet(snippet, effectiveName, sourceTokenFor(chosenType));
        if (!rewritten) {
          setFlowError(
            'The generated view snippet had an unexpected shape — nothing was written. ' +
              'This is a bug worth reporting; the raw snippet is in the console.',
          );
          console.error('[CreateViewPrompt] unrewritable scratch snippet:', snippet);
          return;
        }
        try {
          if (!loadedFiles.has(targetUri)) await focusFile(targetUri);
          const file = useWorkspaceStore.getState().loadedFiles.get(targetUri);
          if (!file) throw new Error(`source buffer for ${targetUri} did not load`);
          updateSource(targetUri, `${file.source.replace(/\s*$/, '')}\n\n${rewritten}\n`);
          await focusFile(targetUri);
          setDone({ file: targetUri, name: effectiveName });
        } catch (err) {
          setFlowError(`Couldn't write into the source buffer: ${String(err)}`);
        }
      },
    });
  };

  if (done) {
    return (
      <div
        data-testid="create-view-prompt"
        className="flex flex-col gap-2"
        style={{ padding: context === 'run' ? 24 : 0, maxWidth: 560, margin: context === 'run' ? '0 auto' : undefined }}
      >
        <div style={{ fontSize: 'var(--text-md)', fontWeight: 500, color: 'var(--text-primary)' }}>
          <code className="mono-text">{done.name}</code> added — unsaved
        </div>
        <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)', lineHeight: 1.5 }}>
          The view definition was appended to{' '}
          <code className="mono-text">{done.file.split('/').pop()}</code> (the buffer is
          marked dirty). Review and save it in the source editor — saving belongs to the
          editor, not the canvas. Once saved, pick the view in the Views rail to render it.
        </div>
        <button
          type="button"
          data-testid="create-view-another"
          onClick={() => {
            setDone(null);
            setSelected(new Set());
            setNameInput(null);
          }}
          style={{
            alignSelf: 'flex-start',
            background: 'none',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            color: 'var(--text-secondary)',
            padding: '4px 12px',
            fontSize: 'var(--text-xs)',
            cursor: 'pointer',
          }}
        >
          Create another
        </button>
      </div>
    );
  }

  return (
    <div
      data-testid="create-view-prompt"
      className="flex flex-col gap-3"
      style={{
        padding: context === 'run' ? 24 : 0,
        maxWidth: 560,
        margin: context === 'run' ? '0 auto' : undefined,
        color: 'var(--text-primary)',
      }}
    >
      {context === 'run' ? (
        <>
          <div style={{ fontSize: 'var(--text-md)', fontWeight: 500 }}>No declared view for this run</div>
          <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)', lineHeight: 1.5 }}>
            Nothing in the model exposes the run target yet — declare a view and the live
            overlay joins it. The time-series strip keeps working meanwhile.
          </div>
        </>
      ) : context === 'modal' ? (
        <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)', lineHeight: 1.5 }}>
          The definition is written into your model source, where it stays yours to edit.
        </div>
      ) : (
        <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)', lineHeight: 1.5 }}>
          This workspace declares no views yet. Author its first one — the definition is
          written into your model source, where it stays yours to edit.
        </div>
      )}

      {/* ── 1: projection (type-first, v2) ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>As</span>
        <div className="grid grid-cols-2 gap-1" role="radiogroup" aria-label="View type">
          {VIEW_TYPES.map((vt) => {
            const active = chosenType === vt.token;
            const count = availability[vt.token] ?? 0;
            return (
              <button
                key={vt.token}
                type="button"
                role="radio"
                aria-checked={active}
                data-testid={`create-view-type-${vt.token}`}
                onClick={() => {
                  setChosenType(vt.token);
                  setSelected(new Set()); // scope eligibility changes with the kind
                }}
                title={vt.blurb}
                className="flex flex-col items-start gap-0.5 text-left"
                style={{
                  padding: '6px 8px',
                  background: active ? 'var(--accent-tint)' : 'none',
                  border: `1px solid ${active ? 'var(--accent)' : 'var(--border-default)'}`,
                  borderRadius: 'var(--radius-sm)',
                  cursor: 'pointer',
                  opacity: count > 0 || active ? 1 : 0.5,
                }}
              >
                <span className="flex items-baseline gap-1.5" style={{ fontSize: 'var(--text-sm)', color: active ? 'var(--accent-fg)' : 'var(--text-primary)' }}>
                  {vt.label}
                  <span data-testid={`create-view-count-${vt.token}`} style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
                    {count > 0 ? count : '—'}
                  </span>
                </span>
                <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', lineHeight: 1.35 }}>
                  {vt.blurb}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      {/* ── 2: scope — specialized, hierarchical, multi-expose ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Of</span>
        <div
          data-testid="create-view-scope"
          className="flex flex-col overflow-y-auto"
          style={{
            maxHeight: 220,
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            padding: 4,
          }}
        >
          {scopeRows.length === 0 ? (
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', padding: '6px 8px' }}>
              No eligible elements for this projection in the loaded model.
            </span>
          ) : (
            scopeRows.map((row) => {
              const checked = prefillApplied.has(row.node.elementId);
              return row.eligible ? (
                <label
                  key={row.node.id}
                  data-testid={`create-view-scope-${row.node.elementId}`}
                  className="flex items-baseline gap-2"
                  style={{
                    padding: '3px 6px',
                    paddingLeft: 6 + row.depth * 14,
                    borderRadius: 'var(--radius-sm)',
                    background: checked ? 'var(--accent-tint)' : 'none',
                    cursor: 'pointer',
                  }}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => toggle(row.node.elementId)}
                    style={{ accentColor: 'var(--accent)' }}
                  />
                  <span className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
                    {row.node.name}
                  </span>
                  {row.hint && (
                    <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {row.hint}
                    </span>
                  )}
                </label>
              ) : (
                <div
                  key={row.node.id}
                  className="mono-text"
                  style={{
                    padding: '3px 6px',
                    paddingLeft: 6 + row.depth * 14,
                    fontSize: 'var(--text-xs)',
                    color: 'var(--text-muted)',
                  }}
                >
                  {row.node.name}
                </div>
              );
            })
          )}
        </div>
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
          Each selection becomes one <code className="mono-text">expose</code> line — exposing an
          element already projects its members, so children don't need separate ticks.
        </span>
      </div>

      {/* ── 3: name ── */}
      <label className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Named</span>
        <input
          data-testid="create-view-name"
          value={effectiveName}
          onChange={(e) => setNameInput(e.target.value)}
          className="mono-text"
          style={{
            background: 'var(--surface-sunken)',
            color: 'var(--text-primary)',
            border: `1px solid ${nameValid ? 'var(--border-default)' : 'var(--severity-error)'}`,
            borderRadius: 'var(--radius-sm)',
            padding: '5px 8px',
            fontSize: 'var(--text-sm)',
            width: 240,
          }}
        />
        {!nameValid && (
          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-error)' }}>
            Must be a legal identifier (letters, digits, underscore; not starting with a digit).
          </span>
        )}
      </label>

      {targetUri && (
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
          Will be appended to <code className="mono-text">{targetUri.split('/').pop()}</code> —
          unsaved until you save in the editor.
          {crossFile && ' Selections span files; expose references are fully qualified, so one home file is fine.'}
        </span>
      )}

      <button
        type="button"
        data-testid="create-view-button"
        onClick={onCreate}
        disabled={!canCreate}
        style={{
          alignSelf: 'flex-start',
          background: canCreate ? 'var(--accent)' : 'none',
          color: canCreate ? 'var(--text-inverse)' : 'var(--text-disabled)',
          border: canCreate ? 'none' : '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          padding: '6px 14px',
          fontSize: 'var(--text-sm)',
          fontWeight: 500,
          cursor: canCreate ? 'pointer' : 'not-allowed',
        }}
      >
        {create.isPending
          ? 'Creating…'
          : pickedRows.length > 1
            ? `Create view (${pickedRows.length} exposes)`
            : 'Create view'}
      </button>

      {(create.isError || flowError) && (
        <div data-testid="create-view-error" style={{ fontSize: 'var(--text-sm)', color: 'var(--severity-error)' }}>
          {flowError ?? `Couldn't generate the view: ${String(create.error)}`}
        </div>
      )}
    </div>
  );
}
