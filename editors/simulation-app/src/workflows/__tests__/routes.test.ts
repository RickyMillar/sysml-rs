/**
 * Unit tests for the workflow route descriptors and path helpers
 * introduced in R2.1.
 */

import { describe, it, expect } from 'vitest';
// Route helpers are pure data/logic — import from the source file (not
// the barrel) so this test doesn't transitively pull RunWorkflow and
// the diagram package into the vitest node environment.
import {
  WORKFLOWS,
  navActiveIdForPath,
  pathForWorkflowId,
  workflowIdForPath,
} from '../routes';

describe('WORKFLOWS descriptor table', () => {
  it('contains the required workflow ids from the R2.1 plan', () => {
    const ids = WORKFLOWS.map((w) => w.id);
    // Ids the router + integration tests depend on.
    expect(ids).toContain('session');
    expect(ids).toContain('verify');
    expect(ids).toContain('requirements');
    expect(ids).toContain('analyze');
    expect(ids).toContain('sweep');
    expect(ids).toContain('montecarlo');
    expect(ids).toContain('trade-study');
    expect(ids).toContain('compare');
  });

  it('has unique ids and unique paths', () => {
    const ids = new Set(WORKFLOWS.map((w) => w.id));
    const paths = new Set(WORKFLOWS.map((w) => w.path));
    expect(ids.size).toBe(WORKFLOWS.length);
    expect(paths.size).toBe(WORKFLOWS.length);
  });

  it('every workflow path starts with a slash', () => {
    for (const wf of WORKFLOWS) {
      expect(wf.path.startsWith('/'), `path for ${wf.id}`).toBe(true);
    }
  });

  it('numeric hotkeys are unique within the table', () => {
    const hotkeys = WORKFLOWS.filter((w) => w.hotkey).map((w) => w.hotkey);
    expect(new Set(hotkeys).size).toBe(hotkeys.length);
  });

  it('maps session id to /run (legacy "session" label, canonical /run url)', () => {
    expect(pathForWorkflowId('session')).toBe('/run');
  });

  it('maps analyze shell and sub-workflows under /analyze/*', () => {
    expect(pathForWorkflowId('analyze')).toBe('/analyze');
    expect(pathForWorkflowId('sweep')).toBe('/analyze/sweep');
    expect(pathForWorkflowId('montecarlo')).toBe('/analyze/montecarlo');
    expect(pathForWorkflowId('trade-study')).toBe('/analyze/trade-study');
  });

  it('returns null for an unknown workflow id', () => {
    expect(pathForWorkflowId('does-not-exist')).toBeNull();
  });
});

describe('workflowIdForPath', () => {
  it('resolves the exact known paths', () => {
    expect(workflowIdForPath('/run')).toBe('session');
    expect(workflowIdForPath('/verify')).toBe('verify');
    expect(workflowIdForPath('/requirements')).toBe('requirements');
    // Phase 6 demotion: Compare lives under the Simulate door. The
    // longest-prefix rule keeps /run itself on 'session'.
    expect(workflowIdForPath('/run/compare')).toBe('compare');
    expect(workflowIdForPath('/run/compare/anything')).toBe('compare');
  });

  it('resolves analyze sub-paths to their concrete workflow, not a parent', () => {
    expect(workflowIdForPath('/analyze/sweep')).toBe('sweep');
    expect(workflowIdForPath('/analyze/montecarlo')).toBe('montecarlo');
    expect(workflowIdForPath('/analyze/trade-study')).toBe('trade-study');
  });

  it('treats unknown and removed legacy paths as null (Navigate-to-/run catches them)', () => {
    expect(workflowIdForPath('/')).toBeNull();
    expect(workflowIdForPath('/bogus')).toBeNull();
    expect(workflowIdForPath('/packages')).toBeNull();
    expect(workflowIdForPath('/run-targets')).toBeNull();
  });

  it('resolves /analyze to the Analyze shell', () => {
    expect(workflowIdForPath('/analyze')).toBe('analyze');
  });

  it('matches nested paths under a workflow (longest-prefix)', () => {
    // Future verify sub-routes should still resolve to the verify
    // workflow so the tab stays highlighted.
    expect(workflowIdForPath('/verify/anything')).toBe('verify');
    expect(workflowIdForPath('/run/anything/deeper')).toBe('session');
  });
});

describe('navActiveIdForPath (nav highlight for hidden sub-routes)', () => {
  it('lights the door tab for routes that have no nav tab of their own', () => {
    // Compare is a Simulate mode (Phase 6 demotion) — /run/compare
    // resolves to the compare workflow but lights the Run tab.
    expect(navActiveIdForPath('/run/compare')).toBe('session');
    // Analyze methods light the Analyze tab (previously a hardcoded
    // remap list in WorkflowSwitcher).
    expect(navActiveIdForPath('/analyze/sweep')).toBe('analyze');
    expect(navActiveIdForPath('/analyze/montecarlo')).toBe('analyze');
  });

  it('matches visible workflows exactly like workflowIdForPath', () => {
    expect(navActiveIdForPath('/run')).toBe('session');
    expect(navActiveIdForPath('/verify')).toBe('verify');
    expect(navActiveIdForPath('/requirements')).toBe('requirements');
    expect(navActiveIdForPath('/bogus')).toBeNull();
  });
});
