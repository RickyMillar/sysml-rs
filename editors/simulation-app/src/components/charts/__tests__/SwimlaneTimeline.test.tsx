import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import { SwimlaneTimeline, type TimelineEntry } from '../SwimlaneTimeline';

afterEach(cleanup);

const UUID = 'aab68f93-b7c4-48c5-8934-e4793154f3d4';

describe('SwimlaneTimeline', () => {
  it('renders state-machine lanes with named states', () => {
    const entries: TimelineEntry[] = [
      { tick: 0, timeMs: 0, subsystems: { sm: 'idle' } },
      { tick: 1, timeMs: 10, subsystems: { sm: 'running' } },
      { tick: 2, timeMs: 20, subsystems: { sm: 'running' } },
    ];
    const { container } = render(<SwimlaneTimeline entries={entries} width={600} />);
    const texts = Array.from(container.querySelectorAll('text')).map((t) => t.textContent ?? '');
    expect(texts).toContain('sm');
    expect(texts.some((t) => t.includes('idle') || t.includes('running'))).toBe(true);
  });

  it('drops action/occurrence lanes whose state is an element UUID and never renders a UUID label', () => {
    // `act` is an action lane: its "state" is `<uuid>_initial` / `<uuid>_final`,
    // not a human state name. It must be filtered out, and no UUID text may
    // appear anywhere (regression for the State Timeline UUID leak).
    const entries: TimelineEntry[] = [
      { tick: 0, timeMs: 0, subsystems: { sm: 'idle', act: `${UUID}_initial` } },
      { tick: 1, timeMs: 10, subsystems: { sm: 'running', act: `${UUID}_final` } },
      { tick: 2, timeMs: 20, subsystems: { sm: 'running', act: `${UUID}_final` } },
    ];
    const { container } = render(<SwimlaneTimeline entries={entries} width={600} />);
    const texts = Array.from(container.querySelectorAll('text')).map((t) => t.textContent ?? '');

    // The real state-machine lane survives.
    expect(texts).toContain('sm');
    // The UUID-stated action lane is dropped — no lane label.
    expect(texts).not.toContain('act');
    // And no UUID leaks into any rendered label.
    expect(texts.some((t) => /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.test(t))).toBe(false);
  });

  it('skips ODE lanes with numeric states', () => {
    const entries: TimelineEntry[] = [
      { tick: 0, timeMs: 0, subsystems: { sm: 'idle', temp: '290.5' } },
      { tick: 1, timeMs: 10, subsystems: { sm: 'running', temp: '305.2' } },
    ];
    const { container } = render(<SwimlaneTimeline entries={entries} width={600} />);
    const texts = Array.from(container.querySelectorAll('text')).map((t) => t.textContent ?? '');
    expect(texts).toContain('sm');
    expect(texts).not.toContain('temp');
  });
});
