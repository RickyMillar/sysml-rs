/**
 * Perf probe — quantify the streaming hot path at espresso-production-cell scale
 * (14k scalars, 1% tick churn). Used to size the Variables-pane fix and the
 * stdlib-filter deferral.
 *
 * Reports wall time for:
 *   - applyDelta()           — how long to fold one tick into the store snapshot
 *   - buildTree + flatten    — how long to produce the Variables-pane row list
 *   - CBOR encode/decode     — reality-check the wire cost at that scale
 */
import { describe, it } from 'vitest';
import { encode as cborEncode, decode as cborDecode } from 'cbor-x';
import {
  applyDelta,
  type NormalizedSnapshot,
  type DeltaFrame,
} from '../features/sessions/sessionLiveStore';
import {
  buildTree,
  flattenTree,
  filterEntries,
  type VariableEntry,
} from '../features/variables/VariableTree';

const VAR_COUNT = 14_000;
const CHURN_PER_TICK = 140; // 1%

function makeBase(): NormalizedSnapshot {
  const scalars: Record<string, number> = {};
  for (let i = 0; i < VAR_COUNT; i++) {
    scalars[`m${(i / 200) | 0}.v_${i}`] = i * 0.01;
  }
  return {
    tick: 0,
    time_ms: 0,
    completed: false,
    subsystems: {},
    scalar_vars: scalars,
    string_vars: {},
    constraint_results: [],
  };
}

function makeDelta(base: NormalizedSnapshot, n: number): DeltaFrame {
  const changed: Record<string, number> = {};
  const names = Object.keys(base.scalar_vars);
  for (let i = 0; i < n; i++) {
    const name = names[i % names.length];
    changed[name] = (base.scalar_vars[name] ?? 0) + 0.25;
  }
  return {
    tick: 1,
    time_ms: 10,
    completed: false,
    scalar_changed: changed,
  };
}

function measure(label: string, iters: number, fn: () => void): void {
  // Warm-up the JIT before measuring.
  for (let i = 0; i < 3; i++) fn();
  const start = performance.now();
  for (let i = 0; i < iters; i++) fn();
  const elapsed = performance.now() - start;
  // eslint-disable-next-line no-console
  console.log(
    `  ${label.padEnd(32)} ${(elapsed / iters).toFixed(2)} ms/iter  (${iters} iters, total ${elapsed.toFixed(1)} ms)`,
  );
}

describe('perf-stream-14k (reports only, no assertions)', () => {
  const base = makeBase();
  const delta = makeDelta(base, CHURN_PER_TICK);

  it('applyDelta at 14k scalars / 1% churn', () => {
    measure('applyDelta (immutable)', 200, () => {
      applyDelta(base, delta);
    });
  });

  it('CBOR encode / decode of a tick frame', () => {
    // eslint-disable-next-line no-console
    console.log(`  (fresh Hello base size refs below)`);
    let json = '';
    let cbor: Uint8Array = new Uint8Array();
    measure('JSON.stringify(delta)', 500, () => {
      json = JSON.stringify({ type: 'tick', delta });
    });
    measure('cborEncode(delta)', 500, () => {
      cbor = cborEncode({ type: 'tick', delta });
    });
    measure('JSON.parse(tick)', 500, () => {
      JSON.parse(json);
    });
    measure('cborDecode(tick)', 500, () => {
      cborDecode(cbor);
    });
    // eslint-disable-next-line no-console
    console.log(
      `  wire sizes                      JSON ${json.length} B   CBOR ${cbor.length} B`,
    );
  });

  it('CBOR encode of a fresh Hello (full 14k snapshot)', () => {
    const hello = {
      type: 'hello',
      schema_version: 'sysml-session-v1',
      session_id: 'sess',
      tick: 0,
      time_ms: 0,
      base,
    };
    let json = '';
    let cbor: Uint8Array = new Uint8Array();
    measure('JSON.stringify(hello 14k)', 20, () => {
      json = JSON.stringify(hello);
    });
    measure('cborEncode(hello 14k)', 20, () => {
      cbor = cborEncode(hello);
    });
    measure('JSON.parse(hello 14k)', 20, () => {
      JSON.parse(json);
    });
    measure('cborDecode(hello 14k)', 20, () => {
      cborDecode(cbor);
    });
    // eslint-disable-next-line no-console
    console.log(
      `  hello sizes                     JSON ${(json.length / 1024).toFixed(1)} KB   CBOR ${(cbor.length / 1024).toFixed(1)} KB`,
    );
  });

  it('Variables pane tree build + flatten at 14k entries', () => {
    const entries: VariableEntry[] = Object.entries(base.scalar_vars).map(
      ([name, value]) => ({ name, value, lastChangedTick: null }),
    );
    measure('filterEntries (no chip, no search)', 50, () => {
      filterEntries(entries, {});
    });
    measure('buildTree', 50, () => {
      buildTree(entries, {});
    });
    const tree = buildTree(entries, {});
    measure('flattenTree (all expanded)', 50, () => {
      flattenTree(tree, new Set());
    });
  });

  it('Variables pane: search-filtered subset', () => {
    const entries: VariableEntry[] = Object.entries(base.scalar_vars).map(
      ([name, value]) => ({ name, value, lastChangedTick: null }),
    );
    measure('buildTree (search "m3.")', 50, () => {
      buildTree(entries, { search: 'm3.' });
    });
  });
});
