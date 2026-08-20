/**
 * fieldEdit splice logic — the client half of the §7.2/§7.5 buffer-
 * writeback contract. The guard is BINDING: a mismatch must never write.
 */

import { describe, expect, it } from 'vitest';
import {
  applyGuardedEdit,
  isValidRequirementName,
  lineColToOffset,
} from '../fieldEdit';
import type { WireTextEdit } from '../fieldEdit';

const SOURCE = ['package P {', '\trequirement r {', '\t\tdoc /* old text */', '\t}', '}', ''].join(
  '\n',
);

function edit(overrides: Partial<WireTextEdit>): WireTextEdit {
  return {
    line_start: 0,
    col_start: 0,
    line_end: 0,
    col_end: 0,
    new_text: '',
    ...overrides,
  };
}

describe('lineColToOffset', () => {
  it('resolves line starts, mid-line columns, and line ends', () => {
    expect(lineColToOffset(SOURCE, 0, 0)).toBe(0);
    expect(lineColToOffset(SOURCE, 1, 1)).toBe(SOURCE.indexOf('requirement'));
    // End-of-line position (col == line length) is valid: edits may end there.
    expect(lineColToOffset(SOURCE, 0, 'package P {'.length)).toBe('package P {'.length);
  });

  it('counts columns in UTF-16 units (astral chars are two)', () => {
    const text = 'a𝕏b\nnext';
    // '𝕏' is one surrogate pair = 2 UTF-16 units; col 3 lands on 'b'.
    expect(text.slice(lineColToOffset(text, 0, 3)!)).toMatch(/^b/);
    expect(lineColToOffset(text, 1, 0)).toBe(text.indexOf('next'));
  });

  it('returns null for positions outside the text', () => {
    expect(lineColToOffset(SOURCE, 99, 0)).toBeNull();
    // Column past the end of its line = divergence, not clamping.
    expect(lineColToOffset(SOURCE, 0, 999)).toBeNull();
  });
});

describe('applyGuardedEdit', () => {
  it('splices when the guard matches', () => {
    const line = 2; // "\t\tdoc /* old text */"
    const start = '\t\tdoc /*'.length;
    const result = applyGuardedEdit(
      SOURCE,
      edit({
        line_start: line,
        col_start: start,
        line_end: line,
        col_end: start + ' old text '.length,
        new_text: ' new text ',
        expected_old_text: ' old text ',
      }),
    );
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.next).toContain('doc /* new text */');
      expect(result.next).not.toContain('old text');
    }
  });

  it('fails loudly on guard mismatch and writes nothing', () => {
    const result = applyGuardedEdit(
      SOURCE,
      edit({
        line_start: 2,
        col_start: 0,
        line_end: 2,
        col_end: 4,
        new_text: 'X',
        expected_old_text: 'not what is there',
      }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toContain('stale buffer');
  });

  it('fails when the edit range no longer exists', () => {
    const result = applyGuardedEdit(
      'one line only',
      edit({ line_start: 5, col_start: 0, line_end: 5, col_end: 1, new_text: 'X' }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toContain('no longer exists');
  });

  it('applies guard-less insertions (expected_old_text absent)', () => {
    const result = applyGuardedEdit(
      SOURCE,
      edit({
        line_start: 3,
        col_start: 0,
        line_end: 3,
        col_end: 0,
        new_text: '\t\t@StatusInfo { status = StatusKind::tbd; }\n',
      }),
    );
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.next).toContain('@StatusInfo');
  });
});

describe('isValidRequirementName', () => {
  it('accepts identifiers, rejects everything else', () => {
    expect(isValidRequirementName('TripTime')).toBe(true);
    expect(isValidRequirementName('_r2')).toBe(true);
    expect(isValidRequirementName('')).toBe(false);
    expect(isValidRequirementName('2fast')).toBe(false);
    expect(isValidRequirementName('has space')).toBe(false);
    expect(isValidRequirementName('semi;colon')).toBe(false);
  });
});
