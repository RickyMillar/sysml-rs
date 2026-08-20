/**
 * Verifies that mounting MonacoSysmlEditor wires the sysml language and
 * the view-supertype snippet provider — the load-bearing part of T4.
 *
 * We mock `@monaco-editor/react` so the test runs in jsdom without
 * needing the real monaco bundle. The mock's `Editor` synchronously
 * calls `onMount` with a stub editor + monaco namespace, exactly the
 * surface our component depends on.
 */

import { describe, expect, it, vi } from 'vitest';
import { render } from '@testing-library/react';

const hoisted = vi.hoisted(() => {
  const register = vi.fn();
  const getLanguages = vi.fn(() => [] as Array<{ id: string }>);
  const setLanguageConfiguration = vi.fn(() => ({ dispose: () => {} }));
  const registerCompletionItemProvider = vi.fn(() => ({ dispose: () => {} }));
  const revealLineInCenter = vi.fn();
  const setPosition = vi.fn();

  const defineTheme = vi.fn();
  const stubMonaco = {
    languages: {
      register,
      getLanguages,
      setLanguageConfiguration,
      registerCompletionItemProvider,
      CompletionItemKind: { Snippet: 27 },
      CompletionItemInsertTextRule: { InsertAsSnippet: 4 },
    },
    editor: {
      defineTheme,
    },
  };
  const editorStub = { revealLineInCenter, setPosition };

  return {
    register,
    getLanguages,
    setLanguageConfiguration,
    registerCompletionItemProvider,
    revealLineInCenter,
    setPosition,
    stubMonaco,
    editorStub,
  };
});

vi.mock('@monaco-editor/react', () => ({
  loader: { config: vi.fn() },
  default: ({ onMount }: { onMount?: (e: unknown, m: unknown) => void }) => {
    onMount?.(hoisted.editorStub, hoisted.stubMonaco);
    return null;
  },
}));

vi.mock('monaco-editor/esm/vs/editor/editor.worker?worker', () => ({
  default: class FakeWorker {},
}));

vi.mock('monaco-editor', () => ({}));

import { MonacoSysmlEditor, utf16OffsetToByteOffset } from '../MonacoSysmlEditor';

describe('utf16OffsetToByteOffset (text→diagram link, byte spans)', () => {
  it('is identity for ASCII source', () => {
    expect(utf16OffsetToByteOffset('package P;', 8)).toBe(8);
  });

  it('counts UTF-8 bytes for multi-byte chars before the cursor', () => {
    // "café " — 'é' is 1 UTF-16 unit but 2 UTF-8 bytes. Cursor after the space
    // is UTF-16 offset 5 but byte offset 6.
    expect(utf16OffsetToByteOffset('café x', 5)).toBe(6);
  });

  it('handles astral (surrogate-pair) characters', () => {
    // "🚀x": rocket is 2 UTF-16 units / 4 UTF-8 bytes; cursor before 'x' is
    // UTF-16 offset 2 but byte offset 4.
    expect(utf16OffsetToByteOffset('🚀x', 2)).toBe(4);
  });
});

describe('MonacoSysmlEditor (mount wiring)', () => {
  it('registers the sysml language and view snippets on first mount', () => {
    render(<MonacoSysmlEditor value="package P {}" readOnly />);

    expect(hoisted.register).toHaveBeenCalledWith({ id: 'sysml' });
    expect(hoisted.setLanguageConfiguration).toHaveBeenCalledWith('sysml', expect.anything());
    // Monarch tokenizer intentionally removed — colouring is the LSP
    // semantic-tokens provider's job. No setMonarchTokensProvider call.
    expect(hoisted.registerCompletionItemProvider).toHaveBeenCalledWith(
      'sysml',
      expect.objectContaining({ provideCompletionItems: expect.any(Function) }),
    );
  });

  it('scrolls to revealLineCol when provided', () => {
    hoisted.revealLineInCenter.mockClear();
    hoisted.setPosition.mockClear();
    render(<MonacoSysmlEditor value="x" revealLineCol={{ line: 7, col: 3 }} />);
    expect(hoisted.revealLineInCenter).toHaveBeenCalledWith(7);
    expect(hoisted.setPosition).toHaveBeenCalledWith({ lineNumber: 7, column: 3 });
  });
});
