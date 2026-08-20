/**
 * `sysml-engineering` — Monaco theme that aligns the editor with the
 * rest of the simulation app's "Engineering Atelier" palette and
 * gives every LSP semantic-token class a distinguishable hue.
 *
 * Two-axis design:
 *
 *   1. **Monarch fallback** (`rules` with no dot) covers buffers that
 *      haven't yet received semantic tokens from the LSP — initial
 *      paint before the first `textDocument/semanticTokens/full`
 *      reply lands. Keeps comments/strings/keywords readable.
 *   2. **Semantic tokens** (`rules` keyed by the legend's type names)
 *      take over once the server responds, classifying each
 *      identifier by its SysML kind (PartDef → CLASS,
 *      RequirementDef → STRUCT, ActionDef → FUNCTION, ...).
 *
 * The 15 token types + 6 modifiers are exactly what
 * `sysml-lsp-server` advertises in its `semanticTokensProvider.legend`
 * (`crates/tooling/sysml-lsp-server/src/types.rs` —
 * `SEMANTIC_TOKEN_TYPES`). Adding a new SysML kind on the server side
 * adds a token type *here* automatically (assuming we add a colour
 * rule).
 *
 * Cross-editor consistency: the same token-type → hue mapping should
 * be picked up by the Zed and VS Code extensions in a follow-on so
 * an engineer's SysML code looks the same wherever they read it. The
 * LSP defines the *names*; each editor maps them to colours.
 */

import { sysmlLanguageId } from './sysmlLanguageId';

/**
 * Engineering Atelier palette excerpt, lifted from
 * `src/styles/tokens.css`. Duplicated here because Monaco's theme
 * registration accepts only hex strings — it can't resolve CSS
 * custom properties from the page stylesheet.
 *
 * Keep in sync with the design system. If we add a SysML kind that
 * needs a brand-new hue, add the token here first then reference it
 * below.
 */
type Palette = {
  onSurface: string;
  onSurfaceVariant: string;
  outline: string;
  outlineVariant: string;
  surface: string;
  surfaceContainerLow: string;
  surfaceContainerLowest: string;
  selection: string;
  inactiveSelection: string;
  keyword: string;
  class: string;
  struct: string;
  function: string;
  interface: string;
  enum: string;
  variable: string;
  property: string;
  parameter: string;
  namespace: string;
  type: string;
  comment: string;
  string: string;
  number: string;
  operator: string;
};

const PALETTE: Palette = {
  // Text + chrome
  onSurface: '#dde2f2',
  onSurfaceVariant: '#c6c5d5',
  outline: '#908f9e',
  outlineVariant: '#454653',
  surface: '#0d131f',
  surfaceContainerLow: '#151c27',
  surfaceContainerLowest: '#080e19',
  selection: '#3b4252',
  inactiveSelection: '#2e3440',

  // SysML-kind colours
  keyword: '#bdc2ff', // --primary       — language reserved words
  class: '#a4c9ff', // --secondary     — Part/Item/Occurrence/Connection Defs
  struct: '#ffb95f', // --tertiary      — Requirement/Constraint/Case Defs
  function: '#34d399', // --sim-visited   — Action/State Defs & Usages (behaviour)
  interface: '#60a5fa', // --sim-available — Port/Interface Defs & Usages (boundary)
  enum: '#c084fc', // --sim-causal    — Enum Defs & Usages (discrete)
  variable: '#dde2f2', // --on-surface    — usage instances (parts, items)
  property: '#c6c5d5', // --on-surface-v  — attribute defs/usages
  parameter: '#fbbf24', // --sim-debug     — direction-bound parameters
  namespace: '#a78bfa', // violet 500      — Package / Namespace
  type: '#a4c9ff', // --secondary     — generic definition types
  comment: '#6b7280', // muted grey      — comments
  string: '#34d399', // green           — string literals
  number: '#ffb95f', // amber           — numeric literals
  operator: '#dde2f2', // --on-surface    — operators (neutral)
};

/**
 * Light "warm paper" variant. The dark syntax hues are tuned for a dark
 * ground and wash out on paper, so each is darkened/saturated to hold
 * contrast on the near-white editor surface (the same kind → hue meaning
 * is preserved, only the value shifts). Surfaces come from the light
 * ramp in tokens.css.
 */
