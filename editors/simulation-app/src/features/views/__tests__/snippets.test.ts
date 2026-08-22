import { describe, expect, it } from 'vitest';
import { sysmlViewSnippets } from '../snippets';

describe('sysmlViewSnippets', () => {
  it('covers the eight standard view definitions (canonical names)', () => {
    const supertypes = sysmlViewSnippets.map((s) => s.supertype).sort();
    // The requirement-projection snippet also specializes GeneralView
    // (SysML v2 defines no RequirementView), hence the duplicate.
    expect(supertypes).toEqual([
      'ActionFlowView',
      'BrowserView',
      'GeneralView',
      'GeneralView',
      'GeometryView',
      'GridView',
      'InterconnectionView',
      'SequenceView',
      'StateTransitionView',
    ]);
  });

  it('uses unique trigger prefixes across all aliases', () => {
    // VS Code's JSON gives each snippet a list of aliases (e.g.
    // `["gview", "view-general"]`); flattening across snippets must
    // still produce a unique set so no two snippets fight over the
    // same trigger.
    const prefixes = sysmlViewSnippets.flatMap((s) => s.prefixes);
    expect(new Set(prefixes).size).toBe(prefixes.length);
    // VS Code today ships two prefixes per supertype.
    expect(prefixes.length).toBe(sysmlViewSnippets.length * 2);
  });

  it('every snippet body specializes via `:>` (no name-suffix heuristics)', () => {
    for (const s of sysmlViewSnippets) {
      expect(s.body).toContain(`:> ${s.supertype}`);
      expect(s.body).toContain('view def ${1:Name}');
      expect(s.body).toContain('expose ${2:Subject};');
    }
  });
});
