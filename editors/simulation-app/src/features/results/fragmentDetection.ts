/**
 * Heuristic detection of combined-fragment patterns in state timelines.
 *
 * Since the backend TimelineEntry carries only { tick, timeMs, subsystems },
 * we infer loop / alt / opt / break patterns from repeated state sequences.
 *
 * Terminology follows UML combined fragments:
 *   - loop: consecutive repetitions of the same state sequence
 *   - alt:  divergence where two subsystems swap to different states at the same tick
 *   - opt:  a single subsystem briefly enters a state then returns (guard-like)
 *   - break: a subsystem enters a terminal/error state and stays there
 */

import type { TimelineEntry } from '../sessions/types';

// ── Public types ─────────────────────────────────────────────────────

export interface CollapsedLoop {
  /** Subsystem this loop applies to. */
  subsystem: string;
  /** Tick where the first iteration starts. */
  startTick: number;
  /** Tick where the last iteration ends (exclusive). */
  endTick: number;
  /** The repeating state sequence. */
  pattern: string[];
  /** Number of full repetitions detected. */
  iterations: number;
}

export type FragmentKind = 'alt' | 'opt' | 'break';

export interface FragmentBoundary {
  kind: FragmentKind;
  /** Tick at which the fragment begins. */
  tick: number;
  /** Subsystem(s) involved. */
  subsystems: string[];
  /** Human-readable label for the overlay. */
  label: string;
}

export interface TransitionTrigger {
  /** Tick at which the transition fires. */
  tick: number;
  /** Subsystem that transitions. */
  subsystem: string;
  /** State before. */
  fromState: string;
  /** State after. */
  toState: string;
  /** Inferred event name. */
  event: string;
}

// ── Loop detection ───────────────────────────────────────────────────

/**
 * Find repeated state-sequence runs per subsystem.
 *
 * Algorithm: for each subsystem, build a sequence of (state, tick) pairs.
 * Try pattern lengths 1..N/2; count consecutive repeats >= 3.
 */
export function detectLoops(entries: TimelineEntry[]): CollapsedLoop[] {
  if (entries.length < 4) return [];

  const subsystems = discoverSubsystems(entries);
  const loops: CollapsedLoop[] = [];

  for (const sub of subsystems) {
    // Build state sequence (skip numeric/ODE states)
    const seq: Array<{ state: string; tick: number }> = [];
    let prev = '';
    for (const e of entries) {
      const s = e.subsystems[sub];
      if (!s || !isNaN(parseFloat(s))) continue;
      if (s !== prev) {
        seq.push({ state: s, tick: e.tick });
        prev = s;
      }
    }

    if (seq.length < 4) continue;

    // Try pattern lengths from 1 up to half the sequence
    for (let patLen = 1; patLen <= Math.floor(seq.length / 2); patLen++) {
      let i = 0;
      while (i + patLen * 2 <= seq.length) {
        const pattern = seq.slice(i, i + patLen).map((s) => s.state);
        let reps = 1;
        let j = i + patLen;
        while (j + patLen <= seq.length) {
          const candidate = seq.slice(j, j + patLen).map((s) => s.state);
          if (candidate.every((s, idx) => s === pattern[idx])) {
            reps++;
            j += patLen;
          } else {
            break;
          }
        }
        if (reps >= 3) {
          const startTick = seq[i].tick;
          const endIdx = Math.min(i + patLen * reps, seq.length - 1);
          const endTick = seq[endIdx]?.tick ?? entries[entries.length - 1].tick;

          // Avoid overlapping with already-found longer patterns
          const overlaps = loops.some(
            (l) =>
              l.subsystem === sub &&
              l.startTick <= startTick &&
              l.endTick >= endTick,
          );
          if (!overlaps) {
            loops.push({
              subsystem: sub,
              startTick,
              endTick,
              pattern,
              iterations: reps,
            });
          }
          i = j; // skip past matched region
        } else {
          i++;
        }
      }
    }
  }

  return loops;
}

// ── Fragment boundary detection ──────────────────────────────────────

/**
 * Detect alt/opt/break patterns from timeline entries.
 *
 * - **alt**: two or more subsystems change state at the same tick
 *   (suggests an alternative/decision point).
 * - **opt**: a subsystem enters a state for exactly one tick then returns
 *   to the prior state (guard/optional behavior).
 * - **break**: a subsystem transitions to a state containing "error",
 *   "fault", "fail", "abort", or "final" and never leaves it.
 */
