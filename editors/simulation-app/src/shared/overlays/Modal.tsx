/**
 * Modal — the accessible overlay host primitive (ninebar Phase 1).
 *
 * Portalled to `document.body` over a `--surface-scrim` backdrop; the
 * panel sits on `--surface-overlay` (its own tier, distinct from the
 * `--surface-panel` rails use), `--radius-lg`, `--shadow-float`, and a
 * 1px `--border-default`. This is the home for every config/action
 * surface the plan's placement matrix (§1) routes to "Modal" — Phase 1
 * ships the primitive only, no concrete modal is registered yet (see
 * `modalStore.ts` for the id-based registry frame chips / Cmd-K will use).
 *
 * Accessibility:
 *   - `role="dialog"` + `aria-modal="true"` + `aria-label={title}`.
 *   - Focus moves to the first focusable element inside the panel on
 *     open (falls back to the panel itself if it has none).
 *   - Tab / Shift+Tab cycle within the panel (simple wrap, same pattern
 *     as `CommandPalette`'s focus trap).
 *   - Escape and a backdrop click both close.
 *   - Focus is restored to whatever had it before the modal opened.
 *   - The enter transition is skipped under `prefers-reduced-motion`
 *     (opacity still resolves to 1 immediately; no transform/fade).
 */
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
}

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

export function Modal({ open, onClose, title, children }: ModalProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  // Scopes the *initial*-focus query to the caller's content, not the
  // modal's own header chrome (title + close button) — the close button
  // sits before the content in DOM order, so a panel-wide query would
  // always land initial focus there instead of on the content the caller
  // actually cares about. The close button is still reachable — and
  // still part of the loop — via the panel-wide Tab-cycle below.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const lastActiveElement = useRef<HTMLElement | null>(null);
  const [entered, setEntered] = useState(false);

  // Focus the first focusable element on open; remember what had focus
  // beforehand so it can be restored on close.
  useEffect(() => {
    if (!open) return;
    lastActiveElement.current = (document.activeElement as HTMLElement) ?? null;
    const t = window.setTimeout(() => {
      const body = bodyRef.current;
      const panel = panelRef.current;
      const bodyFocusable = body?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
      const target =
        bodyFocusable && bodyFocusable.length > 0
          ? bodyFocusable[0]
          : (panel?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)[0] ?? panel);
      target?.focus();
    }, 0);
    return () => window.clearTimeout(t);
  }, [open]);

  // Restore focus on close.
  useEffect(() => {
    if (open) return;
    lastActiveElement.current?.focus?.();
  }, [open]);

  // Escape closes.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  // Enter transition — flips one frame after mount so the CSS transition
  // actually runs (skipped entirely under prefers-reduced-motion).
  useEffect(() => {
    if (!open) {
      setEntered(false);
      return;
    }
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(raf);
  }, [open]);

  if (!open) return null;

  function onPanelKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key !== 'Tab') return;
    const panel = panelRef.current;
    if (!panel) return;
    const focusable = panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement as HTMLElement | null;
    if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  const reducedMotion = prefersReducedMotion();

  return createPortal(
    <div
      data-testid="modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--surface-scrim)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 1000,
      }}
    >
      <div
        ref={panelRef}
        data-testid="modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onKeyDown={onPanelKeyDown}
        className="flex flex-col"
        style={{
          width: 'min(560px, 92vw)',
          maxHeight: '85vh',
          background: 'var(--surface-overlay)',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-lg)',
          boxShadow: 'var(--shadow-float)',
          overflow: 'hidden',
          opacity: reducedMotion ? 1 : entered ? 1 : 0,
          transform: reducedMotion ? undefined : entered ? 'scale(1)' : 'scale(0.98)',
          transition: reducedMotion
            ? undefined
            : 'opacity var(--motion-panel) ease-out, transform var(--motion-panel) ease-out',
        }}
      >
        <div
          className="flex items-center justify-between shrink-0 px-4"
          style={{
            height: 'var(--row-default)',
            borderBottom: '1px solid var(--border-default)',
          }}
        >
          <span
            data-testid="modal-title"
            style={{ fontSize: 'var(--text-md)', fontWeight: 500, color: 'var(--text-primary)' }}
          >
            {title}
          </span>
          <button
            type="button"
            data-testid="modal-close"
            onClick={onClose}
            aria-label="Close"
            className="material-symbols-outlined"
            style={{
              fontSize: 18,
              color: 'var(--text-muted)',
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              padding: 4,
            }}
          >
            close
          </button>
        </div>
        <div ref={bodyRef} className="flex-1 overflow-auto px-4 py-3">
          {children}
        </div>
      </div>
    </div>,
    document.body,
  );
}
