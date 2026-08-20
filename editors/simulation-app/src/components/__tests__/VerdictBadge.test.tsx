import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import {
  VerdictBadge,
  VerdictBadgeFromBool,
  buildVerdictTooltip,
  normalizeVerdict,
} from '../VerdictBadge';

afterEach(() => {
  cleanup();
});

describe('VerdictBadge — visual states', () => {
  it('renders pass with emerald check glyph and Pass label', () => {
    render(<VerdictBadge verdict="pass" actual="42" />);
    const badge = screen.getByTestId('verdict-badge-pass');
    expect(badge).toHaveAttribute('data-verdict', 'pass');
    expect(badge).toHaveAttribute('data-verdict-shape', 'check');
    expect(badge).toHaveTextContent('\u2713');
    expect(badge).toHaveTextContent('Pass');
  });

  it('renders fail with red cross glyph and Fail label', () => {
    render(<VerdictBadge verdict="fail" actual="10" expected=">= 100" />);
    const badge = screen.getByTestId('verdict-badge-fail');
    expect(badge).toHaveAttribute('data-verdict', 'fail');
    expect(badge).toHaveAttribute('data-verdict-shape', 'cross');
    expect(badge).toHaveTextContent('\u2717');
    expect(badge).toHaveTextContent('Fail');
  });

  it('renders inconclusive with neutral question glyph and Inconclusive label', () => {
    render(<VerdictBadge verdict="inconclusive" reason="Expression evaluated non-boolean" />);
    const badge = screen.getByTestId('verdict-badge-inconclusive');
    expect(badge).toHaveAttribute('data-verdict', 'inconclusive');
    expect(badge).toHaveAttribute('data-verdict-shape', 'question');
    expect(badge).toHaveTextContent('?');
    expect(badge).toHaveTextContent('Inconclusive');
  });

  it('renders error with neutral triangle glyph and Error label (distinct shape from Fail)', () => {
    render(<VerdictBadge verdict="error" reason="division by zero" />);
    const badge = screen.getByTestId('verdict-badge-error');
    expect(badge).toHaveAttribute('data-verdict', 'error');
    // Distinct shape from the red cross → a11y + colorblind safety
    expect(badge).toHaveAttribute('data-verdict-shape', 'triangle');
    expect(badge).toHaveTextContent('\u26A0');
    expect(badge).toHaveTextContent('Error');
  });

  it('error and fail use different glyphs and different shapes', () => {
    const { container: cFail } = render(<VerdictBadge verdict="fail" />);
    cleanup();
    const { container: cErr } = render(<VerdictBadge verdict="error" reason="oops" />);
    // Different text content means different glyph even under monochrome render.
    expect(cFail.textContent).not.toBe(cErr.textContent);
  });
});

describe('VerdictBadge — tooltip and a11y', () => {
  it('pass tooltip includes actual value', () => {
    render(<VerdictBadge verdict="pass" actual="42" />);
    const badge = screen.getByTestId('verdict-badge-pass');
    expect(badge).toHaveAttribute('title', expect.stringContaining('Pass'));
    expect(badge).toHaveAttribute('title', expect.stringContaining('actual: 42'));
  });

  it('fail tooltip includes actual and expected', () => {
    render(<VerdictBadge verdict="fail" actual="1" expected="2" />);
    const badge = screen.getByTestId('verdict-badge-fail');
    const title = badge.getAttribute('title')!;
    expect(title).toMatch(/Fail/);
    expect(title).toContain('actual: 1');
    expect(title).toContain('expected: 2');
  });

  it('inconclusive tooltip uses reason when provided', () => {
    render(<VerdictBadge verdict="inconclusive" reason="custom reason" />);
    const badge = screen.getByTestId('verdict-badge-inconclusive');
    expect(badge).toHaveAttribute('title', expect.stringContaining('custom reason'));
  });

  it('inconclusive tooltip falls back when no reason', () => {
    render(<VerdictBadge verdict="inconclusive" />);
    const badge = screen.getByTestId('verdict-badge-inconclusive');
    expect(badge).toHaveAttribute('title', expect.stringContaining('Expression evaluated non-boolean'));
  });

  it('error tooltip surfaces metadata.error_reason verbatim', () => {
    render(<VerdictBadge verdict="error" reason="NaN in constraint expression" />);
    const badge = screen.getByTestId('verdict-badge-error');
    expect(badge).toHaveAttribute('title', expect.stringContaining('NaN in constraint expression'));
  });

  it('error tooltip falls back when no reason', () => {
    render(<VerdictBadge verdict="error" />);
    const badge = screen.getByTestId('verdict-badge-error');
    expect(badge).toHaveAttribute('title', expect.stringContaining('Constraint could not be evaluated'));
  });

  it('has role=status with aria-label carrying verdict + reason', () => {
    render(<VerdictBadge verdict="error" reason="bad input" name="myConstraint" />);
    const badge = screen.getByTestId('verdict-badge-error');
    expect(badge).toHaveAttribute('role', 'status');
    const ariaLabel = badge.getAttribute('aria-label')!;
    expect(ariaLabel).toMatch(/Error/);
    expect(ariaLabel).toContain('bad input');
    expect(ariaLabel).toContain('myConstraint');
  });

  it('pass badge has aria-live off (does not interrupt screen readers)', () => {
    render(<VerdictBadge verdict="pass" />);
    const badge = screen.getByTestId('verdict-badge-pass');
    expect(badge).toHaveAttribute('aria-live', 'off');
  });
});

