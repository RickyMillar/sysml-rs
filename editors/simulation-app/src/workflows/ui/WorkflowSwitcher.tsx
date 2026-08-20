/**
 * WorkflowSwitcher — top-of-shell tab strip for the workflow router.
 *
 * Replaces the legacy `ToolNav` introduced in Round 0. Each tab links
 * to its workflow route via React Router; the active route is
 * highlighted. Analyze sub-workflows are visually grouped.
 *
 * Keyboard: Cmd/Ctrl+<digit> jumps to the matching workflow. On mac
 * we bind to ⌘, elsewhere to Alt (which is the conventional "browser
 * stays out of the way" modifier on Windows/Linux).
 *
 * Each tab exposes a stable `tool-tab-<id>` testid for integration
 * helpers.
 */

import { useCallback, useEffect, useMemo } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { WORKFLOWS, navActiveIdForPath } from '@/workflows/routes';
import { useWorkspaceUIStore, type ActiveTool } from '@/features/workspace/store';

// ── Platform detection ───────────────────────────────────────────────

export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  // navigator.platform is deprecated but still universally available;
  // userAgent fallback covers everything except rare embedded runtimes.
  const p = navigator.platform ?? '';
  const ua = navigator.userAgent ?? '';
  return /Mac|iPhone|iPad|iPod/i.test(p) || /Macintosh/i.test(ua);
}

export function modifierLabel(mac: boolean): string {
  return mac ? '⌘' : 'Alt';
}

// ── Hotkey resolver ─────────────────────────────────────────────────
// Pulled out of the component so it's unit-testable (node env, no DOM).

export interface HotkeyIntent {
  /** The digit key that was pressed (e.g. "1"). */
  digit: string;
  /** The workflow id mapped from that digit, or null if none. */
  workflowId: string | null;
  /** The URL path to navigate to, or null if no match. */
  path: string | null;
}

export function resolveHotkey(
  e: { metaKey: boolean; altKey: boolean; ctrlKey: boolean; key: string },
  mac: boolean,
): HotkeyIntent | null {
  const mod = mac ? e.metaKey : e.altKey;
  if (!mod) return null;
  // Don't accidentally swallow system shortcuts (Ctrl+Alt+1 etc).
  if (!mac && e.ctrlKey) return null;
  if (!/^[1-9]$/.test(e.key)) return null;
  const target = WORKFLOWS.find((w) => w.hotkey === e.key);
  return {
    digit: e.key,
    workflowId: target?.id ?? null,
    path: target?.path ?? null,
  };
}

// ── Switcher ─────────────────────────────────────────────────────────

/**
 * `variant` (ninebar screenshot-comparison ruling C, 2026-07-14 — "frame
 * de-noise"): `variant="ninebar"` renders the tabs **text-only** (no
 * Material icon) with the demo's calm active treatment (neutral surface
 * tint + hairline border, never accent), and drops the switcher's own
 * nested sub-bar chrome (height / border / background) so the tabs sit
 * inline in the frame rather than as a bordered strip inside it. Only
 * `AppShell` passes it; the legacy shell omits it and gets the unchanged
 * `variant="default"` rendering (icon + label, accent-tint active).
 */
export type WorkflowSwitcherVariant = 'default' | 'ninebar';

