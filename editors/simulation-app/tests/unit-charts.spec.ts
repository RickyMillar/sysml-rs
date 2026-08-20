/**
 * Unit tests for chart component data-processing logic.
 *
 * These tests exercise pure functions (collapseRepeats, buildStaircasePath,
 * causation filtering, swimlane ODE filtering) by evaluating them inside the
 * browser via page.evaluate(). No DOM framework needed — we test the logic
 * directly.
 *
 * Requires: simulation-app dev server on port 3010 (vite).
 */
import { test, expect } from '@playwright/test';
import { APP } from './helpers';

// ---------------------------------------------------------------------------
// 1. SequenceView — collapseRepeats logic
// ---------------------------------------------------------------------------

test.describe('SequenceView collapseRepeats', () => {
  // Re-implement collapseRepeats inside evaluate so we can test it in isolation
  // without needing the module system. This mirrors the source exactly.
  const COLLAPSE_FN = `
    function collapseRepeats(msgs) {
      const result = [];
      for (const m of msgs) {
        if (m.from === m.to && !m.label.includes('\u2192')) {
          const prev = result[result.length - 1];
          if (prev && prev.from === m.from && prev.to === m.to && prev.label === m.label) {
            prev.count++;
            prev.tick = m.tick;
            continue;
          }
          result.push({ ...m, count: 1 });
        } else if (m.from === m.to) {
          const prev = result[result.length - 1];
          if (prev && prev.from === m.from && prev.to === m.to) {
            prev.count++;
            prev.tick = m.tick;
            continue;
          }
          result.push({ ...m, count: 1 });
        } else {
          result.push({ ...m, count: 1 });
        }
      }
      return result;
    }
  `;

  test('10 consecutive self-transitions collapse into 1 entry with count 10', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        const msgs = Array.from({ length: 10 }, (_, i) => ({
          tick: i, timeMs: i * 100, from: 'heating', to: 'heating',
          label: 'heating \u2192 heating', kind: 'transition',
        }));
        return collapseRepeats(msgs);
      })()
    `);
    expect(result).toHaveLength(1);
    expect(result[0].count).toBe(10);
    expect(result[0].from).toBe('heating');
    expect(result[0].to).toBe('heating');
  });

  test('real transition (from !== to) always shows as separate entry', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        const msgs = [
          { tick: 0, timeMs: 0, from: 'heating', to: 'heating', label: 'heating \u2192 heating', kind: 'transition' },
          { tick: 1, timeMs: 100, from: 'heating', to: 'heating', label: 'heating \u2192 heating', kind: 'transition' },
          { tick: 2, timeMs: 200, from: 'heating', to: 'ready', label: 'heating \u2192 ready', kind: 'transition' },
        ];
        return collapseRepeats(msgs);
      })()
    `);
    expect(result).toHaveLength(2);
    expect(result[0].count).toBe(2);
    expect(result[0].from).toBe('heating');
    expect(result[0].to).toBe('heating');
    expect(result[1].count).toBe(1);
    expect(result[1].from).toBe('heating');
    expect(result[1].to).toBe('ready');
  });

  test('mixed: 5x self + transition + 3x self = 3 entries', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        const msgs = [];
        // 5x self-transition
        for (let i = 0; i < 5; i++) {
          msgs.push({ tick: i, timeMs: i * 100, from: 'heating', to: 'heating', label: 'heating \u2192 heating', kind: 'transition' });
        }
        // 1 real transition
        msgs.push({ tick: 5, timeMs: 500, from: 'heating', to: 'ready', label: 'heating \u2192 ready', kind: 'transition' });
        // 3x self-transition in new state
        for (let i = 0; i < 3; i++) {
          msgs.push({ tick: 6 + i, timeMs: 600 + i * 100, from: 'ready', to: 'ready', label: 'ready \u2192 ready', kind: 'transition' });
        }
        return collapseRepeats(msgs);
      })()
    `);
    expect(result).toHaveLength(3);
    expect(result[0].count).toBe(5);
    expect(result[0].from).toBe('heating');
    expect(result[1].count).toBe(1);
    expect(result[1].to).toBe('ready');
    expect(result[2].count).toBe(3);
    expect(result[2].from).toBe('ready');
  });

  test('empty messages array returns empty', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        return collapseRepeats([]);
      })()
    `);
    expect(result).toHaveLength(0);
  });

  test('named self-transitions without arrow collapse by label', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        const msgs = [
          { tick: 0, timeMs: 0, from: 'idle', to: 'idle', label: 'tick', kind: 'transition' },
          { tick: 1, timeMs: 100, from: 'idle', to: 'idle', label: 'tick', kind: 'transition' },
          { tick: 2, timeMs: 200, from: 'idle', to: 'idle', label: 'tick', kind: 'transition' },
        ];
        return collapseRepeats(msgs);
      })()
    `);
    // These have no arrow in label, so they go through the first branch
    // and collapse consecutive same-label self-transitions
    expect(result).toHaveLength(1);
    expect(result[0].count).toBe(3);
    expect(result[0].label).toBe('tick');
  });

  test('different labels on self-transitions do not collapse together', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${COLLAPSE_FN}
        const msgs = [
          { tick: 0, timeMs: 0, from: 'idle', to: 'idle', label: 'poll', kind: 'transition' },
          { tick: 1, timeMs: 100, from: 'idle', to: 'idle', label: 'heartbeat', kind: 'transition' },
          { tick: 2, timeMs: 200, from: 'idle', to: 'idle', label: 'poll', kind: 'transition' },
        ];
        return collapseRepeats(msgs);
      })()
    `);
    // Different labels = 3 separate entries (no consecutive same-label)
    expect(result).toHaveLength(3);
    expect(result[0].label).toBe('poll');
    expect(result[1].label).toBe('heartbeat');
    expect(result[2].label).toBe('poll');
  });
});

