/**
 * Cmd-K command palette (dev mode).
 *
 * Gated behind `import.meta.env.VITE_DEV_CMDK === '1'`. See
 * README notes or run with: `VITE_DEV_CMDK=1 npm run dev`.
 *
 * Public surface:
 *   - <CommandPalette /> — the modal itself, gated by the env flag
 *     internally via `isDevCmdKEnabled()`.
 *   - isDevCmdKEnabled() — lets callers decide whether to mount at all.
 */

export { CommandPalette } from './CommandPalette';
export { ParameterForm } from './ParameterForm';
export {
  fetchCommandCatalog,
  filterCommands,
  scoreCommand,
  classifyParamType,
  isOptionalType,
  runCommand,
  resetCommandCatalogCache,
  cachedCommandCatalog,
} from './commandCatalog';
export type {
  CommandMeta,
  ParamMeta,
  CommandCategory,
  ParamKind,
  CommandResult,
} from './commandCatalog';
export { useCmdKShortcut } from './useCmdKShortcut';

/**
 * Whether the Cmd-K palette should be mounted. Reads the `VITE_DEV_CMDK`
 * Vite env variable at build time. Any non-empty value other than "0",
 * "false" (case-insensitive) counts as enabled; the canonical form is
 * `VITE_DEV_CMDK=1`.
 *
 * Returns false in non-browser contexts (SSR, unit tests that don't
 * shim `import.meta.env`) so the palette cannot accidentally render
 * outside a dev session.
 */
export function isDevCmdKEnabled(): boolean {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const env = (import.meta as any).env as Record<string, unknown> | undefined;
    return parseDevCmdKFlag(env?.VITE_DEV_CMDK);
  } catch {
    return false;
  }
}

/**
 * Normalise a raw env flag value into a boolean. Exposed so unit tests
 * can verify parsing rules directly without fighting Vite's static
 * `import.meta.env` replacement.
 */
export function parseDevCmdKFlag(raw: unknown): boolean {
  if (raw === undefined || raw === null) return false;
  const s = String(raw).trim().toLowerCase();
  if (!s) return false;
  if (s === '0' || s === 'false' || s === 'off') return false;
  return true;
}
