/**
 * StaticVerifyModal — the "Verify workspace (static)" modal body,
 * registered under id `readiness-static-verify` (ninebar Phase 1.5).
 *
 * Fires `sysml.workspace.verify` (`useWorkspaceVerify`,
 * `features/packages/queries.ts`) on open — cross-file merge + full
 * verification-case evaluation, NO session involved. This is a
 * *validation* act, distinct from Phase 4's live verdict matrix (which
 * is per-run evidence): both `ModalHost`'s title (registered below) and
 * this body's own persistent inline label read "Static / pre-run" so a
 * screenshot of this modal is never mistaken for live run evidence
 * (plan requirement).
 *
 * Reuses `VerdictBadge` — the app's one canonical verdict-rendering
 * component — for the pass/fail summary. Backend finding: the wire
 * response (`{total_cases, passed, failed, elapsed_ms, per_file}`) only
 * carries aggregate counts + the set of files that had a failing case,
 * not a per-case verdict list, so this modal renders one summary badge
 * plus the affected-file list rather than a per-case matrix — a finer
 * drill would need a backend shape change (see report).
 *
 * Loading state is the `<Ninebar/>` indeterminate glyph — a genuine
 * pending measure (the verification run itself), never decorative.
 */
import { useEffect } from 'react';
import { Ninebar } from '@/components/Ninebar';
import { VerdictBadge, type VerdictKind } from '@/components/VerdictBadge';
import { useWorkspaceVerify } from '@/features/packages/queries';
import { registerModal } from '@/shared/overlays/modalStore';

export const READINESS_STATIC_VERIFY_MODAL_ID = 'readiness-static-verify';
export const STATIC_PRE_RUN_LABEL = 'Static / pre-run';

function overallVerdict(passed: number, failed: number, totalCases: number): VerdictKind {
  if (failed > 0) return 'fail';
  if (totalCases === 0) return 'inconclusive';
  return 'pass';
}

export function StaticVerifyModal() {
  const verify = useWorkspaceVerify();

  // The mutation IS the action here — there's no cached query to read
  // on mount, so the modal fires it once as soon as it opens.
  useEffect(() => {
    verify.mutate(undefined);
    // Intentionally fire-once on mount; `verify.mutate` is stable
    // across renders (react-query mutation identity), and the footer
    // "Re-run" button covers the repeat case explicitly.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const data = verify.data;

  return (
    <div data-testid="static-verify-modal-body" className="flex flex-col gap-3">
      <div
        data-testid="static-verify-label"
        style={{
          display: 'inline-flex',
          alignSelf: 'flex-start',
          alignItems: 'center',
          padding: '2px 8px',
          fontSize: 'var(--text-xs)',
          fontWeight: 600,
          letterSpacing: '0.03em',
          textTransform: 'uppercase',
          color: 'var(--text-muted)',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
        }}
      >
        {STATIC_PRE_RUN_LABEL}
      </div>

      {verify.isPending && (
        <div className="flex items-center gap-2" data-testid="static-verify-loading">
          <Ninebar label="running static verification" />
          <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}>
            Running cross-file verification…
          </span>
        </div>
      )}

      {verify.isError && (
        <div
          data-testid="static-verify-error"
          role="alert"
          style={{ fontSize: 'var(--text-sm)', color: 'var(--severity-error)' }}
        >
          {verify.error instanceof Error ? verify.error.message : 'Verification failed'}
        </div>
      )}

      {data && (
        <div data-testid="static-verify-result" className="flex flex-col gap-3">
          <div className="flex items-center gap-3">
            <VerdictBadge verdict={overallVerdict(data.passed, data.failed, data.total_cases)} />
            <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}>
              {data.passed}/{data.total_cases} cases passed
              {data.failed > 0 ? ` · ${data.failed} failed` : ''}
            </span>
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
              {(data.elapsed_ms / 1000).toFixed(2)}s
            </span>
          </div>

          {data.per_file.length > 0 && (
            <div>
              <div
                style={{
                  fontSize: 'var(--text-xs)',
                  color: 'var(--text-muted)',
                  marginBottom: 4,
                }}
              >
                Files with failing cases
              </div>
              <ul className="flex flex-col">
                {data.per_file.map((file) => (
                  <li
                    key={file}
                    data-testid="static-verify-file-row"
                    style={{
                      height: 'var(--row-dense)',
                      display: 'flex',
                      alignItems: 'center',
                      fontFamily: 'var(--font-mono)',
                      fontSize: 'var(--text-xs)',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    {file}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}

      <button
        type="button"
        data-testid="static-verify-rerun"
        onClick={() => verify.mutate(undefined)}
        disabled={verify.isPending}
        style={{
          alignSelf: 'flex-start',
          height: 'var(--row-compact)',
          padding: '0 10px',
          fontSize: 'var(--text-sm)',
          color: 'var(--text-primary)',
          background: 'transparent',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          cursor: verify.isPending ? 'default' : 'pointer',
        }}
      >
        Re-run
      </button>
    </div>
  );
}

registerModal({
  id: READINESS_STATIC_VERIFY_MODAL_ID,
  title: `${STATIC_PRE_RUN_LABEL} workspace verification`,
  component: StaticVerifyModal,
});