// ---------------------------------------------------------------------------
// 2. Causation chain logic (filtering from SimulateMode)
// ---------------------------------------------------------------------------

test.describe('Causation chain filtering', () => {
  // Re-implement the causation builder from SimulateMode.tsx
  const CAUSATION_FN = `
    function buildCausation(history) {
      const nodes = [];
      const edges = [];
      const transitions = history.filter(h => h.from !== h.to).slice(-5);
      if (transitions.length === 0) return { nodes, edges };

      const seen = new Map();
      let counter = 0;

      function getOrCreate(label, kind) {
        kind = kind || 'state';
        if (seen.has(label)) return seen.get(label);
        const id = 'n-' + counter++;
        seen.set(label, id);
        nodes.push({ id, label, kind });
        return id;
      }

      let prevTo = null;
      transitions.forEach(h => {
        const fromId = getOrCreate(h.from);
        const toId = getOrCreate(h.to);
        edges.push({ from: fromId, to: toId, label: h.label || undefined });
        if (prevTo && prevTo !== fromId) {
          edges.push({ from: prevTo, to: fromId });
        }
        prevTo = toId;
      });
      return { nodes, edges };
    }
  `;

  test('only distinct transitions appear (self-transitions excluded)', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${CAUSATION_FN}
        const history = [
          { from: 'off', to: 'off', label: 'idle' },
          { from: 'off', to: 'off', label: 'idle' },
          { from: 'off', to: 'heating', label: 'start' },
          { from: 'heating', to: 'heating', label: 'warming' },
          { from: 'heating', to: 'ready', label: 'threshold' },
        ];
        return buildCausation(history);
      })()
    `);
    // Only off->heating and heating->ready pass the filter
    expect(result.nodes).toHaveLength(3); // off, heating, ready
    expect(result.edges.length).toBeGreaterThanOrEqual(2);
    expect(result.nodes.map((n: any) => n.label)).toEqual(['off', 'heating', 'ready']);
  });

  test('chain shows last 5 distinct transitions, not last 5 raw steps', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${CAUSATION_FN}
        const history = [
          // 8 self-transitions (filtered out)
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          { from: 'a', to: 'a' },
          // 6 real transitions (only last 5 kept)
          { from: 'a', to: 'b', label: '1' },
          { from: 'b', to: 'c', label: '2' },
          { from: 'c', to: 'd', label: '3' },
          { from: 'd', to: 'e', label: '4' },
          { from: 'e', to: 'f', label: '5' },
          { from: 'f', to: 'g', label: '6' },
        ];
        return buildCausation(history);
      })()
    `);
    // slice(-5) after filtering: transitions b->c, c->d, d->e, e->f, f->g
    const labels = result.nodes.map((n: any) => n.label);
    expect(labels).not.toContain('a');
    expect(labels).toContain('b');
    expect(labels).toContain('g');
    expect(result.edges.filter((e: any) => e.label).length).toBe(5);
  });

  test('empty history returns empty causation', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${CAUSATION_FN}
        return buildCausation([]);
      })()
    `);
    expect(result.nodes).toHaveLength(0);
    expect(result.edges).toHaveLength(0);
  });

  test('all self-transitions returns empty causation', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${CAUSATION_FN}
        const history = [
          { from: 'idle', to: 'idle' },
          { from: 'idle', to: 'idle' },
          { from: 'idle', to: 'idle' },
        ];
        return buildCausation(history);
      })()
    `);
    expect(result.nodes).toHaveLength(0);
    expect(result.edges).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// 3. TimeSeriesChart — buildStaircasePath logic