export function detectFragments(entries: TimelineEntry[]): FragmentBoundary[] {
  if (entries.length < 2) return [];

  const fragments: FragmentBoundary[] = [];
  const subsystems = discoverSubsystems(entries);
  const seenTicks = new Set<string>(); // dedup key: `${kind}:${tick}`

  // Build per-subsystem state timeline
  const perSub: Record<string, Array<{ tick: number; state: string }>> = {};
  for (const sub of subsystems) {
    perSub[sub] = [];
    let prev = '';
    for (const e of entries) {
      const s = e.subsystems[sub];
      if (!s || !isNaN(parseFloat(s))) continue;
      if (s !== prev) {
        perSub[sub].push({ tick: e.tick, state: s });
        prev = s;
      }
    }
  }

  // alt detection: multiple subsystems transitioning at the same tick
  const transitionsByTick: Record<number, string[]> = {};
  for (const sub of subsystems) {
    for (const t of perSub[sub]) {
      if (!transitionsByTick[t.tick]) transitionsByTick[t.tick] = [];
      transitionsByTick[t.tick].push(sub);
    }
  }
  for (const [tickStr, subs] of Object.entries(transitionsByTick)) {
    if (subs.length >= 2) {
      const tick = Number(tickStr);
      const key = `alt:${tick}`;
      if (!seenTicks.has(key)) {
        seenTicks.add(key);
        fragments.push({
          kind: 'alt',
          tick,
          subsystems: subs,
          label: 'alt',
        });
      }
    }
  }

  // opt detection: enter-then-return within 1-2 transitions
  for (const sub of subsystems) {
    const states = perSub[sub];
    for (let i = 1; i < states.length - 1; i++) {
      if (states[i - 1].state === states[i + 1].state && states[i].state !== states[i - 1].state) {
        const key = `opt:${states[i].tick}`;
        if (!seenTicks.has(key)) {
          seenTicks.add(key);
          fragments.push({
            kind: 'opt',
            tick: states[i].tick,
            subsystems: [sub],
            label: `opt [${states[i].state}]`,
          });
        }
      }
    }
  }

  // break detection: terminal states
  const terminalPatterns = /error|fault|fail|abort|final|done|halt/i;
  for (const sub of subsystems) {
    const states = perSub[sub];
    if (states.length < 2) continue;
    const last = states[states.length - 1];
    if (terminalPatterns.test(last.state)) {
      const key = `break:${last.tick}`;
      if (!seenTicks.has(key)) {
        seenTicks.add(key);
        fragments.push({
          kind: 'break',
          tick: last.tick,
          subsystems: [sub],
          label: `break [${last.state}]`,
        });
      }
    }
  }

  return fragments.sort((a, b) => a.tick - b.tick);
}

// ── Trigger annotation ───────────────────────────────────────────────

/**
 * Build trigger annotations for every state transition.
 *
 * Since the backend doesn't provide explicit event names, we synthesize
 * them from the transition shape:
 *   - Timeout pattern: entering a "timeout"/"expired" state -> "after(timeout)"
 *   - Completion: entering "done"/"final"/"completed" -> "completion"
 *   - Otherwise: "do / <fromState> -> <toState>"
 */
export function detectTriggers(entries: TimelineEntry[]): TransitionTrigger[] {
  if (entries.length < 2) return [];

  const triggers: TransitionTrigger[] = [];
  const subsystems = discoverSubsystems(entries);

  for (const sub of subsystems) {
    let prevState = '';
    for (const e of entries) {
      const s = e.subsystems[sub];
      if (!s || !isNaN(parseFloat(s))) continue;
      if (s !== prevState && prevState !== '') {
        triggers.push({
          tick: e.tick,
          subsystem: sub,
          fromState: prevState,
          toState: s,
          event: inferEventName(prevState, s),
        });
      }
      prevState = s;
    }
  }

  return triggers;
}

// ── Helpers ──────────────────────────────────────────────────────────

function discoverSubsystems(entries: TimelineEntry[]): string[] {
  const names: string[] = [];
  for (const e of entries) {
    for (const name of Object.keys(e.subsystems)) {
      if (!names.includes(name)) names.push(name);
    }
  }
  // Filter to state-machine subsystems (non-numeric states)
  return names.filter((name) => {
    const first = entries.find((e) => e.subsystems[name])?.subsystems[name] ?? '';
    return isNaN(parseFloat(first));
  });
}

function inferEventName(from: string, to: string): string {
  const lower = to.toLowerCase();
  if (/timeout|expired/.test(lower)) return 'after(timeout)';
  if (/done|final|completed/.test(lower)) return 'completion';
  if (/error|fault|fail/.test(lower)) return `error [${to}]`;
  if (/idle|init|reset/.test(lower)) return 'reset';
  // Default: use a transition-style label
  return `${from} \u2192 ${to}`;
}
