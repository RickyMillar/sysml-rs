/**
 * ReadinessChip — frame chip surfacing the model-readiness aggregation
 * (ninebar Phase 1.5, plan "Model readiness & Browse floor" — the
 * "can I trust this model?" front door: full diagnostics +
 * per-root dependency resolution + capability flags, one summary).
 *
 * Sourced from `useModelReadiness()` (`features/readiness`). Renders
 * nothing while `level === 'unknown'` (no workspace loaded) — same
 * render-nothing-until-meaningful convention as `QuotaChip`. Otherwise
 * a quiet dot + label:
 *   - ready:    green dot,  "Ready"
 *   - warnings: amber dot,  "<n> warnings"
 *   - errors:   red dot,    "<n> errors"  (diagnostic errors +
 *               unresolved dependencies — both are load-blocking)
 *
 * Token note: warning/error reuse `--severity-warning` / `--severity-error`
 * (this chip IS a severity surface — diagnostics + dependency failures
 * are severity-shaped data). The severity ladder has no "ok" value for
 * the ready dot; `--health-nominal` covers it — model readiness is a
 * health-of-the-model reading, not run evidence, so the health ladder
 * is the right home for its "nothing's wrong" glyph. Verdict tokens
 * appear only inside the static-verify modal, where actual verdicts
 * render.
 *
 * Click opens a `Popover` (drill list: file · severity · message,
 * `--row-dense` rows) with a footer action that opens the static
 * verify modal (`StaticVerifyModal`, registered under
 * `readiness-static-verify`). Clicking a drill row selects the element
 * via the shared selection store — same `select(uri, elementId)` shape
 * `DiagnosticsPanel` uses, so tree/inspector reveal-on-select behaves
 * identically from either surface.
 */
import { useRef, useState } from 'react';
import { Popover } from '@/shared/overlays/Popover';
import { useModalStore } from '@/shared/overlays/modalStore';
import { useSelectionStore } from '@/features/selection/store';
import { useModelReadiness } from '@/features/readiness/useModelReadiness';
import { DIAGNOSTIC_SEVERITY_COLORS } from '@/features/diagnostics/types';
import type { ReadinessDrillEntry, ReadinessLevel } from '@/features/readiness/types';
// Side-effect import: registers the `readiness-static-verify` modal
// descriptor so `openModal(READINESS_STATIC_VERIFY_MODAL_ID)` below has
// something to look up in `ModalHost`'s registry.
import { READINESS_STATIC_VERIFY_MODAL_ID } from '@/features/readiness/StaticVerifyModal';

function shortFileName(uri: string): string {
  if (!uri) return '';
  const lastSlash = uri.lastIndexOf('/');
  return lastSlash < 0 ? uri : uri.slice(lastSlash + 1) || uri;
}

interface LevelPresentation {
  color: string;
  label: string;
}

function presentationFor(level: ReadinessLevel, errors: number, warnings: number): LevelPresentation {
  switch (level) {
    case 'ready':
      return { color: 'var(--health-nominal)', label: 'Ready' };
    case 'warnings':
      return { color: 'var(--severity-warning)', label: `${warnings} warning${warnings === 1 ? '' : 's'}` };
    case 'errors':
      return { color: 'var(--severity-error)', label: `${errors} error${errors === 1 ? '' : 's'}` };
    case 'unknown':
      return { color: 'var(--text-muted)', label: '' };
  }
}

export function ReadinessChip() {
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);

  const readiness = useModelReadiness();
  const select = useSelectionStore((s) => s.select);
  const openModal = useModalStore((s) => s.openModal);

  if (readiness.level === 'unknown') return null;

  // Unresolved dependencies are load-blocking but not counted in
  // `counts.errors` (that field is diagnostics-only) — fold them into
  // the displayed number so "n errors" matches the full problem count
  // the drill list below shows.
  const displayErrors = readiness.counts.errors + readiness.unresolvedDeps.length;
  const { color, label } = presentationFor(readiness.level, displayErrors, readiness.counts.warnings);

  function handleRowClick(entry: ReadinessDrillEntry) {
    select(entry.file, entry.elementId ?? null, 'ui');
    setOpen(false);
  }

  function handleVerify() {
    setOpen(false);
    openModal(READINESS_STATIC_VERIFY_MODAL_ID);
  }

  return (
    <div style={{ position: 'relative', display: 'inline-flex' }}>
      <button
        ref={buttonRef}
        type="button"
        data-testid="readiness-chip"
        data-level={readiness.level}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={{
          height: 'var(--row-compact)',
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          padding: '0 8px',
          fontSize: 'var(--text-sm)',
          color: 'var(--text-primary)',
          background: 'transparent',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          cursor: 'pointer',
        }}
      >
        <span
          data-testid="readiness-chip-dot"
          aria-hidden="true"
          style={{
            width: 8,
            height: 8,
            borderRadius: '50%',
            background: color,
            flexShrink: 0,
          }}
        />
        <span>{label}</span>
      </button>

      <Popover anchorEl={buttonRef.current} open={open} onClose={() => setOpen(false)} placement="bottom">
        <div data-testid="readiness-drill-list" style={{ minWidth: 320, maxWidth: 440, padding: 6 }}>
          {readiness.drill.length === 0 ? (
            <div
              style={{
                padding: '6px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-muted)',
              }}
            >
              No issues
            </div>
          ) : (
            readiness.drill.map((entry, i) => (
              <button
                key={`${entry.file}-${i}`}
                type="button"
                data-testid={`readiness-drill-row-${i}`}
                onClick={() => handleRowClick(entry)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  width: '100%',
                  height: 'var(--row-dense)',
                  textAlign: 'left',
                  padding: '0 8px',
                  fontSize: 'var(--text-xs)',
                  background: 'transparent',
                  border: 'none',
                  borderRadius: 'var(--radius-sm)',
                  cursor: 'pointer',
                  color: 'var(--text-primary)',
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    color: DIAGNOSTIC_SEVERITY_COLORS[entry.severity],
                    textTransform: 'uppercase',
                    fontSize: 9,
                    fontWeight: 700,
                    letterSpacing: 0.4,
                    flexShrink: 0,
                    width: 46,
                  }}
                >
                  {entry.severity}
                </span>
                <span
                  data-testid="readiness-drill-row-message"
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                  title={entry.message}
                >
                  {entry.message}
                </span>
                <span
                  style={{
                    flexShrink: 0,
                    maxWidth: 120,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    color: 'var(--text-muted)',
                  }}
                  title={entry.file}
                >
                  {shortFileName(entry.file)}
                </span>
              </button>
            ))
          )}

          <div
            style={{
              borderTop: '1px solid var(--border-default)',
              marginTop: 4,
              paddingTop: 4,
            }}
          >
            <button
              type="button"
              data-testid="readiness-verify-action"
              onClick={handleVerify}
              style={{
                width: '100%',
                textAlign: 'left',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-secondary)',
                background: 'transparent',
                border: 'none',
                cursor: 'pointer',
              }}
            >
              Verify workspace (static)
            </button>
          </div>
        </div>
      </Popover>
    </div>
  );
}