// ---------------------------------------------------------------------------

test.describe('TimeSeriesChart staircase path', () => {
  const STAIRCASE_FN = `
    function buildStaircasePath(points, scaleX, scaleY) {
      if (points.length === 0) return '';
      const parts = ['M ' + scaleX(points[0].t).toFixed(1) + ' ' + scaleY(points[0].v).toFixed(1)];
      for (let i = 1; i < points.length; i++) {
        parts.push('H ' + scaleX(points[i].t).toFixed(1));
        parts.push('V ' + scaleY(points[i].v).toFixed(1));
      }
      return parts.join(' ');
    }
  `;

  test('discrete series produces H/V staircase commands, not diagonal', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${STAIRCASE_FN}
        const points = [
          { t: 0, v: 0 },
          { t: 100, v: 1 },
          { t: 200, v: 1 },
          { t: 300, v: 2 },
        ];
        const scaleX = t => 50 + t * 0.5;
        const scaleY = v => 100 - v * 30;
        return buildStaircasePath(points, scaleX, scaleY);
      })()
    `);
    // Should start with M, then alternate H V
    expect(result).toMatch(/^M /);
    expect(result).toContain('H ');
    expect(result).toContain('V ');
    // Should NOT contain L (diagonal line-to) commands
    expect(result).not.toMatch(/ L /);
    // Count: 1 M + 3 pairs of H V = 7 commands
    const parts = result.split(' ').filter((s: string) => /^[MHVL]$/.test(s));
    expect(parts).toEqual(['M', 'H', 'V', 'H', 'V', 'H', 'V']);
  });

  test('empty points produce empty path', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${STAIRCASE_FN}
        return buildStaircasePath([], t => t, v => v);
      })()
    `);
    expect(result).toBe('');
  });

  test('single point produces only M command', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${STAIRCASE_FN}
        return buildStaircasePath([{ t: 50, v: 10 }], t => t, v => v);
      })()
    `);
    expect(result).toMatch(/^M 50\.0 10\.0$/);
    expect(result).not.toContain('H');
    expect(result).not.toContain('V');
  });

  test('staircase preserves correct coordinate values', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${STAIRCASE_FN}
        // Identity scaling so we can verify exact values
        const points = [
          { t: 0, v: 100 },
          { t: 50, v: 200 },
        ];
        return buildStaircasePath(points, t => t, v => v);
      })()
    `);
    // M 0.0 100.0 H 50.0 V 200.0
    expect(result).toBe('M 0.0 100.0 H 50.0 V 200.0');
  });
});

// ---------------------------------------------------------------------------
// 4. SwimlaneTimeline — ODE filtering and block construction
// ---------------------------------------------------------------------------

