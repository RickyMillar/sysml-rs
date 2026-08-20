/**
 * Tests for the Monte Carlo CSV export (R5.9).
 *
 * Covers the contract report / spreadsheet consumers lean on:
 *   - RFC 4180 escaping (quote, comma, CR, LF)
 *   - Deterministic column + row ordering regardless of input order
 *   - Header shape matches the contract: iteration, session_id, status,
 *     <param cols>, <metric cols>, verdict_overall
 *   - Pending / incomplete children emit blank cells rather than nulls
 *   - Overall verdict folds pass < inconclusive < error < fail
 *   - Filename shape is `monte-carlo-<batchId>-<YYYYMMDD-HHMMSS>.csv`
 */
import { describe, it, expect } from 'vitest';
import {
  exportMonteCarloCsv,
  escapeCsvField,
  monteCarloCsvFilename,
  __internals,
} from '../exportMonteCarloCsv';
import type { ChildDescriptor } from '../passRateHelpers';

describe('escapeCsvField', () => {
  it('returns empty string for empty input', () => {
    expect(escapeCsvField('')).toBe('');
  });

  it('leaves simple fields unquoted', () => {
    expect(escapeCsvField('hello')).toBe('hello');
    expect(escapeCsvField('123.45')).toBe('123.45');
  });

  it('wraps fields containing commas', () => {
    expect(escapeCsvField('a,b')).toBe('"a,b"');
  });

  it('wraps and doubles embedded quotes', () => {
    expect(escapeCsvField('say "hi"')).toBe('"say ""hi"""');
  });

  it('wraps fields containing CR or LF', () => {
    expect(escapeCsvField('line1\nline2')).toBe('"line1\nline2"');
    expect(escapeCsvField('line1\r\nline2')).toBe('"line1\r\nline2"');
  });
});

describe('exportMonteCarloCsv — header + empty input', () => {
  it('header-only on empty batch', () => {
    const csv = exportMonteCarloCsv([]);
    expect(csv).toBe('iteration,session_id,status,verdict_overall\r\n');
  });

  it('uses RFC 4180 CRLF line terminators', () => {
    const csv = exportMonteCarloCsv([
      { index: 0, status: 'complete', session_id: 's0' },
    ]);
    // One header + one row + trailing terminator → 2 CRLFs
    const matches = csv.match(/\r\n/g) ?? [];
    expect(matches.length).toBe(2);
    expect(csv.endsWith('\r\n')).toBe(true);
  });
});

describe('exportMonteCarloCsv — column and row ordering', () => {
  const children: ChildDescriptor[] = [
    // Intentionally out of order to exercise the sort.
    {
      index: 2,
      session_id: 's2',
      status: 'complete',
      params: { gain: 0.8, setpoint: 12 },
      metrics: { rise_time: 1.3 },
      verdicts: [{ verdict: 'pass', id: 'c1' }],
    },
    {
      index: 0,
      session_id: 's0',
      status: 'complete',
      params: { gain: 0.5, setpoint: 10 },
      metrics: { rise_time: 1.1, overshoot: 0.02 },
      verdicts: [{ verdict: 'pass', id: 'c1' }],
    },
    {
      index: 1,
      session_id: null,
      status: 'pending',
      // No params, no metrics, no verdicts — blank everywhere.
    },
  ];

  it('sorts columns alphabetically and rows by iteration index', () => {
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    expect(lines[0]).toBe(
      'iteration,session_id,status,gain,setpoint,overshoot,rise_time,verdict_overall',
    );
    // Rows in ascending index order (0, 1, 2)
    expect(lines[1].startsWith('0,s0,complete,')).toBe(true);
    expect(lines[2].startsWith('1,,pending,')).toBe(true);
    expect(lines[3].startsWith('2,s2,complete,')).toBe(true);
  });

  it('pending rows emit blanks for absent params / metrics / verdict', () => {
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    // index 1 is pending — every cell after status should be blank except
    // the trailing verdict_overall column (also blank).
    expect(lines[2]).toBe('1,,pending,,,,,');
  });

  it('iteration 2 has a gain but no overshoot → blank cell', () => {
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    // Header columns: iteration,session_id,status,gain,setpoint,overshoot,rise_time,verdict_overall
    // Row for index 2: 2,s2,complete,0.8,12,,1.3,pass
    expect(lines[3]).toBe('2,s2,complete,0.8,12,,1.3,pass');
  });
});

