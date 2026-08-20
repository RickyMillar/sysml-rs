import { describe, expect, it, vi } from 'vitest';
import { ensureSysmlLanguage } from '../sysmlLanguage';
import { sysmlLanguageId } from '../sysmlLanguageId';

function makeStubMonaco() {
  const registered: Array<{ id: string }> = [];
  const langs = {
    register: vi.fn((def: { id: string }) => {
      registered.push(def);
    }),
    getLanguages: vi.fn(() => registered.slice()),
    setLanguageConfiguration: vi.fn(() => ({ dispose: () => {} })),
  };
  return { monaco: { languages: langs }, langs, registered };
}

describe('ensureSysmlLanguage', () => {
  it('registers the sysml language id + bracket/autoclose config on first call', () => {
    const { monaco, langs, registered } = makeStubMonaco();
    const first = ensureSysmlLanguage(monaco);
    expect(first).toBe(true);
    expect(registered).toEqual([{ id: sysmlLanguageId }]);
    expect(langs.setLanguageConfiguration).toHaveBeenCalledWith(
      sysmlLanguageId,
      expect.objectContaining({
        comments: expect.objectContaining({ lineComment: '//' }),
      }),
    );
  });

  it('is idempotent — second call is a no-op', () => {
    const { monaco, langs, registered } = makeStubMonaco();
    ensureSysmlLanguage(monaco);
    const second = ensureSysmlLanguage(monaco);
    expect(second).toBe(false);
    expect(registered).toHaveLength(1);
    expect(langs.register).toHaveBeenCalledTimes(1);
    expect(langs.setLanguageConfiguration).toHaveBeenCalledTimes(1);
  });

  it('registers NO Monarch tokens provider — colouring is the LSP semantic-tokens provider job', () => {
    // Hard contract: if someone re-introduces a Monarch fallback the
    // editor will hide backend tokenizer gaps again. This test fails
    // the moment `setMonarchTokensProvider` is called.
    const { monaco } = makeStubMonaco();
    const setMonarch = vi.fn();
    (
      monaco.languages as unknown as {
        setMonarchTokensProvider?: typeof setMonarch;
      }
    ).setMonarchTokensProvider = setMonarch;
    ensureSysmlLanguage(monaco);
    expect(setMonarch).not.toHaveBeenCalled();
  });
});
