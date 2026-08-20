/**
 * AnalyzeWorkflowNinebar — the flag-on Analyze shell (ninebar Phase 5).
 *
 * "Analyze, re-composed": the shell is a 34px method-tab row (the same
 * quiet SubViewTab treatment Verify's sub-views use — router Links, not
 * local state, because each method is its own route per the plan §3
 * Phase 5 layout decision) over the routed method body. Everything else
 * — config modals, rail content, strip progress — belongs to the
 * per-method bodies (`SweepWorkflowNinebar` etc.), not this shell.
 *
 * The legacy mode-tab bar (`AnalyzeWorkflow` flag-off) is untouched.
 */

import { Link, Outlet, useLocation } from 'react-router-dom';

interface MethodTab {
  id: string;
  path: string;
  label: string;
}

const METHODS: MethodTab[] = [
  { id: 'cases', path: '/analyze', label: 'Cases' },
  { id: 'sweep', path: '/analyze/sweep', label: 'Sweep' },
  { id: 'montecarlo', path: '/analyze/montecarlo', label: 'Monte Carlo' },
  { id: 'trade-study', path: '/analyze/trade-study', label: 'Trade Study' },
  { id: 'sensitivity', path: '/analyze/sensitivity', label: 'Sensitivity' },
];

export function AnalyzeWorkflowNinebar() {
  const location = useLocation();
  return (
    <div
      data-testid="analyze-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <nav
        aria-label="Analyze method selector"
        data-testid="analyze-method-tabs"
        className="flex items-center gap-1 px-3 shrink-0"
        style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}
      >
        {METHODS.map((m) => {
          const active =
            m.path === '/analyze'
              ? location.pathname === '/analyze'
              : location.pathname === m.path || location.pathname.startsWith(`${m.path}/`);
          return (
            <Link
              key={m.id}
              to={{ pathname: m.path, search: location.search }}
              data-testid={`analyze-method-${m.id}`}
              data-active={active}
              style={{
                textDecoration: 'none',
                padding: '3px 10px',
                borderRadius: 4,
                fontSize: 12,
                background: active ? 'var(--surface-raised)' : 'transparent',
                color: active ? 'var(--text-primary)' : 'var(--text-muted)',
                border: active ? '1px solid var(--border-hairline)' : '1px solid transparent',
              }}
            >
              {m.label}
            </Link>
          );
        })}
      </nav>
      <div className="flex-1 min-h-0 overflow-hidden">
        <Outlet />
      </div>
    </div>
  );
}