describe('exportMonteCarloCsv — escaping inside rows', () => {
  it('escapes commas, quotes, and newlines inside string values', () => {
    const children: ChildDescriptor[] = [
      {
        index: 0,
        session_id: 'has,comma',
        status: 'complete',
        params: { note: 'quoted "value"' },
        metrics: { msg: 'line1\nline2' },
      },
    ];
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    // session_id has a comma → must be quoted
    expect(lines[1]).toContain('"has,comma"');
    // param note has quotes → must double + wrap
    expect(lines[1]).toContain('"quoted ""value"""');
    // metric msg has newline → must be wrapped (may span multiple split chunks)
    expect(csv).toContain('"line1\nline2"');
  });

  it('drops non-finite numbers to blank', () => {
    const children: ChildDescriptor[] = [
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        metrics: { bad: Number.POSITIVE_INFINITY, ok: 1.25 },
      },
    ];
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    // Columns: iteration, session_id, status, bad, ok, verdict_overall
    expect(lines[0]).toBe('iteration,session_id,status,bad,ok,verdict_overall');
    expect(lines[1]).toBe('0,s,complete,,1.25,');
  });

  it('serialises object and array values through JSON.stringify', () => {
    const children: ChildDescriptor[] = [
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        params: { cfg: { a: 1, b: 2 } },
        metrics: { shape: [3, 4, 5] },
      },
    ];
    const csv = exportMonteCarloCsv(children);
    const lines = csv.trim().split('\r\n');
    // The JSON has embedded quotes + a comma → must be wrapped + escaped.
    expect(lines[1]).toContain('"{""a"":1,""b"":2}"');
    expect(lines[1]).toContain('"[3,4,5]"');
  });
});

describe('exportMonteCarloCsv — verdict_overall folding', () => {
  it('pass when all verdicts pass', () => {
    const csv = exportMonteCarloCsv([
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        verdicts: [
          { verdict: 'pass', id: 'a' },
          { verdict: 'pass', id: 'b' },
        ],
      },
    ]);
    expect(csv.trim().split('\r\n')[1].endsWith(',pass')).toBe(true);
  });

  it('fail when any verdict fails (fail dominates error / inconclusive)', () => {
    const csv = exportMonteCarloCsv([
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        verdicts: [
          { verdict: 'pass', id: 'a' },
          { verdict: 'error', id: 'b' },
          { verdict: 'fail', id: 'c' },
        ],
      },
    ]);
    expect(csv.trim().split('\r\n')[1].endsWith(',fail')).toBe(true);
  });

  it('error when worst is error', () => {
    const csv = exportMonteCarloCsv([
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        verdicts: [
          { verdict: 'pass', id: 'a' },
          { verdict: 'error', id: 'b' },
          { verdict: 'inconclusive', id: 'c' },
        ],
      },
    ]);
    expect(csv.trim().split('\r\n')[1].endsWith(',error')).toBe(true);
  });

  it('inconclusive when worst is inconclusive', () => {
    const csv = exportMonteCarloCsv([
      {
        index: 0,
        session_id: 's',
        status: 'complete',
        verdicts: [
          { verdict: 'pass', id: 'a' },
          { verdict: 'inconclusive', id: 'b' },
        ],
      },
    ]);
    expect(csv.trim().split('\r\n')[1].endsWith(',inconclusive')).toBe(true);
  });

  it('blank when no verdicts emitted', () => {
    const csv = exportMonteCarloCsv([
      { index: 0, session_id: 's', status: 'complete' },
    ]);
    // Row: 0,s,complete,  (trailing blank cell for verdict_overall)
    expect(csv.trim().split('\r\n')[1]).toBe('0,s,complete,');
  });
});

describe('monteCarloCsvFilename', () => {
  it('formats with zero-padded date / time components', () => {
    const fixed = new Date(2026, 2, 3, 4, 5, 6); // 3 Mar 2026 04:05:06 local
    expect(monteCarloCsvFilename('b1', fixed)).toBe(
      'monte-carlo-b1-20260303-040506.csv',
    );
  });

  it('sanitizes slashes and other unsafe chars in the batch id', () => {
    const fixed = new Date(2026, 0, 1, 0, 0, 0);
    const name = monteCarloCsvFilename('batch/one:two space', fixed);
    // Slashes, colons, spaces collapsed to single dashes.
    expect(name).toBe('monte-carlo-batch-one-two-space-20260101-000000.csv');
  });

  it('falls back to "batch" when the id sanitises to empty', () => {
    const fixed = new Date(2026, 0, 1, 0, 0, 0);
    expect(monteCarloCsvFilename('', fixed)).toBe(
      'monte-carlo-batch-20260101-000000.csv',
    );
  });
});

describe('__internals.overallVerdict priority', () => {
  it('pass < inconclusive < error < fail', () => {
    const cases: Array<[string[], string]> = [
      [['pass', 'pass'], 'pass'],
      [['pass', 'inconclusive'], 'inconclusive'],
      [['pass', 'error'], 'error'],
      [['pass', 'fail'], 'fail'],
      [['inconclusive', 'error'], 'error'],
      [['inconclusive', 'fail'], 'fail'],
      [['error', 'fail'], 'fail'],
    ];
    for (const [verdicts, expected] of cases) {
      expect(
        __internals.overallVerdict({
          index: 0,
          status: 'complete',
          verdicts: verdicts.map((k, i) => ({
            verdict: k as 'pass' | 'fail' | 'error' | 'inconclusive',
            id: `v${i}`,
          })),
        }),
      ).toBe(expected);
    }
  });
});
