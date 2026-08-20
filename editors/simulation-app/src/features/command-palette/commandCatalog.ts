/**
 * Command catalog fetcher + cache.
 *
 * Pulls the full set of registered backend commands from `GET /commands`
 * (the REST mirror of `sysml_service::registered_command_metas()`). Result
 * is cached for the lifetime of the page so the Cmd-K palette opens
 * instantly on re-opens.
 */

import { httpGet, httpPost } from '@/shared/api/http';

// ── Types mirroring sysml-service command_meta.rs ────────────────────────

export interface ParamMeta {
  name: string;
  ty: string;
  required: boolean;
  description: string;
}

export type CommandCategory =
  | 'FileManagement'
  | 'Query'
  | 'Analysis'
  | 'Execution'
  | 'Visualization'
  | 'Storage'
  // Client-side UI actions (e.g. rail open/close/pin) that have no
  // backend command behind them — see `railCommands.ts`. Kept as a
  // distinct category so the picker list visually and semantically
  // separates them from `sysml.*` service commands.
  | 'Client';

export interface CommandMeta {
  name: string;
  category: CommandCategory;
  description: string;
  params: ParamMeta[];
  returns: string;
  stateful: boolean;
  /**
   * When set, selecting this command runs `clientAction()` directly
   * instead of the normal params-form → `POST /api/command` flow. Used
   * for pure client UI state (right-rail open/close/pin) that isn't a
   * backend service command. Absent (`undefined`) for every real
   * `sysml.*` catalog entry.
   */
  clientAction?: () => void;
  /**
   * Superseded by another command. Set by the backend's
   * `#[service_command(deprecated = true)]` flag.
   *
   * Deprecated commands stay registered and dispatchable — existing API
   * callers keep working — but they are HIDDEN from the palette list, which
   * is a user-facing surface. Before this flag existed the deprecation was
   * written into `description` ("[Deprecated: prefer sessions.create]"), so
   * the palette rendered an internal migration note to end users
   * (punch-list finding 31). Typing the command's exact full name still
   * reveals it, so the dev-console escape hatch survives.
   */
  deprecated?: boolean;
  /**
   * When set, selecting this command NAVIGATES to the given route
   * (react-router) instead of dispatching to the backend. Client-only,
   * like `clientAction`, but resolved inside the palette component
   * where the router context is available (a bare `clientAction` can't
   * call `useNavigate`). Runs after `clientAction` when both are set.
   */
  navigateTo?: string;
}

// ── In-memory cache ──────────────────────────────────────────────────────

let cached: CommandMeta[] | null = null;
let inflight: Promise<CommandMeta[]> | null = null;

/**
 * Fetch the full command catalog. Result is memoised — repeated calls
 * return the same promise while the first request is in flight, and the
 * same array once it resolves.
 *
 * Exposed as a bare function (not a react-query hook) so the palette's
 * keyboard shortcut can pre-warm the cache without mounting the modal.
 */
export async function fetchCommandCatalog(): Promise<CommandMeta[]> {
  if (cached) return cached;
  if (inflight) return inflight;

  inflight = httpGet<CommandMeta[]>('/commands').then(
    (list) => {
      cached = list;
      inflight = null;
      return list;
    },
    (err) => {
      inflight = null;
      throw err;
    },
  );
  return inflight;
}

/** Clear the in-memory cache. Exposed for tests. */
export function resetCommandCatalogCache(): void {
  cached = null;
  inflight = null;
}

/** Read the cached catalog without triggering a fetch. */
export function cachedCommandCatalog(): CommandMeta[] | null {
  return cached;
}

// ── Fuzzy matching ───────────────────────────────────────────────────────

/**
 * Score a command against a query string. Higher is better. Zero means
 * no match. The algorithm is intentionally simple:
 *
 * - Substring match in name beats match in description
 * - Exact prefix match outranks inner substring
 * - Matching every query token (space-separated) is required
 */
export function scoreCommand(cmd: CommandMeta, query: string): number {
  const q = query.trim().toLowerCase();
  if (!q) return 1; // no query: everything passes, ranking unchanged

  const name = cmd.name.toLowerCase();
  const desc = cmd.description.toLowerCase();
  const cat = cmd.category.toLowerCase();

  const tokens = q.split(/\s+/).filter(Boolean);
  let score = 0;
  for (const tok of tokens) {
    if (name.startsWith(tok)) {
      score += 100;
    } else if (name.includes(tok)) {
      score += 40;
    } else if (cat.includes(tok)) {
      score += 20;
    } else if (desc.includes(tok)) {
      score += 10;
    } else {
      // A single unmatched token disqualifies the command.
      return 0;
    }
  }
  return score;
}

