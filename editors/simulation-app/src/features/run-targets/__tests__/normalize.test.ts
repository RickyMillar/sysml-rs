/**
 * Run-target normalization — ownership-path grouping (the structural
 * "compliance suite" keys threaded from the backend ElementSummary
 * projection; Phase-4 coordination item).
 */
import { describe, expect, it } from 'vitest';
import {
  groupByOwnerPath,
  normalizeRunTargets,
  ownerPathOf,
  type RawElement,
} from '../normalize';

function el(
  id: string,
  name: string | null,
  kind: string,
  qualified_name?: string | null,
): RawElement {
  return { id, name, kind, qualified_name };
}

describe('ownerPathOf', () => {
  it('drops the final segment of a qualified name', () => {
    expect(ownerPathOf('Pkg::Sub::Case')).toBe('Pkg::Sub');
    expect(ownerPathOf('Pkg::Case')).toBe('Pkg');
  });

  it('is null for root-namespace and absent names', () => {
    expect(ownerPathOf('Case')).toBeNull();
    expect(ownerPathOf(null)).toBeNull();
    expect(ownerPathOf(undefined)).toBeNull();
    expect(ownerPathOf('')).toBeNull();
  });
});

describe('groupByOwnerPath', () => {
  it('groups by owning scope, sorted by path, ungrouped last', () => {
    const items = [
      el('a', 'TA', 'VerificationCaseUsage', 'Zeta::TA'),
      el('b', 'TB', 'VerificationCaseUsage', 'Alpha::TB'),
      el('c', 'TC', 'VerificationCaseUsage', 'TC'), // root namespace
      el('d', 'TD', 'VerificationCaseUsage', 'Alpha::TD'),
    ];
    const groups = groupByOwnerPath(items, (i) => i.qualified_name);
    expect(groups.map((g) => g.ownerPath)).toEqual(['Alpha', 'Zeta', null]);
    expect(groups[0].items.map((i) => i.id)).toEqual(['b', 'd']);
    expect(groups[2].items.map((i) => i.id)).toEqual(['c']);
  });

  it('omits the ungrouped bucket when every item has an owner', () => {
    const groups = groupByOwnerPath(
      [el('a', 'A', 'K', 'P::A')],
      (i) => i.qualified_name,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].ownerPath).toBe('P');
  });
});

describe('normalizeRunTargets — qualified-name threading', () => {
  it('carries qualifiedName + derived ownerPath onto targets', () => {
    const groups = normalizeRunTargets(
      [],
      [],
      [el('v1', 'TripTest', 'VerificationCaseUsage', 'ProtectionVerification::TripTest')],
      'file:///w.sysml',
    );
    const target = groups[0].targets[0];
    expect(target.qualifiedName).toBe('ProtectionVerification::TripTest');
    expect(target.ownerPath).toBe('ProtectionVerification');
  });

  it('is honest about absent qualified names — null, never derived', () => {
    const groups = normalizeRunTargets(
      [],
      [],
      [el('v1', 'Bare', 'VerificationCaseUsage')],
      'file:///w.sysml',
    );
    const target = groups[0].targets[0];
    expect(target.qualifiedName).toBeNull();
    expect(target.ownerPath).toBeNull();
  });
});