describe('VerdictBadge — size variants', () => {
  it('compact size hides label by default', () => {
    render(<VerdictBadge verdict="pass" size="compact" actual="1" />);
    const badge = screen.getByTestId('verdict-badge-pass');
    expect(badge).not.toHaveTextContent('Pass');
    expect(badge).toHaveTextContent('\u2713');
  });

  it('standard size shows the label', () => {
    render(<VerdictBadge verdict="pass" size="standard" />);
    const badge = screen.getByTestId('verdict-badge-pass');
    expect(badge).toHaveTextContent('Pass');
  });

  it('compact error row includes an sr-only affix with "!" for screen readers', () => {
    render(<VerdictBadge verdict="error" size="compact" reason="oops" />);
    const badge = screen.getByTestId('verdict-badge-error');
    // The affix lives inside the badge.
    expect(badge.textContent).toContain('!');
    expect(badge.textContent).toContain('oops');
  });

  it('compact inconclusive row includes sr-only "?" affix', () => {
    render(<VerdictBadge verdict="inconclusive" size="compact" reason="non-boolean" />);
    const badge = screen.getByTestId('verdict-badge-inconclusive');
    expect(badge.textContent).toContain('non-boolean');
  });

  it('explicit showLabel overrides compact default', () => {
    render(<VerdictBadge verdict="fail" size="compact" showLabel />);
    const badge = screen.getByTestId('verdict-badge-fail');
    expect(badge).toHaveTextContent('Fail');
  });
});

describe('buildVerdictTooltip helper', () => {
  it('builds pass tooltip', () => {
    expect(buildVerdictTooltip('pass', { actual: '1' })).toBe('Pass — actual: 1');
  });

  it('builds fail tooltip with both values', () => {
    expect(buildVerdictTooltip('fail', { actual: '1', expected: '2' })).toContain('actual: 1');
  });

  it('prefixes name when provided', () => {
    expect(buildVerdictTooltip('pass', { name: 'c1', actual: '1' })).toMatch(/^c1: Pass/);
  });

  it('falls back for inconclusive without reason', () => {
    expect(buildVerdictTooltip('inconclusive')).toContain('Expression evaluated non-boolean');
  });

  it('falls back for error without reason', () => {
    expect(buildVerdictTooltip('error')).toContain('Constraint could not be evaluated');
  });
});

describe('normalizeVerdict helper', () => {
  it('passes known kinds through', () => {
    expect(normalizeVerdict('Pass')).toBe('pass');
    expect(normalizeVerdict('FAIL')).toBe('fail');
    expect(normalizeVerdict('Inconclusive')).toBe('inconclusive');
    expect(normalizeVerdict('Error')).toBe('error');
  });

  it('maps unknown/empty to inconclusive (never silently drop)', () => {
    expect(normalizeVerdict(undefined)).toBe('inconclusive');
    expect(normalizeVerdict(null)).toBe('inconclusive');
    expect(normalizeVerdict('')).toBe('inconclusive');
    expect(normalizeVerdict('bogus')).toBe('inconclusive');
  });
});

describe('VerdictBadgeFromBool legacy adapter', () => {
  it('true → pass', () => {
    render(<VerdictBadgeFromBool pass actual="42" />);
    expect(screen.getByTestId('verdict-badge-pass')).toBeInTheDocument();
  });

  it('false → fail', () => {
    render(<VerdictBadgeFromBool pass={false} actual="0" expected=">= 1" />);
    expect(screen.getByTestId('verdict-badge-fail')).toBeInTheDocument();
  });
});
