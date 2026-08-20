/**
 * MonacoSysmlEditor — Monaco mount for SysML v2 source.
 *
 * Two modes:
 * - Read-only sneak-peek (no `lspUri`, `readOnly` true). Used by
 *   `SneakPeek` for hover popups over a span slice.
 * - Live editor (`lspUri` set). Attaches the `/lsp` WebSocket client,
 *   so the buffer participates in LSP diagnostics / hover / completion
 *   / goto-def. The LSP server shares the host's `SysmlService` (see
 *   commit 9aef3948), so REST `sysml.*` queries see edits immediately.
 *
 * On first mount we configure the monaco-editor loader, register the
 * `sysml` language id, and install the view-supertype snippet registry.
 * All three are idempotent so sneak-peeks + the live editor cooperate.
 */

import { useCallback, useEffect, useRef } from 'react';
import Editor, { type OnChange, type OnMount } from '@monaco-editor/react';
import { configureMonaco } from './monacoSetup';
import { ensureSysmlLanguage } from './sysmlLanguage';
import { sysmlLanguageId } from './sysmlLanguageId';
import { applySysmlTheme, ensureSysmlTheme, sysmlThemeIdFor } from './sysmlTheme';
import { useTheme } from '@/app/useTheme';
import { registerSysmlViewSnippets } from '@/features/views/snippets';
import { attachLspClient, lspWebSocketUrl } from './lspClient';

const TEXT_ENCODER = typeof TextEncoder !== 'undefined' ? new TextEncoder() : null;

/**
 * Convert a Monaco UTF-16 code-unit offset into the UTF-8 byte offset that the
 * Rust text-map spans use. Equal for ASCII; they diverge once the source carries
 * any multi-byte character before the cursor. Falls back to the UTF-16 offset if
 * `TextEncoder` is unavailable (non-browser test env).
 */
export function utf16OffsetToByteOffset(text: string, utf16Offset: number): number {
  if (!TEXT_ENCODER) return utf16Offset;
  return TEXT_ENCODER.encode(text.slice(0, utf16Offset)).length;
}

export interface MonacoSysmlEditorProps {
  /** Text content. Pass `''` for an empty buffer. */
  value: string;
  /** Lock the buffer (sneak-peek path). Defaults to false. */
  readOnly?: boolean;
  /** Pixel height, or any CSS height value. Defaults to `100%`. */
  height?: string | number;
  /**
   * 1-based line / column to scroll into view once Monaco has rendered.
   * Used by sneak-peeks to align the popup on the element's span, and
   * by the live editor when the user picks a tree/diagram element.
   */
  revealLineCol?: { line: number; col?: number };
  /**
   * When set, attach the `/lsp` WebSocket LSP client to this buffer
   * using `lspUri` as the document identifier. Triggers live
   * diagnostics / hover / completion / goto-def. Sneak-peeks omit
   * this to stay read-only and not open transient sockets.
   */
  lspUri?: string;
  /** Buffer change callback (live-editor path). */
  onChange?: (value: string) => void;
  /**
   * Cursor-moved callback (Bucket 2.3 text→diagram link). Reports the primary
   * cursor position as a UTF-8 **byte offset** (converted from Monaco's UTF-16
   * offset here, see {@link utf16OffsetToByteOffset}) so it compares directly to
   * the Rust text-map's byte spans. Debounced by the caller; resolves cursor →
   * ElementId via the ViewModel text-map.
   */
  onCursorOffset?: (offset: number) => void;
  /** Forwarded to the underlying Monaco editor wrapper. */
  className?: string;
  /** Test hook — assigned to the wrapping div. */
  testId?: string;
}

