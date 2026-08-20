/**
 * WorkflowStub — shared "coming soon" card used by stub workflows.
 *
 * Each workflow that hasn't landed yet (Verify, Sweep, MonteCarlo,
 * TradeStudy) mounts this card so the route is presentable even
 * before its real UX ships. Props describe the workflow; copy comes
 */

import type { ReactNode } from 'react';
import { useSessionStore } from '@/features/sessions/store';

export interface WorkflowStubProps {
  /** Stable slug, used for the data-testid (`workflow-stub-<slug>`). */
  slug: string;
  /** Page title. */
  title: string;
  /** One-sentence description — what the workflow will do. */
  description: string;
  /** Round badge — e.g. "Round 3", "Round 5". */
  round: string;
  /** Material Symbols icon name. */
  icon: string;
  /**
   * Anchor deep-link into the ux-overhaul plan for "Learn more".
   * Default points at the Round section; override for sub-routes.
   */
  learnMoreHref?: string;
  /** Optional extra children rendered below the main card. */
  children?: ReactNode;
}

const PLAN_HREF =
  'https://github.com/RickyMillar/sysml-rs';

export function WorkflowStub({
  slug,
  title,
  description,
  round,
  icon,
  learnMoreHref = PLAN_HREF,
  children,
}: WorkflowStubProps) {
  // Session-aware indicator: shows whether a live session from another
  // workflow is parked on the backend. Confirms the session store's
  // globalness after the route refactor.
  const activeSessionId = useSessionStore((s) => s.activeSessionId);

  return (
    <div
      data-testid={`workflow-stub-${slug}`}
      className="flex flex-col items-center justify-center h-full w-full overflow-auto px-8 py-12"
      style={{ background: 'var(--surface)' }}
    >
      <div
        className="flex flex-col items-center gap-4 rounded-lg"
        style={{
          maxWidth: 520,
          padding: '40px 32px',
          background: 'var(--surface-container-low)',
          border: '1px solid var(--outline-variant)',
          boxShadow: '0 1px 2px rgba(0,0,0,0.04), 0 4px 16px rgba(0,0,0,0.08)',
        }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 56, color: 'var(--primary)', opacity: 0.85 }}
        >
          {icon}
        </span>

        <h1
          style={{
            fontSize: 22,
            fontWeight: 600,
            color: 'var(--on-surface)',
            letterSpacing: '-0.01em',
            margin: 0,
          }}
        >
          {title}
        </h1>

        <span
          data-testid={`workflow-stub-${slug}-badge`}
          className="uppercase tracking-wider"
          style={{
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: '0.08em',
            padding: '4px 10px',
            borderRadius: 999,
            background: 'var(--primary-container)',
            color: 'var(--on-primary-container)',
          }}
        >
          Coming in {round}
        </span>

        <p
          style={{
            fontSize: 14,
            lineHeight: 1.6,
            color: 'var(--outline)',
            textAlign: 'center',
            margin: 0,
          }}
        >
          {description}
        </p>

        <a
          href={learnMoreHref}
          target="_blank"
          rel="noreferrer"
          className="rounded transition-colors"
          style={{
            fontSize: 13,
            fontWeight: 500,
            color: 'var(--primary)',
            textDecoration: 'none',
            padding: '6px 12px',
            border: '1px solid var(--outline-variant)',
          }}
        >
          Learn more →
        </a>

        {activeSessionId ? (
          <div
            data-testid={`workflow-stub-${slug}-active-session`}
            className="mono-text"
            style={{
              fontSize: 11,
              color: 'var(--outline)',
              marginTop: 12,
              padding: '6px 10px',
              borderRadius: 6,
              background: 'var(--surface-container-high)',
            }}
          >
            Active session: {activeSessionId}
          </div>
        ) : null}

        {children}
      </div>
    </div>
  );
}