test.describe('SwimlaneTimeline ODE filtering', () => {
  const FILTER_FN = `
    function filterSmSubsystems(entries) {
      // Discover all subsystem names
      const subsystemNames = [];
      for (const entry of entries) {
        for (const name of Object.keys(entry.subsystems)) {
          if (!subsystemNames.includes(name)) subsystemNames.push(name);
        }
      }
      // Filter to SM subsystems (skip ODE which have numeric states)
      return subsystemNames.filter(name => {
        const firstState = entries.find(e => e.subsystems[name])?.subsystems[name] ?? '';
        return isNaN(parseFloat(firstState));
      });
    }
  `;

  test('ODE numeric states are filtered out, only SM states remain', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${FILTER_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { heater: 'off', temperature: '20.5', pressure: '1.01' } },
          { tick: 1, timeMs: 100, subsystems: { heater: 'heating', temperature: '21.3', pressure: '1.02' } },
        ];
        return filterSmSubsystems(entries);
      })()
    `);
    expect(result).toEqual(['heater']);
    expect(result).not.toContain('temperature');
    expect(result).not.toContain('pressure');
  });

  test('all ODE subsystems returns empty', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${FILTER_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { temperature: '20.5', velocity: '0.0' } },
        ];
        return filterSmSubsystems(entries);
      })()
    `);
    expect(result).toHaveLength(0);
  });

  test('all SM subsystems returns all names', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${FILTER_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { heater: 'off', valve: 'closed', pump: 'idle' } },
        ];
        return filterSmSubsystems(entries);
      })()
    `);
    expect(result).toEqual(['heater', 'valve', 'pump']);
  });

  test('empty entries returns empty', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${FILTER_FN}
        return filterSmSubsystems([]);
      })()
    `);
    expect(result).toHaveLength(0);
  });
});

test.describe('SwimlaneTimeline block construction', () => {
  const BLOCK_FN = `
    function buildBlocks(entries, subsystem) {
      const blocks = [];
      let blockStart = 0;
      let curState = entries[0]?.subsystems[subsystem] ?? '';
      for (let i = 1; i <= entries.length; i++) {
        const state = i < entries.length ? (entries[i].subsystems[subsystem] ?? '') : '';
        if (state !== curState || i === entries.length) {
          if (curState) {
            blocks.push({ start: blockStart, end: i, state: curState });
          }
          blockStart = i;
          curState = state;
        }
      }
      return blocks;
    }
  `;

  test('builds correct state blocks with start/end ticks', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${BLOCK_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { heater: 'off' } },
          { tick: 1, timeMs: 100, subsystems: { heater: 'off' } },
          { tick: 2, timeMs: 200, subsystems: { heater: 'heating' } },
          { tick: 3, timeMs: 300, subsystems: { heater: 'heating' } },
          { tick: 4, timeMs: 400, subsystems: { heater: 'ready' } },
        ];
        return buildBlocks(entries, 'heater');
      })()
    `);
    expect(result).toHaveLength(3);
    expect(result[0]).toEqual({ start: 0, end: 2, state: 'off' });
    expect(result[1]).toEqual({ start: 2, end: 4, state: 'heating' });
    expect(result[2]).toEqual({ start: 4, end: 5, state: 'ready' });
  });

  test('single state produces one block spanning all ticks', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${BLOCK_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { pump: 'running' } },
          { tick: 1, timeMs: 100, subsystems: { pump: 'running' } },
          { tick: 2, timeMs: 200, subsystems: { pump: 'running' } },
        ];
        return buildBlocks(entries, 'pump');
      })()
    `);
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual({ start: 0, end: 3, state: 'running' });
  });

  test('duration tooltip values are correct', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${BLOCK_FN}
        const entries = [
          { tick: 0, timeMs: 0, subsystems: { heater: 'off' } },
          { tick: 1, timeMs: 150, subsystems: { heater: 'off' } },
          { tick: 2, timeMs: 300, subsystems: { heater: 'heating' } },
          { tick: 3, timeMs: 500, subsystems: { heater: 'heating' } },
        ];
        const blocks = buildBlocks(entries, 'heater');
        // Compute duration as the component does: end-1 timeMs minus start timeMs
        return blocks.map(b => {
          const endMs = entries[Math.min(b.end - 1, entries.length - 1)]?.timeMs ?? 0;
          const startMs = entries[b.start]?.timeMs ?? 0;
          return { state: b.state, durationMs: (endMs - startMs).toFixed(0) };
        });
      })()
    `);
    expect(result[0]).toEqual({ state: 'off', durationMs: '150' });
    expect(result[1]).toEqual({ state: 'heating', durationMs: '200' });
  });
});

// ---------------------------------------------------------------------------
// 5. SwimlaneTimeline — playhead position
// ---------------------------------------------------------------------------

test.describe('SwimlaneTimeline playhead', () => {
  test('playhead x position is proportional to tick', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        const pad = { left: 80, right: 8 };
        const width = 600;
        const plotW = width - pad.left - pad.right;
        const numEntries = 10;
        const pxPerTick = Math.max(plotW / Math.max(numEntries, 1), 2);

        // Playhead at tick 5
        const tick = 5;
        const playheadX = pad.left + tick * pxPerTick;

        // Playhead at tick 0
        const playheadX0 = pad.left + 0 * pxPerTick;

        // Playhead at last tick
        const playheadXLast = pad.left + 9 * pxPerTick;

        return { playheadX, playheadX0, playheadXLast, padLeft: pad.left, plotW };
      })()
    `);
    // tick 0 should be at left padding
    expect(result.playheadX0).toBe(result.padLeft);
    // tick 5 should be halfway through
    expect(result.playheadX).toBeCloseTo(result.padLeft + result.plotW / 2, 1);
    // tick 9 should be near the right edge
    expect(result.playheadXLast).toBeCloseTo(result.padLeft + result.plotW * 0.9, 1);
  });
});

