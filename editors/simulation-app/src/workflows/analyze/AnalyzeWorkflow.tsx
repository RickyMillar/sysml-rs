import { Outlet, Link, useLocation } from 'react-router-dom';
import { AnalysisCasesLanding } from './AnalysisCasesLanding';
import { AnalyzeWorkflowNinebar } from './AnalyzeWorkflowNinebar';
import { isFlagEnabled } from '@/featureFlags';

const MODES = [
  { id: 'cases', path: '/analyze', label: 'Cases', icon: 'analytics' },
  { id: 'sweep', path: '/analyze/sweep', label: 'Sweep', icon: 'tune' },
  { id: 'montecarlo', path: '/analyze/montecarlo', label: 'Monte Carlo', icon: 'casino' },
  { id: 'trade-study', path: '/analyze/trade-study', label: 'Trade Study', icon: 'balance' },
  { id: 'sensitivity', path: '/analyze/sensitivity', label: 'Sensitivity', icon: 'analytics' },
];

/**
 * Route entry for /analyze. Under the (default-on) `ninebar` flag the
 * shell is the re-composed method-tab row (`AnalyzeWorkflowNinebar`);
 * flag-off keeps the legacy mode-tab bar verbatim (deleted in Phase 8
 * per F17).
 */
export function AnalyzeWorkflow() {
  if (isFlagEnabled('ninebar')) return <AnalyzeWorkflowNinebar />;
  return <AnalyzeWorkflowLegacy />;
}

function AnalyzeWorkflowLegacy() {
  return (
    <div data-testid="analyze-workflow" className="flex flex-col h-full w-full overflow-hidden">
      <AnalyzeModeTabs />
      <div className="flex-1 min-h-0 overflow-hidden">
        <Outlet />
      </div>
    </div>
  );
}

export function AnalyzeIndexRedirect() {
  return <AnalysisCasesLanding />;
}

function AnalyzeModeTabs() {
  const location = useLocation();
  return (
    <nav
      data-testid="analyze-mode-tabs"
      aria-label="Analyze mode selector"
      className="flex items-center gap-1 px-3 py-1.5 shrink-0"
      style={{
        borderBottom: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      }}
    >
      <span style={{ fontSize: 11, color: 'var(--outline)', fontWeight: 800, marginRight: 6 }}>
        Analyze
      </span>
      {MODES.map((mode) => {
        const active = mode.path === '/analyze'
          ? location.pathname === '/analyze'
          : location.pathname === mode.path || location.pathname.startsWith(`${mode.path}/`);
        return (
          <Link
            key={mode.id}
            to={{ pathname: mode.path, search: location.search }}
            data-testid={`analyze-mode-${mode.id}`}
            className="inline-flex items-center gap-1 rounded"
            style={{
              textDecoration: 'none',
              border: '1px solid var(--outline-variant)',
              background: active ? 'var(--primary-container)' : 'var(--surface-container)',
              color: active ? 'var(--on-primary-container)' : 'var(--on-surface-variant)',
              padding: '4px 8px',
              fontSize: 11,
              fontWeight: active ? 800 : 600,
            }}
          >
            <span className="material-symbols-outlined" style={{ fontSize: 13 }}>{mode.icon}</span>
            {mode.label}
          </Link>
        );
      })}
      <span style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--outline)' }}>
        Select an AnalysisCase or choose a method directly.
      </span>
    </nav>
  );
}