export function MonacoSysmlEditor({
  value,
  readOnly = false,
  height = '100%',
  revealLineCol,
  lspUri,
  onChange,
  onCursorOffset,
  className,
  testId,
}: MonacoSysmlEditorProps) {
  const lspDisposeRef = useRef<(() => void) | null>(null);
  const cursorDisposeRef = useRef<(() => void) | null>(null);
  // Hold the latest cursor callback so the Monaco listener (attached once at
  // mount) always calls the current closure without re-subscribing.
  const onCursorOffsetRef = useRef<typeof onCursorOffset>(onCursorOffset);
  onCursorOffsetRef.current = onCursorOffset;

  // The editor follows the app's light/dark toggle like every other
  // surface. The reactive `theme` prop below re-themes on flip; the ref
  // lets the mount handler apply the right theme through Monaco's
  // define-before-activate race.
  const { theme } = useTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;

  useEffect(() => {
    // Fire-and-forget: configures monaco-editor's loader on first mount.
    void configureMonaco();
  }, []);

  // Detach the LSP client + cursor listener on unmount / lspUri change.
  useEffect(() => {
    return () => {
      lspDisposeRef.current?.();
      lspDisposeRef.current = null;
      cursorDisposeRef.current?.();
      cursorDisposeRef.current = null;
    };
  }, [lspUri]);

  const handleMount = useCallback<OnMount>(
    (editor, monaco) => {
      ensureSysmlLanguage(monaco);
      ensureSysmlTheme(monaco);
      applySysmlTheme(monaco, themeRef.current);
      registerSysmlViewSnippets(monaco, sysmlLanguageId);
      if (revealLineCol) {
        editor.revealLineInCenter(revealLineCol.line);
        editor.setPosition({
          lineNumber: revealLineCol.line,
          column: revealLineCol.col ?? 1,
        });
      }
      if (lspUri) {
        const client = attachLspClient(editor, monaco, {
          uri: lspUri,
          lspUrl: lspWebSocketUrl(),
        });
        lspDisposeRef.current = () => client.dispose();
      }
      // @monaco-editor/react with `height="100%"` can lose the initial
      // `automaticLayout` race: Monaco paints at its 5px fallback size and
      // the ResizeObserver never fires a correction because the container
      // was already at its final size when the observer attached. The LSP
      // attach above (synchronous provider registration) widens that
      // window enough that the reading floor reliably came up 5×5. Force
      // one explicit layout now (container is sized by mount time) plus a
      // post-paint retry, so the editor deterministically fills its host.
      if (typeof editor.layout === 'function') {
        editor.layout();
        if (typeof requestAnimationFrame === 'function') {
          requestAnimationFrame(() => {
            try {
              editor.layout();
            } catch {
              /* editor disposed before the frame fired — nothing to do */
            }
          });
        }
      }
      // Guard: a minimal Monaco mock (unit tests) may not implement this.
      if (typeof editor.onDidChangeCursorPosition === 'function') {
        const sub = editor.onDidChangeCursorPosition((e) => {
          const model = editor.getModel();
          if (!model) return;
          // Monaco offsets are UTF-16 code units; Rust text-map spans are UTF-8
          // byte offsets. Convert here so the cursor→element lookup compares like
          // for like (they diverge on any non-ASCII source).
          const utf16 = model.getOffsetAt(e.position);
          const text = typeof model.getValue === 'function' ? model.getValue() : '';
          onCursorOffsetRef.current?.(text ? utf16OffsetToByteOffset(text, utf16) : utf16);
        });
        cursorDisposeRef.current = () => sub.dispose();
      }
    },
    [revealLineCol, lspUri],
  );

  const handleChange = useCallback<OnChange>(
    (nextValue) => {
      onChange?.(nextValue ?? '');
    },
    [onChange],
  );

  return (
    <div
      data-testid={testId ?? 'monaco-sysml-editor'}
      className={className}
      style={{ width: '100%', height: '100%' }}
    >
      <Editor
        height={height}
        defaultLanguage={sysmlLanguageId}
        language={sysmlLanguageId}
        theme={sysmlThemeIdFor(theme)}
        value={value}
        options={{
          readOnly,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          fontSize: 12,
          lineNumbers: 'on',
          renderLineHighlight: 'gutter',
          automaticLayout: true,
          wordWrap: 'off',
          // Force-enable LSP semantic tokens. The default value
          // `'configuredByTheme'` means Monaco only asks the provider
          // when the active theme opts in via `"semanticHighlighting":
          // true` in its theme definition — and the built-in
          // `vs-dark` does NOT set that flag in the monaco-editor
          // bundle we ship. With the flag explicit Monaco fires
          // `provideDocumentSemanticTokens` on registration + every
          // model change.
          //
          // Monaco accepts the dotted-string form in the IEditorOptions
          // shape; both spellings end up at the same internal option
          // slot but the dotted form is the only one typed today.
          'semanticHighlighting.enabled': true,
        }}
        onMount={handleMount}
        onChange={onChange ? handleChange : undefined}
      />
    </div>
  );
}