/** Filter + sort commands by the query. Stable-ish by name on ties. */
export function filterCommands(
  catalog: CommandMeta[],
  query: string,
): CommandMeta[] {
  // Deprecated commands are hidden rather than dropped: typing the exact
  // full name still surfaces one, so a developer chasing a specific legacy
  // command can still reach it while nobody browsing the list is offered a
  // superseded entry (see `CommandMeta.deprecated`).
  const exact = query.trim().toLowerCase();
  const visible = catalog.filter(
    (c) => !c.deprecated || c.name.toLowerCase() === exact,
  );

  const scored = visible
    .map((c) => ({ cmd: c, score: scoreCommand(c, query) }))
    .filter((x) => x.score > 0);
  scored.sort((a, b) => b.score - a.score || a.cmd.name.localeCompare(b.cmd.name));
  return scored.map((x) => x.cmd);
}

// ── Parameter type classification ────────────────────────────────────────

export type ParamKind = 'string' | 'number' | 'boolean' | 'json';

/**
 * Classify a CommandMeta type string into a UI input kind.
 *
 * The backend type strings come straight from Rust signatures (e.g.
 * "String", "usize", "bool", "ElementKind?", "Vec<String>"). We do a
 * best-effort mapping; everything we can't confidently match falls
 * through to `json` (textarea with JSON validation).
 */
export function classifyParamType(ty: string): ParamKind {
  const t = ty.trim().replace(/\?$/, '').toLowerCase();

  if (t === 'bool' || t === 'boolean') return 'boolean';
  if (
    t === 'u8' || t === 'u16' || t === 'u32' || t === 'u64' || t === 'u128' ||
    t === 'i8' || t === 'i16' || t === 'i32' || t === 'i64' || t === 'i128' ||
    t === 'usize' || t === 'isize' ||
    t === 'f32' || t === 'f64' ||
    t === 'number' || t === 'integer' || t === 'float' || t === 'double'
  ) {
    return 'number';
  }
  if (t === 'string' || t === 'str' || t === '&str' || t === 'cow<str>' || t === 'pathbuf') {
    return 'string';
  }
  return 'json';
}

/** Whether a type string marks the parameter as optional (trailing '?'). */
export function isOptionalType(ty: string): boolean {
  return ty.trim().endsWith('?');
}

// ── Command execution ────────────────────────────────────────────────────

export interface CommandResult {
  ok: boolean;
  value?: unknown;
  error?: string;
  latencyMs: number;
}

/** Execute a command via the generic `POST /api/command` dispatch. */
export async function runCommand(
  name: string,
  params: Record<string, unknown>,
): Promise<CommandResult> {
  const started = performance.now();
  try {
    const value = await httpPost<unknown>('/api/command', { command: name, params });
    return { ok: true, value, latencyMs: performance.now() - started };
  } catch (err: unknown) {
    const message =
      err instanceof Error ? err.message : typeof err === 'string' ? err : JSON.stringify(err);
    return { ok: false, error: message, latencyMs: performance.now() - started };
  }
}

/**
 * Pull the session a command result identifies, if any.
 *
 * The palette is a *generic* command runner: it POSTs whatever you pick and
 * renders the JSON back. That is fine for queries, but it means a command that
 * creates a session tells the backend and never tells the app — which is how
 * the header ended up reading "no session" next to "1/80 sessions" after the
 * single most important action in the product (punch-list finding 28). Both
 * canonical creation paths (`useSessionController`, `SessionControl`) already
 * call `setActiveSession`; only the palette route skipped the lifecycle.
 *
 * Rather than hardcode a list of session-creating command names, this keys off
 * the catalog's own `returns` metadata, so it keeps working as commands are
 * added or renamed. Shapes seen on the wire today:
 *
 *   `string (session_key)`                  → `"de350677-…"`
 *   `(session_key: string, ExecutionSnapshot)` → `["de350677-…", {snapshot}]`
 *   `SessionSummary`                        → `{ id: "…", … }`
 *
 * Note the rule is "the session this command *talked about*", not "the session
 * it created" — `sessions.step`/`reset`/`resume` also return a SessionSummary,
 * and selecting the session you just operated on is right there too.
 */
export function sessionIdFromCommandResult(
  meta: Pick<CommandMeta, 'returns'> | null | undefined,
  value: unknown,
): string | null {
  const returns = meta?.returns ?? '';
  const nonEmpty = (v: unknown): string | null =>
    typeof v === 'string' && v.trim() !== '' ? v : null;

  if (/session_key/i.test(returns)) {
    if (typeof value === 'string') return nonEmpty(value);
    if (Array.isArray(value)) return nonEmpty(value[0]);
    if (value && typeof value === 'object') {
      return nonEmpty((value as Record<string, unknown>).session_key);
    }
    return null;
  }

  if (/SessionSummary/.test(returns) && value && typeof value === 'object' && !Array.isArray(value)) {
    return nonEmpty((value as Record<string, unknown>).id);
  }

  return null;
}
