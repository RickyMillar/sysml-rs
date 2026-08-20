/**
 * SessionStatusBar -- Zone 4: bottom-edge status bar.
 *
 * Layout (per simulation-ui-endgame.md "Zone 4: Status Bar"):
 *   Left:  model counts (files, SMs, ODEs, flows, constraints, tests)
 *   Mid:   API health dot + session quota (active/max)
 *   Right: live session status (t=X.Xs step NNNN, phase badge)
 *
 * Backend health polled every 5s; quota at 0.5 Hz (per contract).
 * Right-side time/step display throttled to 500ms (ADR-008).
 */

import { useRef, useState, useEffect } from 'react';
import { useModelCapabilities } from '@/hooks/useModelCapabilities';
import { useWorkspaceStore } from '@/store/workspace';
import { useSessionStore } from './store';
import { useSessionDetail, useBackendHealth, useSessionQuota } from './queries';

// ── Separator ────────────────────────────────────────────────────────

function Sep() {
  return (
    <span
      style={{
        width: 1,
        height: 12,
        background: 'var(--border-default)',
        opacity: 0.5,
        flexShrink: 0,
      }}
    />
  );
}

// ── Health Dot ───────────────────────────────────────────────────────

function HealthDot({ healthy }: { healthy: boolean }) {
  return (
    <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: healthy ? 'var(--verdict-pass)' : 'var(--verdict-fail)',
          display: 'inline-block',
          flexShrink: 0,
        }}
      />
      <span style={{ color: healthy ? 'var(--verdict-pass)' : 'var(--verdict-fail)' }}>
        API
      </span>
    </span>
  );
}

// ── Component ────────────────────────────────────────────────────────

