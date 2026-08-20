/**
 * EvaluationModeBadge — the B10 layer-2 mode primitive (§2.1a(d)).
 *
 * Locks the Verify design loop's GEOMETRY-is-the-channel treatment (1d):
 * static is BARE (no container), trajectory is a SOLID square record,
 * external is a DASHED square provenance record, staleness rides the badge
 * in the warning family, and an absent/unknown mode renders nothing (never
 * fabricated). Modes never wear verdict colours.
 */
import { render, screen, cleanup } from '@testing-library/react';
import { describe, it, expect, afterEach } from 'vitest';
import {
  EvaluationModeBadge,
  EvaluationModeBadgeFromRaw,
  DeclaredComputedPair,
  normalizeEvaluationMode,
  evaluationModeTooltip,
} from '../EvaluationModeBadge';

// This project's vitest config does not auto-cleanup between tests, so
// multi-render tests would accumulate DOM and collide on testIds.
afterEach(() => cleanup());

describe('normalizeEvaluationMode', () => {
  it('accepts the three modes case-insensitively', () => {
    expect(normalizeEvaluationMode('static')).toBe('static');
    expect(normalizeEvaluationMode('TRAJECTORY')).toBe('trajectory');
    expect(normalizeEvaluationMode('External')).toBe('external');
  });

  it('returns null for absent or unknown values (never fabricates a mode)', () => {
    expect(normalizeEvaluationMode(null)).toBeNull();
    expect(normalizeEvaluationMode(undefined)).toBeNull();
    expect(normalizeEvaluationMode('')).toBeNull();
    expect(normalizeEvaluationMode('vibes')).toBeNull();
  });
});

