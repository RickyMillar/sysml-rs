/**
 * Pin the `sysml-engineering` theme registration contract:
 *   1. `ensureSysmlTheme` calls `monaco.editor.defineTheme` with the
 *      stable id consumed by `<Editor theme={sysmlThemeId} />`.
 *   2. The theme inherits from `vs-dark` (so anything we haven't
 *      mapped falls back to a sensible dark baseline).
 *   3. It covers every LSP-emitted token type the server advertises
 *      in its `semanticTokensProvider.legend` — if a kind shows up
 *      from the backend without a colour, this test breaks loudly.
 *   4. The call is idempotent — sneak-peek hovers and the live
 *      editor both call `ensureSysmlTheme`; the second call must
 *      short-circuit.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  applySysmlTheme,
  ensureSysmlTheme,
  sysmlLightThemeId,
  sysmlThemeId,
  sysmlThemeIdFor,
  _resetSysmlThemeForTesting,
} from '../sysmlTheme';

const SERVER_TOKEN_TYPES = [
  'namespace',
  'type',
  'class',
  'struct',
  'property',
  'variable',
  'parameter',
  'function',
  'keyword',
  'comment',
  'string',
  'number',
  'operator',
  'interface',
  'enum',
];

function makeMonacoStub() {
  return {
    editor: {
      defineTheme: vi.fn(),
      setTheme: vi.fn(),
    },
  };
}

afterEach(() => {
  _resetSysmlThemeForTesting();
});

describe('ensureSysmlTheme', () => {
  it('registers both the dark + light themes under their stable ids', () => {
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    // Two themes now: dark (default) + light ("warm paper"), so the
    // editor follows the app's light/dark toggle like every surface.
    expect(monaco.editor.defineTheme).toHaveBeenCalledTimes(2);
    const [name, data] = monaco.editor.defineTheme.mock.calls[0]!;
    expect(name).toBe(sysmlThemeId);
    expect(data.base).toBe('vs-dark');
    // Deliberately do NOT inherit: any token the LSP doesn't classify
    // should render in the magenta hard-fail default, not silently
    // pick up a base theme's baked-in colours.
    expect(data.inherit).toBe(false);
    const fallbackRule = (data.rules as Array<{ token: string; foreground?: string }>)
      .find((r) => r.token === '');
    expect(fallbackRule?.foreground?.toLowerCase()).toBe('ff00ff');
    // CRITICAL: this flag is the only thing that tells Monaco to apply
    // semantic-token rules to colours.
    expect(data.semanticHighlighting).toBe(true);

    // The light theme is a `vs`-based sibling with the same opt-ins.
    const [lightName, lightData] = monaco.editor.defineTheme.mock.calls[1]!;
    expect(lightName).toBe(sysmlLightThemeId);
    expect(lightData.base).toBe('vs');
    expect(lightData.inherit).toBe(false);
    expect(lightData.semanticHighlighting).toBe(true);
  });

  it('applySysmlTheme activates the id matching the app theme', () => {
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    // @monaco-editor/react sets the theme prop BEFORE onMount fires our
    // defineTheme, so the editor first falls back to a built-in; the
    // caller re-applies our registered theme for the current app theme.
    applySysmlTheme(monaco, 'dark');
    expect(monaco.editor.setTheme).toHaveBeenCalledWith(sysmlThemeId);
    applySysmlTheme(monaco, 'light');
    expect(monaco.editor.setTheme).toHaveBeenCalledWith(sysmlLightThemeId);
    expect(sysmlThemeIdFor('light')).toBe(sysmlLightThemeId);
    expect(sysmlThemeIdFor('dark')).toBe(sysmlThemeId);
  });

  it('covers every LSP-emitted token type', () => {
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    const [, data] = monaco.editor.defineTheme.mock.calls[0]!;
    const ruleTokens = new Set(
      (data.rules as Array<{ token: string }>).map((r) => r.token),
    );
    for (const type of SERVER_TOKEN_TYPES) {
      // Must be the PLAIN legend name — that's the scope LSP semantic
      // tokens carry. A `${type}.sysml`-only rule is a dead Monarch
      // scope (the tokenizer was deleted) and never matches a semantic
      // token, so the type would render in the magenta hard-fail colour.
      expect(
        ruleTokens.has(type),
        `theme missing plain-name rule for legend token "${type}" (semantic tokens won't colour without it)`,
      ).toBe(true);
    }
  });

  it('renders unresolved references dimmed + underlined (not magenta) in both themes', () => {
    // The honesty gate: a reference the backend ATTEMPTED but could not
    // resolve carries the `unresolved` legend modifier and must read as a
    // muted, underlined "broken link" — never the magenta hard-fail, which
    // is reserved for tokens the backend does not classify at all.
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    for (const call of monaco.editor.defineTheme.mock.calls) {
      const [, data] = call;
      const rule = (
        data.rules as Array<{ token: string; foreground?: string; fontStyle?: string }>
      ).find((r) => r.token === 'variable.unresolved');
      expect(rule, 'theme missing rule for unresolved reference tokens').toBeDefined();
      expect(rule?.fontStyle).toBe('underline');
      // Dimmed to a real colour (the outline hue), and NOT the magenta default.
      expect(rule?.foreground).toMatch(/^[0-9a-f]{6}$/i);
      expect(rule?.foreground?.toLowerCase()).not.toBe('ff00ff');
    }
  });

  it('is idempotent across mounts', () => {
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    ensureSysmlTheme(monaco);
    ensureSysmlTheme(monaco);
    // Two defineTheme calls (dark + light) on the FIRST ensure; the
    // repeat calls short-circuit — so exactly two, not six.
    expect(monaco.editor.defineTheme).toHaveBeenCalledTimes(2);
  });

  it('paints editor chrome (background / line numbers / gutter)', () => {
    const monaco = makeMonacoStub();
    ensureSysmlTheme(monaco);
    const [, data] = monaco.editor.defineTheme.mock.calls[0]!;
    const colors = data.colors as Record<string, string>;
    expect(colors['editor.background']).toMatch(/^#[0-9a-f]{6}$/i);
    expect(colors['editor.foreground']).toMatch(/^#[0-9a-f]{6}$/i);
    expect(colors['editorLineNumber.foreground']).toMatch(/^#[0-9a-f]{6}$/i);
    expect(colors['editorGutter.background']).toMatch(/^#[0-9a-f]{6}$/i);
  });
});
