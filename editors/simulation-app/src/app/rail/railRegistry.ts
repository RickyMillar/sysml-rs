/**
 * railRegistry — the right-rail context registry (ninebar Phase 1).
 *
 * A "context" is a pluggable panel the right rail can host: inspector,
 * diagnostics, variables, breakpoints, causal-trace, … (plan §1 row 3 /
 * Phase 1 task list). Each context registers itself once, at module load,
 * via a side-effect import (see `contexts/index.ts`); `RightRail` only
 * ever knows ids via `useRightRailStore`, never concrete panel
 * components — that indirection is what lets later phases re-home a
 * resident panel into the rail without touching the host.
 *
 * `render` takes no arguments — see `railStore.ts`'s doc comment (F15): a
 * context is only ever mounted against the active session, so it reads
 * active-session hooks directly rather than receiving a session id.
 */
import type { ReactNode } from 'react';

export interface RailContextDescriptor {
  id: string;
  title: string;
  /** Material Symbols icon name, shown in the header row. Optional. */
  icon?: string;
  render: () => ReactNode;
}

const registry = new Map<string, RailContextDescriptor>();

export function registerRailContext(descriptor: RailContextDescriptor): void {
  registry.set(descriptor.id, descriptor);
}

/** Lookup helper — returns `undefined` when no context matches. */
export function getRailContext(id: string): RailContextDescriptor | undefined {
  return registry.get(id);
}