export function SessionStatusBar() {
  const phase = useSessionStore((s) => s.phase);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const { data: detail } = useSessionDetail(activeSessionId);

  const caps = useModelCapabilities();
  const fileCount = useWorkspaceStore((s) => s.loadedFiles.size);

  // Backend health (poll 5s)
  const { data: backendHealthy } = useBackendHealth();
  const isHealthy = backendHealthy ?? false;

  // Session quota (poll 2s)
  const { data: quota } = useSessionQuota();
  const totalUsed = quota
    ? (quota.simulation?.used ?? 0) + (quota.action?.used ?? 0) + (quota.orchestrator?.used ?? 0)
    : 0;
  const totalCap = quota
    ? (quota.simulation?.cap ?? 0) + (quota.action?.cap ?? 0) + (quota.orchestrator?.cap ?? 0)
    : 0;

  // ── Throttled time/step (500ms per ADR-008) ────────────────────────

  const rawTick = detail?.summary?.tick ?? 0;
  const rawTimeMs = detail?.summary?.time_ms ?? 0;
  const completed = detail?.summary?.completed ?? false;

  const [displayTick, setDisplayTick] = useState(0);
  const [displayTimeMs, setDisplayTimeMs] = useState(0);
  const lastUpdateRef = useRef(0);

  useEffect(() => {
    const now = Date.now();
    if (phase !== 'running' || now - lastUpdateRef.current >= 500) {
      setDisplayTick(rawTick);
      setDisplayTimeMs(rawTimeMs);
      lastUpdateRef.current = now;
    }
  }, [rawTick, rawTimeMs, phase]);

  // ── Model counts (left side) ───────────────────────────────────────

  const countParts: string[] = [];
  if (fileCount > 0) countParts.push(`${fileCount} file${fileCount !== 1 ? 's' : ''}`);
  if (caps.smCount > 0) countParts.push(`${caps.smCount} SM${caps.smCount !== 1 ? 's' : ''}`);
  if (caps.odeCount > 0) countParts.push(`${caps.odeCount} ODE${caps.odeCount !== 1 ? 's' : ''}`);
  if (caps.flowCount > 0) countParts.push(`${caps.flowCount} flow${caps.flowCount !== 1 ? 's' : ''}`);
  if (caps.constraintCount > 0) countParts.push(`${caps.constraintCount} constraint${caps.constraintCount !== 1 ? 's' : ''}`);
  if (caps.analysisCaseCount > 0) countParts.push(`${caps.analysisCaseCount} test${caps.analysisCaseCount !== 1 ? 's' : ''}`);

  return (
    <div
      data-testid="status-bar"
      className="flex items-center px-4 gap-4 shrink-0"
      style={{
        height: 'var(--statusbar-height, 24px)',
        background: 'var(--surface-sunken)',
        fontSize: 'var(--text-xs, 11px)',
        color: 'var(--text-muted)',
        borderTop: '1px solid var(--border-default)',
        fontFamily: 'var(--font-mono, "JetBrains Mono", "Fira Code", "Cascadia Code", monospace)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {/* Left: model counts */}
      {countParts.length > 0 && (
        <span>{countParts.join(' \u00b7 ')}</span>
      )}

      <Sep />

      {/* Mid: API health + quota */}
      <HealthDot healthy={isHealthy} />

      {quota && (
        <SessionCounter
          used={totalUsed}
          cap={totalCap}
          breakdown={{
            simulation: quota.simulation ?? null,
            action: quota.action ?? null,
            orchestrator: quota.orchestrator ?? null,
          }}
        />
      )}

      <div className="flex-1" />

      {/* Right: session status */}
      {phase !== 'idle' && (
        <>
          {displayTick > 0 && (
            <span>
              t={formatTime(displayTimeMs)} step {displayTick}
            </span>
          )}

          <span
            style={{
              textTransform: 'uppercase',
              fontWeight: 600,
              letterSpacing: '0.04em',
              color: completed
                ? 'var(--sim-state-completed)'
                : phase === 'running'
                  ? 'var(--sim-state-active)'
                  : phase === 'error'
                    ? 'var(--severity-error)'
                    : 'var(--text-muted)',
            }}
          >
            {completed ? 'DONE' : phase}
          </span>
        </>
      )}
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────────

/** Format time_ms to a compact string like "1.4s" or "0.0s". */
function formatTime(ms: number): string {
  return `${(ms / 1000).toFixed(1)}s`;
}

// ── Session counter (P7) ─────────────────────────────────────────────

interface QuotaBucket {
  used?: number;
  cap?: number;
}

/**
 * Click-aware session counter chip.
 *
 * P7: replaces the old `{used}/{cap} sessions` label with `N active ·
 * M free`. Clicking toggles a compact popover that lists the per-kind
 * breakdown (simulation / action / orchestrator) and surfaces the
 * 30 s reaper cadence so the user knows the counter will self-correct
 * shortly. The full stop-per-session / archive / "reap now" controls
 * per the UX plan will land alongside the right-rail session manager —
 * this chip is the entry point.
 */
export function SessionCounter({
  used,
  cap,
  breakdown,
}: {
  used: number;
  cap: number;
  breakdown: Record<string, QuotaBucket | null>;
}) {
  const [open, setOpen] = useState(false);
  const free = Math.max(0, cap - used);
  const label = cap > 0 ? `${used} active · ${free} free` : `${used} active`;

  return (
    <span
      style={{ position: 'relative', display: 'inline-flex', alignItems: 'center' }}
    >
      <button
        type="button"
        data-testid="session-counter"
        data-used={used}
        data-cap={cap}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((prev) => !prev)}
        title="Click for per-kind breakdown"
        style={{
          border: 'none',
          background: 'transparent',
          color: 'inherit',
          font: 'inherit',
          padding: 0,
          cursor: 'pointer',
          opacity: 0.8,
        }}
      >
        {label}
      </button>
      {open && (
        <div
          data-testid="session-counter-popover"
          role="dialog"
          style={{
            position: 'absolute',
            bottom: 'calc(100% + 6px)',
            left: 0,
            minWidth: 200,
            padding: '8px 10px',
            background: 'var(--surface-raised)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-default)',
            borderRadius: 4,
            boxShadow: '0 6px 16px rgba(0,0,0,0.22)',
            fontSize: 11,
            zIndex: 40,
            fontFamily: 'var(--font-sans, inherit)',
          }}
        >
          <div
            style={{
              fontSize: 10,
              textTransform: 'uppercase',
              letterSpacing: '0.06em',
              fontWeight: 600,
              color: 'var(--text-muted)',
              marginBottom: 4,
            }}
          >
            Sessions
          </div>
          <div
            className="mono-text"
            style={{ fontSize: 11, lineHeight: 1.6, color: 'var(--text-primary)' }}
          >
            {Object.entries(breakdown).map(([kind, bucket]) =>
              bucket ? (
                <div
                  key={kind}
                  data-testid={`session-counter-row-${kind}`}
                  style={{ display: 'flex', gap: 8, justifyContent: 'space-between' }}
                >
                  <span>{kind}</span>
                  <span style={{ color: 'var(--text-muted)' }}>
                    {bucket.used ?? 0} / {bucket.cap ?? 0}
                  </span>
                </div>
              ) : null,
            )}
          </div>
          <div
            style={{
              marginTop: 6,
              paddingTop: 6,
              borderTop: '1px solid var(--border-default)',
              color: 'var(--text-muted)',
              fontSize: 10,
              lineHeight: 1.4,
            }}
          >
            Stopped sessions auto-reap every 30 s. Use the Archive tool
            to recover completed runs.
          </div>
        </div>
      )}
    </span>
  );
}
