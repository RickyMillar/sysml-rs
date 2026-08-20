/**
 * Idempotent `sysml` language registration for any Monaco instance.
 *
 * **No Monarch tokenizer** — colouring is the LSP semantic-tokens
 * provider's job (see `lspClient.ts`). Removed deliberately so any
 * gap in the server's tokenizer surfaces visually as uncoloured
 * text (the `sysml-engineering` theme paints unclassified content
 * in `--surface-magenta` to make this loud). This is the only way
 * to keep the backend honest — silent Monarch fallback was masking
 * misclassifications.
 *
 * Language registration still ships:
 *  - bracket / autoclose / surrounding pairs
 *  - line + block comment markers
 * because those drive editor UX (matching braces, comment-toggle
 * shortcut, etc.) and don't depend on the tokenizer.
 */

import { sysmlLanguageId } from './sysmlLanguageId';

/**
 * Minimal type surface required from the monaco namespace. Hand-typed so
 * unit tests can pass a stub without pulling in monaco-editor.
 */
interface MonacoLike {
  languages: {
    register: (def: { id: string }) => void;
    getLanguages: () => Array<{ id: string }>;
    setLanguageConfiguration: (
      id: string,
      conf: unknown,
    ) => { dispose: () => void };
  };
}

/**
 * Register the `sysml` language id with the given monaco instance. Safe
 * to call multiple times — subsequent calls are no-ops.
 *
 * No tokenizer registration: colouring is the LSP semantic-tokens
 * provider's job. Anything the server doesn't classify renders
 * uncoloured, which is the point — silent Monarch fallback used to
 * mask backend tokenizer gaps.
 *
 * Returns `true` if this call performed the registration, `false` if a
 * prior call already did. Tests assert idempotence.
 */
export function ensureSysmlLanguage(monaco: MonacoLike): boolean {
  const already = monaco.languages
    .getLanguages()
    .some((l) => l.id === sysmlLanguageId);
  if (already) return false;

  monaco.languages.register({ id: sysmlLanguageId });

  monaco.languages.setLanguageConfiguration(sysmlLanguageId, {
    comments: { lineComment: '//', blockComment: ['/*', '*/'] },
    brackets: [
      ['{', '}'],
      ['[', ']'],
      ['(', ')'],
    ],
    autoClosingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"' },
    ],
    surroundingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"' },
    ],
  });

  return true;
}