export function WorkflowSwitcher({ variant = 'default' }: { variant?: WorkflowSwitcherVariant } = {}) {
  const location = useLocation();
  const navigate = useNavigate();
  const setActiveTool = useWorkspaceUIStore((s) => s.setActiveTool);

  // Longest-prefix match over VISIBLE workflows: hidden sub-routes
  // light their door's tab (/analyze/sweep → Analyze, /run/compare →
  // Run — Compare is a Simulate mode per the Phase 6 demotion).
  const activeId = useMemo(
    () => navActiveIdForPath(location.pathname),
    [location.pathname],
  );

  const mac = useMemo(isMacPlatform, []);

  // Global Cmd/Alt+<digit> shortcut. Uses capture so nothing in the
  // workflow surface (Monaco, inputs, etc.) can swallow it first.
  const handleHotkey = useCallback(
    (e: KeyboardEvent) => {
      const intent = resolveHotkey(e, mac);
      if (!intent?.path) return;
      e.preventDefault();
      navigate(intent.path);
    },
    [mac, navigate],
  );

  useEffect(() => {
    window.addEventListener('keydown', handleHotkey, true);
    return () => window.removeEventListener('keydown', handleHotkey, true);
  }, [handleHotkey]);

  // Group visible workflows for visual separation (primary | analyze | trailing).
  const visibleWorkflows = WORKFLOWS.filter((w) => w.visibleInNav !== false);
  const primary = visibleWorkflows.filter(
    (w) => w.group === 'primary' && w.id !== 'compare',
  );
  const analyze = visibleWorkflows.filter((w) => w.id === 'analyze');
  const trailing = visibleWorkflows.filter((w) => w.id === 'compare');

  const ninebar = variant === 'ninebar';
  // ninebar: inline in the frame, no nested sub-bar chrome (ruling C).
  const navStyle: React.CSSProperties = ninebar
    ? {}
    : {
        height: 36,
        borderBottom: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      };

  return (
    <nav
      data-testid="tool-nav"
      aria-label="Workflow switcher"
      className="flex items-center gap-1 px-2 shrink-0"
      style={navStyle}
    >
      <TabGroup
        workflows={primary}
        activeId={activeId}
        mac={mac}
        variant={variant}
        onNavigate={(id) => setActiveTool(id as ActiveTool)}
      />

      {analyze.length > 0 && <Divider ninebar={ninebar} />}

      <TabGroup
        workflows={analyze}
        activeId={activeId}
        mac={mac}
        variant={variant}
        onNavigate={(id) => setActiveTool(id as ActiveTool)}
      />

      <div style={{ flex: 1 }} />

      <TabGroup
        workflows={trailing}
        activeId={activeId}
        mac={mac}
        variant={variant}
        onNavigate={(id) => setActiveTool(id as ActiveTool)}
      />
    </nav>
  );
}

// ── Divider ──────────────────────────────────────────────────────────

function Divider({ ninebar }: { ninebar?: boolean }) {
  return (
    <div
      aria-hidden
      style={{
        width: 1,
        height: 18,
        background: ninebar ? 'var(--border-hairline)' : 'var(--outline-variant)',
        margin: '0 4px',
      }}
    />
  );
}

// ── TabGroup ─────────────────────────────────────────────────────────

interface TabGroupProps {
  workflows: typeof WORKFLOWS;
  activeId: string | null;
  mac: boolean;
  variant: WorkflowSwitcherVariant;
  onNavigate: (id: string) => void;
}

function TabGroup({ workflows, activeId, mac, variant, onNavigate }: TabGroupProps) {
  if (workflows.length === 0) return null;
  const ninebar = variant === 'ninebar';
  return (
    <>
      {workflows.map((wf) => {
        const isActive = activeId === wf.id;
        // ninebar (ruling C): text-only, calm active = neutral surface
        // tint + hairline border (never accent); semantic tokens only.
        const style: React.CSSProperties = ninebar
          ? {
              background: isActive ? 'var(--surface-raised)' : 'transparent',
              color: isActive ? 'var(--text-primary)' : 'var(--text-muted)',
              border: isActive ? '1px solid var(--border-hairline)' : '1px solid transparent',
              cursor: 'pointer',
              fontSize: '12px',
              fontWeight: isActive ? 600 : 400,
              textDecoration: 'none',
            }
          : {
              background: isActive ? 'var(--primary-container)' : 'transparent',
              color: isActive ? 'var(--on-primary-container)' : 'var(--outline)',
              border: 'none',
              cursor: 'pointer',
              fontSize: '12px',
              fontWeight: isActive ? 600 : 400,
              textDecoration: 'none',
            };
        return (
          <Link
            key={wf.id}
            to={wf.path}
            data-testid={`tool-tab-${wf.id}`}
            data-workflow-id={wf.id}
            data-active={isActive ? 'true' : 'false'}
            title={
              wf.hotkey
                ? `${wf.label} (${modifierLabel(mac)}+${wf.hotkey})`
                : wf.label
            }
            onClick={() => onNavigate(wf.id)}
            className="flex items-center gap-1.5 px-3 py-1 rounded transition-colors no-underline"
            style={style}
          >
            {/* ninebar tabs are text-only (ruling C); the legacy variant
                keeps the leading Material icon. */}
            {!ninebar && (
              <span className="material-symbols-outlined" style={{ fontSize: '16px' }}>
                {wf.icon}
              </span>
            )}
            {wf.label}
          </Link>
        );
      })}
    </>
  );
}
