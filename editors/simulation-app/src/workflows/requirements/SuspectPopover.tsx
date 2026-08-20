/**
 * Suspect popover (demo 1c §6) — anchored overlay explaining WHY a row
 * is flagged ⚑ against the selected baseline.
 *
 * Anatomy: beak-anchored 420px card (12px radius per the ninebar
 * overlay ruling) → header "⚑ Changed since baseline ⟨B⟩" + row id →
 * diff excerpt (− removed struck/dimmed, + added; gutters use the
 * DIFF token family, never verdict colours) → downstream-impact list
 * (verification results + derived requirements, from row link refs) →
 * LIVE attestation actions (v1.5b): "Attest unchanged intent" records
 * a SuspectClearingAttestation pinned to the current content commit —
 * the flag drops until the requirement changes again; "Re-verify…"
 * reruns workspace verification (deliberately NO workflow event —
 * suspect state is computed, not attested).
 *
 * Actor identity is an explicit one-time setting (never an OS-username
 * default — the backend hard-rejects blank actors); the popover
 * collects it inline before the first attestation.
 *
 * ADR-009 honesty (binding): diff correlation is id-strict. A row whose
 * identity changed (removed+added, e.g. after a scope rename) is
 * presented as "requirement replaced (identity changed)" — the UI never
 * name-matches the two sides into a fake before/after.
 */

import { useState } from 'react';
import type { SuspectRecord } from '@/features/baselines/suspect';
import { useAttestClearing, useReverify } from '@/features/workflow/queries';
import { ActorGate, WORKFLOW_INPUT_STYLE } from '@/features/workflow/ActorGate';
import type { RequirementRow } from '@/features/requirements/types';
import { rowDisplayId } from '@/features/requirements/rollup';

export function SuspectPopover({
  row,
  record,
  baselineName,
  onClose,
}: {
  row: RequirementRow;
  record: SuspectRecord;
  baselineName: string;
  onClose: () => void;
}) {
  const impactedVerifications = row.verified_by.length;
  const impactedDerived = row.derives.length;

  return (
    <div
      data-testid="suspect-popover"
      role="dialog"
      aria-label={`Changed since baseline ${baselineName}`}
      style={{
        position: 'absolute',
        top: 'calc(100% + 8px)',
        right: 8,
        width: 420,
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-lg)',
        boxShadow: 'var(--shadow-float)',
        zIndex: 30,
        fontFamily: 'var(--font-body)',
      }}
    >
      {/* beak pointing at the flag */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: -5,
          right: 22,
          width: 8,
          height: 8,
          background: 'var(--surface-overlay)',
          borderLeft: '1px solid var(--border-default)',
          borderTop: '1px solid var(--border-default)',
          transform: 'rotate(45deg)',
        }}
      />

      {/* header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '10px 12px',
          borderBottom: '1px solid var(--border-hairline)',
          fontSize: 'var(--text-sm)',
        }}
      >
        <span aria-hidden style={{ color: 'var(--severity-warning)' }}>⚑</span>
        <span style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
          Changed since baseline {baselineName}
        </span>
        <span style={{ flex: 1 }} />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-xs)',
            color: 'var(--text-muted)',
          }}
        >
          {rowDisplayId(row)}
        </span>
        <button
          type="button"
          aria-label="close"
          data-testid="suspect-popover-close"
          onClick={onClose}
          style={{
            border: 'none',
            background: 'transparent',
            color: 'var(--text-muted)',
            cursor: 'pointer',
            padding: 0,
            fontSize: 'var(--text-sm)',
            lineHeight: 1,
          }}
        >
          ✕
        </button>
      </div>

      {/* change detail */}
      <div
        style={{
          padding: '10px 12px',
          borderBottom: '1px solid var(--border-hairline)',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          lineHeight: 1.6,
        }}
      >
        {record.kind === 'identity-changed' ? (
          <div data-testid="suspect-identity-changed" style={{ color: 'var(--text-secondary)' }}>
            Requirement replaced (identity changed since {baselineName}). The
            baseline diff correlates strictly by element identity — renaming a
            containing scope regenerates ids, so no before/after text can be
            honestly shown.
          </div>
        ) : record.textDeltas.length > 0 || record.propDeltas.length > 0 ? (
          <>
            {record.textDeltas.map((delta, i) => (
              <div key={i} data-testid="suspect-text-delta">
                {delta.from !== null && (
                  <div style={{ display: 'flex', gap: 8 }}>
                    <span aria-hidden style={{ color: 'var(--diff-removed)' }}>−</span>
                    <span
                      style={{
                        color: 'var(--text-muted)',
                        textDecoration: 'line-through',
                      }}
                    >
                      {delta.from}
                    </span>
                  </div>
                )}
                {delta.to !== null && (
                  <div style={{ display: 'flex', gap: 8 }}>
                    <span aria-hidden style={{ color: 'var(--diff-added)' }}>+</span>
                    <span style={{ color: 'var(--text-primary)' }}>{delta.to}</span>
                  </div>
                )}
              </div>
            ))}
            {/* other scalar prop edits (W4): constraint bodies, attribute
                values — labeled by prop key, kind on hover */}
            {record.propDeltas.map((delta, i) => (
              <div key={i} data-testid="suspect-prop-delta" title={delta.elementKind}>
                <div style={{ color: 'var(--text-muted)' }}>{delta.key}</div>
                <div style={{ display: 'flex', gap: 8 }}>
                  <span aria-hidden style={{ color: 'var(--diff-removed)' }}>−</span>
                  <span
                    style={{
                      color: 'var(--text-muted)',
                      textDecoration: 'line-through',
                    }}
                  >
                    {delta.from}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: 8 }}>
                  <span aria-hidden style={{ color: 'var(--diff-added)' }}>+</span>
                  <span style={{ color: 'var(--text-primary)' }}>{delta.to}</span>
                </div>
              </div>
            ))}
          </>
        ) : (
          <div data-testid="suspect-nontext-change" style={{ color: 'var(--text-secondary)' }}>
            {record.changeSummary}
          </div>
        )}
      </div>

      {/* downstream impact */}
      <div
        style={{
          padding: '8px 12px',
          borderBottom: '1px solid var(--border-hairline)',
        }}
      >
        <div
          style={{
            fontSize: 'var(--text-xs)',
            color: 'var(--text-muted)',
            textTransform: 'lowercase',
            marginBottom: 4,
          }}
        >
          downstream impact
        </div>
        <ImpactLine
          count={impactedVerifications}
          label="verification results to re-check"
          detail={row.verified_by.map((r) => r.name ?? r.id).join(' · ')}
        />
        <ImpactLine
          count={impactedDerived}
          label="derived requirements"
          detail={row.derives.map((r) => r.name ?? r.id).join(' · ')}
        />
        {impactedVerifications === 0 && impactedDerived === 0 && (
          <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
            no linked verifications or derived requirements
          </div>
        )}
      </div>

      <AttestationSection row={row} baselineName={baselineName} onCleared={onClose} />
    </div>
  );
}