const PALETTE_LIGHT: Palette = {
  onSurface: '#241c15', // ink
  onSurfaceVariant: '#4c3f2c',
  outline: '#9b8b74', // line numbers
  outlineVariant: '#d2c7b5', // indent guides
  surface: '#fbfaf6', // editor bg — warm paper
  surfaceContainerLow: '#f0eae0', // line highlight
  surfaceContainerLowest: '#f8f4ec', // gutter
  selection: '#e4dcce',
  inactiveSelection: '#ece5d8',

  keyword: '#4b45c0', // violet-blue
  class: '#1f5c9e', // blue
  struct: '#985e0f', // amber/brown (accent-700)
  function: '#1d7a54', // green
  interface: '#1e5f9e', // blue
  enum: '#7a3fb0', // purple
  variable: '#241c15', // ink
  property: '#4c3f2c',
  parameter: '#8a5a00', // amber
  namespace: '#6a3fb0', // violet
  type: '#1f5c9e',
  comment: '#7e6e58', // muted
  string: '#1d7a54', // green
  number: '#985e0f', // amber
  operator: '#241c15',
};

/** Stable theme ids consumed by `<Editor theme={...}>`. */
export const sysmlThemeId = 'sysml-engineering';
export const sysmlLightThemeId = 'sysml-engineering-light';

interface MonacoLike {
  editor: {
    defineTheme: (
      name: string,
      data: {
        base: 'vs' | 'vs-dark' | 'hc-black' | 'hc-light';
        inherit: boolean;
        rules: Array<{
          token: string;
          foreground?: string;
          background?: string;
          fontStyle?: string;
        }>;
        colors: Record<string, string>;
        /**
         * Required to opt this theme in to LSP semantic-token
         * colouring. Without it Monaco ignores semantic-token rules
         * entirely (the editor's
         * `'semanticHighlighting.enabled': true` option only
         * controls whether tokens are REQUESTED from providers;
         * application to colours is gated separately on the theme).
         */
        semanticHighlighting?: boolean;
      },
    ) => void;
    /**
     * @monaco-editor/react reads `theme={sysmlThemeId}` and calls
     * `setTheme` BEFORE onMount fires, so when `defineTheme` finally
     * lands the editor has already silently fallen back to `vs`. We
     * re-apply explicitly here.
     */
    setTheme?: (name: string) => void;
  };
}

let themeDefined = false;

/**
 * Register `sysml-engineering` with the given Monaco namespace. Safe
 * to call multiple times — subsequent calls are no-ops.
 *
 * Mounting the sneak-peek hover popups will trigger this in addition
 * to the live editor; the second call short-circuits via the
 * module-local `themeDefined` flag so we don't waste Monaco cycles
 * re-defining the same theme object.
 */
/** Build the semantic + Monarch colour rules for a palette. The token →
 *  hue MEANING is identical across themes; only the palette values differ. */