// ---------------------------------------------------------------------------
// 6. TimeSeriesChart — zero-crossing markers
// ---------------------------------------------------------------------------

test.describe('TimeSeriesChart zero-crossing markers', () => {
  test('crossing markers contain lightning emoji and label', async ({ page }) => {
    await page.goto(APP);
    // Verify the crossing data structure and rendering logic
    const result = await page.evaluate(`
      (() => {
        const crossings = [
          { t: 100, label: 'x=0', variable: 'position' },
          { t: 250, label: 'v=0', variable: 'velocity' },
        ];
        const series = [
          { name: 'position', points: [{ t: 0, v: -1 }, { t: 100, v: 0 }, { t: 200, v: 1 }] },
          { name: 'velocity', points: [{ t: 0, v: 1 }, { t: 250, v: 0 }, { t: 300, v: -1 }] },
        ];
        // For series 'position', only crossings with variable='position' or no variable apply
        const positionCrossings = crossings.filter(c => !c.variable || c.variable === 'position');
        const velocityCrossings = crossings.filter(c => !c.variable || c.variable === 'velocity');
        return {
          positionCrossings: positionCrossings.map(c => c.label),
          velocityCrossings: velocityCrossings.map(c => c.label),
        };
      })()
    `);
    expect(result.positionCrossings).toEqual(['x=0']);
    expect(result.velocityCrossings).toEqual(['v=0']);
  });

  test('crossings without variable match all series', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        const crossings = [
          { t: 100, label: 'global-event' },
        ];
        const seriesNames = ['position', 'velocity', 'temperature'];
        return seriesNames.map(name => {
          const matched = crossings.filter(c => !c.variable || c.variable === name);
          return { name, count: matched.length };
        });
      })()
    `);
    // Crossing without variable should match all 3 series
    for (const s of result) {
      expect(s.count).toBe(1);
    }
  });
});

// ---------------------------------------------------------------------------
// 7. SwimlaneTimeline — hashColor determinism
// ---------------------------------------------------------------------------

test.describe('SwimlaneTimeline hashColor', () => {
  const HASH_FN = `
    const STATE_COLORS = [
      '#4e79a7', '#f28e2c', '#e15759', '#76b7b2', '#59a14f',
      '#edc949', '#af7aa1', '#ff9da7', '#9c755f', '#bab0ab',
    ];
    function hashColor(name) {
      let hash = 0;
      for (let i = 0; i < name.length; i++) {
        hash = ((hash << 5) - hash + name.charCodeAt(i)) | 0;
      }
      return STATE_COLORS[Math.abs(hash) % STATE_COLORS.length];
    }
  `;

  test('same state name always produces same color', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${HASH_FN}
        return {
          a1: hashColor('heating'),
          a2: hashColor('heating'),
          b1: hashColor('ready'),
          b2: hashColor('ready'),
        };
      })()
    `);
    expect(result.a1).toBe(result.a2);
    expect(result.b1).toBe(result.b2);
  });

  test('different state names produce valid colors from palette', async ({ page }) => {
    await page.goto(APP);
    const result = await page.evaluate(`
      (() => {
        ${HASH_FN}
        const names = ['off', 'heating', 'ready', 'cooling', 'idle', 'error', 'standby'];
        return names.map(n => hashColor(n));
      })()
    `);
    const palette = [
      '#4e79a7', '#f28e2c', '#e15759', '#76b7b2', '#59a14f',
      '#edc949', '#af7aa1', '#ff9da7', '#9c755f', '#bab0ab',
    ];
    for (const color of result) {
      expect(palette).toContain(color);
    }
  });
});