/** Live attestation actions (v1.5b). */
function AttestationSection({
  row,
  baselineName,
  onCleared,
}: {
  row: RequirementRow;
  baselineName: string;
  onCleared: () => void;
}) {
  const [rationale, setRationale] = useState('');
  const attest = useAttestClearing();
  const reverify = useReverify();

  return (
    <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8 }}>
      <ActorGate prompt="Attestations are signed">
        {(actor) => (
          <>
            <input
              data-testid="suspect-rationale-input"
              value={rationale}
              placeholder={`reason — recorded with your signature and the ${baselineName} hash`}
              onChange={(e) => setRationale(e.target.value)}
              style={WORKFLOW_INPUT_STYLE}
            />
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <ActionButton
                testid="suspect-reverify"
                label={reverify.isPending ? 'verifying…' : 'Re-verify…'}
                disabled={reverify.isPending}
                onClick={() => reverify.mutate()}
              />
              <ActionButton
                testid="suspect-attest"
                label={attest.isPending ? 'attesting…' : 'Attest unchanged intent'}
                disabled={attest.isPending || rationale.trim() === ''}
                title={
                  rationale.trim() === ''
                    ? 'A rationale is required — it is recorded in the audit trail'
                    : undefined
                }
                onClick={() =>
                  attest.mutate(
                    {
                      elementId: row.id,
                      baseline: baselineName,
                      rationale: rationale.trim(),
                      actor,
                    },
                    { onSuccess: onCleared },
                  )
                }
              />
              <span
                style={{
                  flex: 1,
                  fontSize: 'var(--text-xs)',
                  color: 'var(--text-muted)',
                  textAlign: 'right',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                as {actor}
              </span>
            </div>
            {attest.isError && (
              <div
                data-testid="suspect-attest-error"
                style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-error)' }}
              >
                Attestation failed: {attest.error instanceof Error ? attest.error.message : 'unknown error'}
              </div>
            )}
          </>
        )}
      </ActorGate>
    </div>
  );
}

function ActionButton({
  label,
  testid,
  disabled,
  title,
  onClick,
}: {
  label: string;
  testid: string;
  disabled: boolean;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={testid}
      disabled={disabled}
      title={title}
      onClick={onClick}
      style={{
        height: 26,
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-sm)',
        background: 'transparent',
        color: disabled ? 'var(--text-muted)' : 'var(--text-primary)',
        fontSize: 'var(--text-xs)',
        padding: '0 10px',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
      }}
    >
      {label}
    </button>
  );
}

function ImpactLine({
  count,
  label,
  detail,
}: {
  count: number;
  label: string;
  detail: string;
}) {
  if (count === 0) return null;
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'baseline',
        gap: 8,
        fontSize: 'var(--text-xs)',
        lineHeight: '20px',
      }}
    >
      <span aria-hidden style={{ color: 'var(--severity-warning)' }}>⚑</span>
      <span style={{ color: 'var(--text-primary)' }}>
        {count} {label}
      </span>
      <span
        style={{
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-mono)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {detail}
      </span>
    </div>
  );
}