function buildRules(palette: Palette) {
  const lang = `.${sysmlLanguageId}`;
  const mk = (token: string, fg: string, fontStyle?: string) => ({
    token,
    foreground: fg.replace(/^#/, ''),
    ...(fontStyle ? { fontStyle } : {}),
  });
  return [
    // Hard-fail default: anything the LSP doesn't classify renders in
    // magenta so backend tokenizer gaps surface immediately.
    mk('', '#ff00ff'),

    // ── LSP semantic tokens (per legend type name) ───────────────
    mk('namespace', palette.namespace),
    mk(`namespace${lang}`, palette.namespace),
    mk('type', palette.type),
    mk(`type${lang}`, palette.type),
    mk('class', palette.class),
    mk(`class${lang}`, palette.class),
    mk('struct', palette.struct),
    mk(`struct${lang}`, palette.struct),
    mk('property', palette.property),
    mk(`property${lang}`, palette.property),
    mk('variable', palette.variable),
    mk(`variable${lang}`, palette.variable),
    mk('parameter', palette.parameter),
    mk(`parameter${lang}`, palette.parameter),
    mk('function', palette.function),
    mk(`function${lang}`, palette.function),
    mk('keyword', palette.keyword),
    mk(`keyword${lang}`, palette.keyword),
    // comment / string / number / operator need the PLAIN legend name to
    // match — that's the scope the LSP semantic tokens actually carry.
    // The `.sysml`-suffixed variants are legacy Monarch scopes (the
    // tokenizer was deleted), so a plain-name rule is the only one that
    // fires; without it these fall through to the magenta hard-fail
    // default even though the server classifies them correctly.
    mk('comment', palette.comment, 'italic'),
    mk(`comment${lang}`, palette.comment, 'italic'),
    mk('string', palette.string),
    mk(`string${lang}`, palette.string),
    mk('number', palette.number),
    mk(`number${lang}`, palette.number),
    mk('operator', palette.operator),
    mk(`operator${lang}`, palette.operator),
    mk('interface', palette.interface),
    mk(`interface${lang}`, palette.interface),
    mk('enum', palette.enum),
    mk(`enum${lang}`, palette.enum),

    // Definition sites get bold so `part def Heater` stands out from a
    // `part heater : Heater` usage.
    mk(`class.definition${lang}`, palette.class, 'bold'),
    mk(`struct.definition${lang}`, palette.struct, 'bold'),
    mk(`function.definition${lang}`, palette.function, 'bold'),
    mk(`interface.definition${lang}`, palette.interface, 'bold'),
    mk(`enum.definition${lang}`, palette.enum, 'bold'),
    mk(`namespace.definition${lang}`, palette.namespace, 'bold'),

    // Unresolved references (legend modifier `unresolved`, carried by the
    // resolution-backed reference tokens when a name could NOT be resolved).
    // Dimmed to the muted outline hue + underlined so a broken reference reads
    // as "attempted, unresolved" without the magenta hard-fail — magenta stays
    // reserved for tokens the backend classifies not at all. Emitted today only
    // on VARIABLE (bare feature-reference misses); both the plain and
    // `.sysml`-suffixed scopes are covered so whichever Monaco matches fires.
    mk('variable.unresolved', palette.outline, 'underline'),
    mk(`variable.unresolved${lang}`, palette.outline, 'underline'),
  ];
}

function buildColors(palette: Palette): Record<string, string> {
  return {
    'editor.background': palette.surface,
    'editor.foreground': palette.onSurface,
    'editor.lineHighlightBackground': palette.surfaceContainerLow,
    'editor.lineHighlightBorder': '#00000000',
    'editorLineNumber.foreground': palette.outline,
    'editorLineNumber.activeForeground': palette.onSurface,
    'editorCursor.foreground': palette.keyword,
    'editor.selectionBackground': palette.selection,
    'editor.inactiveSelectionBackground': palette.inactiveSelection,
    'editorGutter.background': palette.surfaceContainerLowest,
    'editorIndentGuide.background': palette.outlineVariant,
    'editorIndentGuide.activeBackground': palette.outline,
    'editorWhitespace.foreground': palette.outlineVariant,
    'editorBracketMatch.background': palette.surfaceContainerLow,
    'editorBracketMatch.border': palette.outline,
  };
}

export function ensureSysmlTheme(monaco: MonacoLike): void {
  if (themeDefined) return;
  themeDefined = true;

  // Opt both themes in to LSP semantic-token application — without the
  // flag Monaco ignores every semantic rule and glyphs fall through to
  // the magenta default (the old "everything pinkish" symptom).
  monaco.editor.defineTheme(sysmlThemeId, {
    base: 'vs-dark',
    inherit: false,
    semanticHighlighting: true,
    rules: buildRules(PALETTE),
    colors: buildColors(PALETTE),
  });
  monaco.editor.defineTheme(sysmlLightThemeId, {
    base: 'vs',
    inherit: false,
    semanticHighlighting: true,
    rules: buildRules(PALETTE_LIGHT),
    colors: buildColors(PALETTE_LIGHT),
  });
}

/** The Monaco theme id for the current app theme. The editor follows the
 *  app's light/dark toggle (data-theme) like every other surface. */
export function sysmlThemeIdFor(appTheme: 'dark' | 'light'): string {
  return appTheme === 'light' ? sysmlLightThemeId : sysmlThemeId;
}

/** Apply the right Monaco theme now. @monaco-editor/react sets the theme
 *  prop BEFORE `ensureSysmlTheme` runs from onMount, so Monaco has already
 *  fallen back to a built-in; this re-activates our registered theme. */
export function applySysmlTheme(monaco: MonacoLike, appTheme: 'dark' | 'light'): void {
  monaco.editor.setTheme?.(sysmlThemeIdFor(appTheme));
}

/** Test-only: drop the cached registration so a fresh mount re-defines. */
export function _resetSysmlThemeForTesting(): void {
  themeDefined = false;
}
