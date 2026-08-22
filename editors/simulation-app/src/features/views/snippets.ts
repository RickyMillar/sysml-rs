/**
 * Monaco snippet registry for `view def` declarations over the eight
 * standard view definitions (Bucket 5.A3). Supertypes are the canonical
 * std-lib names (`GeneralView`, `InterconnectionView`, ...) — the
 * standard library defines no bare-name aliases.
 *
 * Source of truth: `./sysml-views.json` (co-located). This was rehomed from
 * the deleted VS Code extension (`editors/vscode/snippets/sysml-views.json`,
 * Bucket 3.1 rip-out); it keeps VS Code's `contributes.snippets` JSON shape.
 *
 * Each entry has a 2-element prefix array (`["gview",
 * "view-general"]` etc.); both prefixes register against the same body
 * so either trigger expands the snippet. The supertype name is derived
 * from the body's `:> <Name>` segment, kept in sync with the eight
 * supertypes documented in `the-book/src/ch13-metadata-and-views.md`
 * "What Kind of Diagram Does a View Become?".
 *
 * `registerSysmlViewSnippets(monaco, languageId?)` plugs them into a
 * Monaco instance via `registerCompletionItemProvider`. Call once after
 * the language id is registered. Returns the disposer so the caller
 * can detach on unmount.
 */

import rawSnippets from './sysml-views.json';

const STANDARD_SUPERTYPES = [
  'GeneralView',
  'InterconnectionView',
  'ActionFlowView',
  'StateTransitionView',
  'SequenceView',
  'BrowserView',
  'GridView',
  'GeometryView',
] as const;

export type SysmlViewSupertype = (typeof STANDARD_SUPERTYPES)[number];

export interface SysmlViewSnippet {
  /** All trigger tokens that expand this snippet (VS Code allows aliases). */
  prefixes: string[];
  /** Human-readable label shown in the completion popup. */
  label: string;
  /** Snippet body in Monaco / TextMate format (`${n:placeholder}`, `$0`). */
  body: string;
  /** Doc shown in the completion popup detail pane. */
  documentation: string;
  /** The standard supertype this snippet specializes from. */
  supertype: SysmlViewSupertype;
}

interface VscodeSnippet {
  prefix: string | string[];
  body: string | string[];
  description?: string;
}

function extractSupertype(body: string): SysmlViewSupertype {
  // The view-def header line is `view def ${1:Name} :> <Supertype> {`.
  const match = body.match(/:>\s+(\w+)\s*\{/);
  if (!match) {
    throw new Error(`view-def snippet body lacks ":> Supertype {" segment: ${body}`);
  }
  const supertype = match[1];
  if (!(STANDARD_SUPERTYPES as readonly string[]).includes(supertype)) {
    throw new Error(
      `Unknown view-def supertype "${supertype}" — must be one of ${STANDARD_SUPERTYPES.join(', ')}`,
    );
  }
  return supertype as SysmlViewSupertype;
}

function toMonacoShape(label: string, entry: VscodeSnippet): SysmlViewSnippet {
  const prefixes = Array.isArray(entry.prefix) ? entry.prefix : [entry.prefix];
  const body = Array.isArray(entry.body) ? entry.body.join('\n') : entry.body;
  return {
    prefixes,
    label,
    body,
    documentation: entry.description ?? label,
    supertype: extractSupertype(body),
  };
}

export const sysmlViewSnippets: ReadonlyArray<SysmlViewSnippet> = Object.entries(
  rawSnippets as Record<string, VscodeSnippet>,
).map(([label, entry]) => toMonacoShape(label, entry));

/**
 * Minimal subset of the `monaco` namespace this module needs.
 * Hand-typed so the registry doesn't require a runtime monaco import
 * just to build (lets the snippet table be unit-tested without DOM).
 */
interface MonacoLike {
  languages: {
    registerCompletionItemProvider: (
      languageId: string,
      provider: {
        provideCompletionItems: (
          model: { getWordUntilPosition: (pos: { lineNumber: number; column: number }) => { startColumn: number; endColumn: number } },
          position: { lineNumber: number; column: number },
        ) => { suggestions: unknown[] };
      },
    ) => { dispose: () => void };
    CompletionItemKind: { Snippet: number };
    CompletionItemInsertTextRule: { InsertAsSnippet: number };
  };
}

export function registerSysmlViewSnippets(
  monaco: MonacoLike,
  languageId: string = 'sysml',
): { dispose: () => void } {
  return monaco.languages.registerCompletionItemProvider(languageId, {
    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };
      // Flatten across (snippet × prefix) so each alias gets its own
      // completion entry — matches VS Code's behavior where `gview`
      // and `view-general` both trigger the same expansion.
      const suggestions = sysmlViewSnippets.flatMap((s) =>
        s.prefixes.map((prefix) => ({
          label: prefix,
          kind: monaco.languages.CompletionItemKind.Snippet,
          detail: s.label,
          documentation: { value: s.documentation, isTrusted: false },
          insertText: s.body,
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          range,
        })),
      );
      return { suggestions };
    },
  });
}
