/**
 * SneakPeek — read-only Monaco preview of an element's source slice (S4.T5).
 *
 * Lives inside the hover popup and the utility-drawer source panel will
 * surface it for diagram-click flows too. Reads from the cached
 * `sysml.get_source` query so a hover after a diagram-click reuses the
 * same response, and a diagram-click after a hover never refetches.
 *
 * Read-only on purpose — live edits arrive once ADR-013 settles the
 * editor transport. The Monaco language registry is idempotent, so
 * multiple SneakPeek instances mounted side-by-side with the main
 * SourcePanel cooperate without re-registering the sysml grammar.
 */

import type { CSSProperties } from 'react';
import { MonacoSysmlEditor } from '@/features/editor/MonacoSysmlEditor';
import { useGetSource } from '@/features/editor/useGetSource';

export interface SneakPeekProps {
  /** Element URI from the active selection / hover target. */
  uri: string | null;
  /** Element id (the same id the diagram and tree share). */
  elementId: string | null;
  /** Pixel height for the embedded Monaco. Defaults to 140. */
  heightPx?: number;
  /** Test hook for the outer wrapper. */
  testId?: string;
}

/** Default sneak-peek height — fits ~6 lines of SysML at 12px font. */
const DEFAULT_HEIGHT_PX = 140;

export function SneakPeek({
  uri,
  elementId,
  heightPx = DEFAULT_HEIGHT_PX,
  testId,
}: SneakPeekProps) {
  const query = useGetSource(uri, elementId);

  if (!uri || !elementId) return null;

  const baseStyle: CSSProperties = {
    marginTop: 8,
    border: '1px solid color-mix(in srgb, var(--border-default) 22%, transparent)',
    borderRadius: 6,
    overflow: 'hidden',
    background: 'var(--surface-panel)',
  };

  if (query.isLoading) {
    return (
      <div
        data-testid={testId ?? 'sneak-peek-loading'}
        style={{ ...baseStyle, padding: 8, fontSize: 11, color: 'var(--text-secondary)' }}
      >
        Loading source…
      </div>
    );
  }

  if (query.isError) {
    return (
      <div
        data-testid={testId ?? 'sneak-peek-error'}
        style={{ ...baseStyle, padding: 8, fontSize: 11, color: 'var(--severity-error)' }}
      >
        Failed to load source.
      </div>
    );
  }

  const result = query.data;
  if (!result) {
    return (
      <div
        data-testid={testId ?? 'sneak-peek-no-span'}
        style={{ ...baseStyle, padding: 8, fontSize: 11, color: 'var(--text-secondary)' }}
      >
        No source span — synthesised or stdlib element.
      </div>
    );
  }

  return (
    <div data-testid={testId ?? 'sneak-peek'} style={{ ...baseStyle, height: heightPx }}>
      <MonacoSysmlEditor
        value={result.text}
        readOnly
        height={heightPx}
        revealLineCol={
          typeof result.line === 'number' ? { line: result.line, col: result.col } : undefined
        }
        testId="sneak-peek-editor"
      />
    </div>
  );
}
