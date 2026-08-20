/**
 * Pure-function tests for `filterDiagnostics` + helpers (R6.1).
 */

import { describe, expect, it } from 'vitest';
import {
  filterDiagnostics,
  groupDiagnosticsByUri,
  formatSpanLocation,
} from '../filterDiagnostics';
import {
  DEFAULT_DIAGNOSTICS_FILTER,
  type DiagnosticEntry,
  type DiagnosticsFilter,
  type SeverityFilter,
} from '../types';

function makeEntry(
  overrides: Partial<DiagnosticEntry['diagnostic']> & { uri?: string } = {},
): DiagnosticEntry {
  const { uri = 'file:///a.sysml', ...diagOverrides } = overrides;
  return {
    uri,
    diagnostic: {
      severity: 'error',
      message: 'default message',
      ...diagOverrides,
    },
  };
}

function withFilter(partial: Partial<DiagnosticsFilter>): DiagnosticsFilter {
  return { ...DEFAULT_DIAGNOSTICS_FILTER, ...partial };
}

describe('filterDiagnostics', () => {
  it('returns an empty list when the input is empty', () => {
    const result = filterDiagnostics([], { ...withFilter({}), activeUri: null });
    expect(result).toEqual([]);
  });

  it('passes everything with the default filter + workspace scope', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ severity: 'error', message: 'bad syntax' }),
      makeEntry({ severity: 'warning', message: 'unused import' }),
      makeEntry({ severity: 'info', message: 'deprecated call' }),
      makeEntry({ severity: 'hint', message: 'consider refactor' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({}),
      activeUri: null,
    });
    expect(result).toHaveLength(4);
  });

  it('drops diagnostics whose severity is masked off', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ severity: 'error', message: 'e1' }),
      makeEntry({ severity: 'warning', message: 'w1' }),
      makeEntry({ severity: 'info', message: 'i1' }),
      makeEntry({ severity: 'hint', message: 'h1' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({
        severity: {
          error: true,
          warning: false,
          info: false,
          hint: false,
        } as SeverityFilter,
      }),
      activeUri: null,
    });
    expect(result.map((e) => e.diagnostic.message)).toEqual(['e1']);
  });

  it('returns an empty list when every severity is off', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ severity: 'error' }),
      makeEntry({ severity: 'warning' }),
      makeEntry({ severity: 'info' }),
      makeEntry({ severity: 'hint' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({
        severity: {
          error: false,
          warning: false,
          info: false,
          hint: false,
        },
      }),
      activeUri: null,
    });
    expect(result).toEqual([]);
  });

  it('does case-insensitive substring search on the message', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ message: 'Unexpected token EOF' }),
      makeEntry({ message: 'undefined reference' }),
      makeEntry({ message: 'unused import' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ search: 'UNUSED' }),
      activeUri: null,
    });
    expect(result.map((e) => e.diagnostic.message)).toEqual(['unused import']);
  });

  it('also matches the diagnostic code on substring search', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ message: 'foo', code: 'E001' }),
      makeEntry({ message: 'bar', code: 'W002' }),
      makeEntry({ message: 'baz' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ search: 'e001' }),
      activeUri: null,
    });
    expect(result).toHaveLength(1);
    expect(result[0]!.diagnostic.code).toBe('E001');
  });

  it('narrows to the current file when scope = current-file', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ uri: 'file:///a.sysml', message: 'a-msg' }),
      makeEntry({ uri: 'file:///b.sysml', message: 'b-msg' }),
      makeEntry({ uri: 'file:///c.sysml', message: 'c-msg' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ scope: 'current-file' }),
      activeUri: 'file:///b.sysml',
    });
    expect(result).toHaveLength(1);
    expect(result[0]!.uri).toBe('file:///b.sysml');
  });

  it('includes diagnostics whose span.file matches the active URI under current-file scope', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({
        uri: 'file:///a.sysml',
        message: 'reference into b',
        span: { file: 'file:///b.sysml', start: 0, end: 5 },
      }),
      makeEntry({
        uri: 'file:///a.sysml',
        message: 'pure a issue',
      }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ scope: 'current-file' }),
      activeUri: 'file:///b.sysml',
    });
    expect(result).toHaveLength(1);
    expect(result[0]!.diagnostic.message).toBe('reference into b');
  });

  it('returns an empty list under current-file scope when activeUri is null', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ uri: 'file:///a.sysml' }),
      makeEntry({ uri: 'file:///b.sysml' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ scope: 'current-file' }),
      activeUri: null,
    });
    expect(result).toEqual([]);
  });

  it('returns every URI under workspace scope regardless of activeUri', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ uri: 'file:///a.sysml' }),
      makeEntry({ uri: 'file:///b.sysml' }),
      makeEntry({ uri: 'file:///c.sysml' }),
    ];
    const result = filterDiagnostics(entries, {
      ...withFilter({ scope: 'workspace' }),
      activeUri: 'file:///b.sysml',
    });
    expect(result).toHaveLength(3);
  });

  it('does not mutate the input array', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ severity: 'error' }),
      makeEntry({ severity: 'warning' }),
    ];
    const before = [...entries];
    filterDiagnostics(entries, {
      ...withFilter({
        severity: { error: true, warning: false, info: true, hint: true },
      }),
      activeUri: null,
    });
    expect(entries).toEqual(before);
  });
});

describe('groupDiagnosticsByUri', () => {
  it('preserves insertion order across URIs', () => {
    const entries: DiagnosticEntry[] = [
      makeEntry({ uri: 'file:///a.sysml', message: 'a1' }),
      makeEntry({ uri: 'file:///b.sysml', message: 'b1' }),
      makeEntry({ uri: 'file:///a.sysml', message: 'a2' }),
      makeEntry({ uri: 'file:///c.sysml', message: 'c1' }),
    ];
    const groups = groupDiagnosticsByUri(entries);
    expect(Array.from(groups.keys())).toEqual([
      'file:///a.sysml',
      'file:///b.sysml',
      'file:///c.sysml',
    ]);
    expect(groups.get('file:///a.sysml')!.map((e) => e.diagnostic.message)).toEqual([
      'a1',
      'a2',
    ]);
  });

  it('yields an empty map on empty input', () => {
    expect(groupDiagnosticsByUri([]).size).toBe(0);
  });
});

describe('formatSpanLocation', () => {
  it('returns null when span is undefined', () => {
    expect(formatSpanLocation(undefined)).toBeNull();
  });

  it('formats line:col when both are present', () => {
    expect(
      formatSpanLocation({ line: 12, col: 7, start: 100, end: 110 }),
    ).toBe('12:7');
  });

  it('formats line only when col is absent', () => {
    expect(formatSpanLocation({ line: 9, start: 0, end: 4 })).toBe('9');
  });

  it('falls back to byte offset when line and col are absent', () => {
    expect(formatSpanLocation({ start: 42, end: 80 })).toBe('@42');
  });
});