describe('EvaluationModeBadge', () => {
  it('renders a distinct badge per mode with the mode word', () => {
    render(<EvaluationModeBadge mode="static" />);
    render(<EvaluationModeBadge mode="trajectory" />);
    render(<EvaluationModeBadge mode="external" />);
    expect(screen.getByTestId('evaluation-mode-badge-static').textContent).toContain('static');
    expect(screen.getByTestId('evaluation-mode-badge-trajectory').textContent).toContain('trajectory');
    expect(screen.getByTestId('evaluation-mode-badge-external').textContent).toContain('external');
  });

  it('tooltips explain what each mode MEANS, not just the word', () => {
    render(<EvaluationModeBadge mode="static" />);
    render(<EvaluationModeBadge mode="trajectory" />);
    render(<EvaluationModeBadge mode="external" />);
    expect(screen.getByTestId('evaluation-mode-badge-static').getAttribute('title')).toContain(
      'current/default values',
    );
    expect(screen.getByTestId('evaluation-mode-badge-trajectory').getAttribute('title')).toContain(
      'live simulation run',
    );
    // The B10 hard line: external is provenance, not a computed verdict.
    expect(screen.getByTestId('evaluation-mode-badge-external').getAttribute('title')).toContain(
      'produced outside the tool',
    );
  });

  it('geometry is the channel: static is BARE, trajectory SOLID, external DASHED', () => {
    render(<EvaluationModeBadge mode="static" />);
    render(<EvaluationModeBadge mode="trajectory" />);
    render(<EvaluationModeBadge mode="external" />);
    // jsdom keeps the `border` shorthand un-decomposed, so assert on the
    // inline style string.
    const staticStyle = screen.getByTestId('evaluation-mode-badge-static').getAttribute('style') ?? '';
    const trajStyle = screen.getByTestId('evaluation-mode-badge-trajectory').getAttribute('style') ?? '';
    const externalStyle = screen.getByTestId('evaluation-mode-badge-external').getAttribute('style') ?? '';

    // Static is weightless — no record, no border edge (CSSOM drops the
    // `border: none` declaration entirely, so assert no border edge at all).
    expect(staticStyle).not.toContain('solid');
    expect(staticStyle).not.toContain('dashed');
    // Records are square (radius 4), verdicts are round.
    expect(trajStyle).toContain('solid');
    expect(trajStyle).toContain('border-radius: 4px');
    // External reads as ingested-not-computed: dashed edge.
    expect(externalStyle).toContain('dashed');
    expect(externalStyle).toContain('border-radius: 4px');

    // No verdict colour token leaks into any mode badge.
    for (const s of [staticStyle, trajStyle, externalStyle]) {
      expect(s).not.toContain('--verdict-');
    }
  });

  it('names the record (session / tool) via recordRef on the standard chip', () => {
    render(<EvaluationModeBadge mode="trajectory" recordRef="S-1842" testId="traj" />);
    render(<EvaluationModeBadge mode="external" recordRef="hil-bench-2" testId="ext" />);
    expect(screen.getByTestId('traj').textContent).toContain('S-1842');
    expect(screen.getByTestId('ext').textContent).toContain('hil-bench-2');
  });

  it('renders the ⚑ older-model marker in the warning family when stale, never a verdict colour', () => {
    render(<EvaluationModeBadge mode="external" recordRef="hil-bench-2" stale testId="stale" />);
    const badge = screen.getByTestId('stale');
    expect(badge.getAttribute('data-mode-stale')).toBe('true');
    const marker = screen.getByTestId('evaluation-mode-stale');
    expect(marker.textContent).toContain('older model');
    const markerStyle = marker.getAttribute('style') ?? '';
    expect(markerStyle).toContain('--severity-warning');
    expect(markerStyle).not.toContain('--verdict-');
  });

  it('compact drops the container (bare text) but keeps the glyph + word + tooltip', () => {
    render(<EvaluationModeBadge mode="trajectory" size="compact" testId="m" />);
    const el = screen.getByTestId('m');
    // Compact flattens every mode to bare text for matrix/rollup density —
    // no record edge even for trajectory (CSSOM drops `border: none`).
    const style = el.getAttribute('style') ?? '';
    expect(style).not.toContain('solid');
    expect(style).not.toContain('dashed');
    expect(el.textContent).toContain('trajectory');
    expect(el.getAttribute('title')).toBe(evaluationModeTooltip('trajectory'));
  });

  it('compact does not render the record ref (density form)', () => {
    render(<EvaluationModeBadge mode="trajectory" size="compact" recordRef="S-1842" testId="c" />);
    expect(screen.getByTestId('c').textContent).not.toContain('S-1842');
  });
});

describe('EvaluationModeBadgeFromRaw', () => {
  it('renders nothing for an absent/unknown mode', () => {
    const { container } = render(<EvaluationModeBadgeFromRaw mode={null} />);
    expect(container.firstChild).toBeNull();
    const { container: c2 } = render(<EvaluationModeBadgeFromRaw mode="bogus" />);
    expect(c2.firstChild).toBeNull();
  });

  it('renders the badge for a valid raw string', () => {
    render(<EvaluationModeBadgeFromRaw mode="trajectory" testId="raw" />);
    expect(screen.getByTestId('raw').getAttribute('data-evaluation-mode')).toBe('trajectory');
  });
});

describe('DeclaredComputedPair', () => {
  it('renders both registers with their overline labels (two different questions)', () => {
    render(<DeclaredComputedPair methods={['inspect', 'test']} mode="trajectory" recordRef="S-1" />);
    const pair = screen.getByTestId('declared-computed-pair');
    expect(pair.textContent).toContain('DECLARED');
    expect(pair.textContent).toContain('COMPUTED ƒ');
    expect(screen.getByTestId('declared-methods').textContent).toBe('inspect · test');
    expect(screen.getByTestId('computed-mode').getAttribute('data-evaluation-mode')).toBe('trajectory');
  });

  it('shows the honest empty declared placeholder, never a defaulted method chip', () => {
    render(<DeclaredComputedPair methods={[]} mode="static" />);
    expect(screen.getByTestId('declared-methods-empty').textContent).toContain(
      'no @VerificationMethod declared',
    );
    expect(screen.queryByTestId('declared-methods')).toBeNull();
  });
});
