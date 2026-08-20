/**
 * Suspect view-model mapping — pure-logic tests.
 *
 * The honesty-critical branch: `not_in_baseline` must map to
 * 'identity-changed' (ADR-009 — the diff cannot tell newly-authored from
 * scope-rename replacement, and the UI must never pretend otherwise).
 */

import { describe, expect, it } from 'vitest';
import { suspectsById, toSuspectRecord } from '../suspect';
import { projectIdForWorkspace } from '../queries';

describe('toSuspectRecord', () => {
  it('maps text_changed causes to before/after deltas', () => {
    const record = toSuspectRecord({
      requirement: 'r-1',
      causes: [
        { kind: 'text_changed', element: 'd-1', from: 'trip in 40 ms', to: 'trip in 25 ms' },
      ],
    });
    expect(record.kind).toBe('changed');
    expect(record.textDeltas).toEqual([{ from: 'trip in 40 ms', to: 'trip in 25 ms' }]);
  });

  it('maps prop_text_changed causes to labeled prop deltas (W4)', () => {
    const record = toSuspectRecord({
      requirement: 'r-1',
      causes: [
        {
          kind: 'prop_text_changed',
          element: 'c-1',
          element_kind: 'RequirementConstraintMembership',
          key: 'constraint',
          from: 'actualTime <= 40',
          to: 'actualTime <= 25',
        },
      ],
    });
    expect(record.kind).toBe('changed');
    expect(record.textDeltas).toEqual([]);
    expect(record.propDeltas).toEqual([
      {
        elementKind: 'RequirementConstraintMembership',
        key: 'constraint',
        from: 'actualTime <= 40',
        to: 'actualTime <= 25',
      },
    ]);
    expect(record.changeSummary).toContain('1 value changed');
  });

  it('maps not_in_baseline to identity-changed (ADR-009)', () => {
    const record = toSuspectRecord({
      requirement: 'r-2',
      causes: [{ kind: 'not_in_baseline' }],
    });
    expect(record.kind).toBe('identity-changed');
    expect(record.textDeltas).toEqual([]);
  });

  it('summarizes non-text causes for the popover fallback line', () => {
    const record = toSuspectRecord({
      requirement: 'r-3',
      causes: [
        { kind: 'content_changed', element: 'e-1', element_kind: 'RequirementUsage' },
        { kind: 'child_removed', element: 'e-2', element_kind: 'Documentation' },
        { kind: 'upstream_suspect', via: 'r-up' },
      ],
    });
    expect(record.kind).toBe('changed');
    expect(record.changeSummary).toContain('1 element changed');
    expect(record.changeSummary).toContain('1 nested element removed');
    expect(record.changeSummary).toContain('upstream requirement');
    expect(record.upstreamVia).toEqual(['r-up']);
  });
});

describe('suspectsById', () => {
  it('indexes records by requirement id', () => {
    const map = suspectsById([
      { requirement: 'a', causes: [{ kind: 'not_in_baseline' }] },
      { requirement: 'b', causes: [{ kind: 'upstream_suspect', via: 'a' }] },
    ]);
    expect(map.size).toBe(2);
    expect(map.get('a')?.kind).toBe('identity-changed');
    expect(map.get('b')?.upstreamVia).toEqual(['a']);
  });
});

describe('projectIdForWorkspace', () => {
  it('uses the workspace directory basename, tolerating trailing slashes', () => {
    expect(projectIdForWorkspace('/home/x/projects/hybrid-core-physics')).toBe(
      'hybrid-core-physics',
    );
    expect(projectIdForWorkspace('/home/x/projects/hybrid-core-physics/')).toBe(
      'hybrid-core-physics',
    );
  });
});
