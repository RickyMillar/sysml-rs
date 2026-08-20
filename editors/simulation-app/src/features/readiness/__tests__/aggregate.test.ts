/**
 * Pure-function tests for `aggregateReadiness` (ninebar Phase 1.5).
 *
 * Exercises the four readiness levels off mocked diagnostics /
 * dependency-status / capabilities data — no react-query mocking
 * needed since the aggregation logic is a pure function.
 */

import { describe, expect, it } from 'vitest';
import { aggregateReadiness, type AggregateReadinessInput } from '../aggregate';
import type { DiagnosticEntry } from '@/features/diagnostics/types';
import type { DependencyStatusWire } from '@/features/packages/queries';
import type { Capabilities } from '@/store/workspace';

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

function emptyCapabilities(): Capabilities {
  return {
    hasStateMachines: false,
    hasActionFlows: false,
    hasOdeDynamics: false,
    hasPortFlows: false,
    hasMultipleSubsystems: false,
    hasConstraints: false,
    hasRequirements: false,
    hasTradeStudies: false,
    stateMachineNames: [],
    actionFlowNames: [],
    tradeStudyNames: [],
  };
}

function baseInput(overrides: Partial<AggregateReadinessInput> = {}): AggregateReadinessInput {
  return {
    hasWorkspace: true,
    diagnostics: [],
    dependencyStatus: undefined,
    capabilities: undefined,
    ...overrides,
  };
}

describe('aggregateReadiness', () => {
  it('returns unknown (and an empty summary) when no workspace is loaded', () => {
    const result = aggregateReadiness(baseInput({ hasWorkspace: false, diagnostics: [makeEntry()] }));
    expect(result.level).toBe('unknown');
    expect(result.counts).toEqual({ errors: 0, warnings: 0 });
    expect(result.drill).toEqual([]);
    expect(result.unresolvedDeps).toEqual([]);
    expect(result.missingCapabilities).toEqual([]);
  });

  it('is ready when there are no errors, warnings, or unresolved deps', () => {
    const result = aggregateReadiness(
      baseInput({
        diagnostics: [makeEntry({ severity: 'info' }), makeEntry({ severity: 'hint' })],
      }),
    );
    expect(result.level).toBe('ready');
    expect(result.counts).toEqual({ errors: 0, warnings: 0 });
    expect(result.drill).toEqual([]);
  });

  it('is warnings when only warning-severity diagnostics are present', () => {
    const result = aggregateReadiness(
      baseInput({
        diagnostics: [
          makeEntry({ severity: 'warning', message: 'unused import' }),
          makeEntry({ severity: 'info', message: 'deprecated' }),
        ],
      }),
    );
    expect(result.level).toBe('warnings');
    expect(result.counts).toEqual({ errors: 0, warnings: 1 });
    expect(result.drill).toHaveLength(1);
    expect(result.drill[0]).toMatchObject({ severity: 'warning', message: 'unused import' });
  });

  it('is errors when any error-severity diagnostic is present, even alongside warnings', () => {
    const result = aggregateReadiness(
      baseInput({
        diagnostics: [
          makeEntry({ severity: 'error', message: 'bad syntax' }),
          makeEntry({ severity: 'warning', message: 'unused import' }),
        ],
      }),
    );
    expect(result.level).toBe('errors');
    expect(result.counts).toEqual({ errors: 1, warnings: 1 });
    // Errors sort before warnings in the drill list.
    expect(result.drill.map((d) => d.severity)).toEqual(['error', 'warning']);
  });

  it('is errors when a dependency fails to resolve, even with zero diagnostics', () => {
    const dependencyStatus: DependencyStatusWire = {
      roots: [
        {
          root: '/ws/root-a',
          manifest: '/ws/root-a/sysml.toml',
          project: 'root-a',
          dependency_count: 1,
          failed_dependencies: [
            { name: 'missing-pkg', source: 'path', reason: 'missing_dependency', message: 'not found on disk' },
          ],
        },
      ],
      summary: { total_dependencies: 1, failed_dependencies: 1 },
    };
    const result = aggregateReadiness(baseInput({ dependencyStatus }));
    expect(result.level).toBe('errors');
    expect(result.unresolvedDeps).toEqual(['missing-pkg']);
    expect(result.counts).toEqual({ errors: 0, warnings: 0 }); // diagnostics-only counts
    expect(result.drill).toHaveLength(1);
    expect(result.drill[0]).toMatchObject({
      file: '/ws/root-a',
      severity: 'error',
      message: expect.stringContaining('missing-pkg'),
    });
  });

  it('tolerates no_manifest / error root entries without throwing', () => {
    const dependencyStatus: DependencyStatusWire = {
      roots: [
        { root: '/ws/no-manifest', status: 'no_manifest' },
        { root: '/ws/broken', status: 'error', error: 'permission denied' },
      ],
      summary: {},
    };
    const result = aggregateReadiness(baseInput({ dependencyStatus }));
    expect(result.level).toBe('ready');
    expect(result.unresolvedDeps).toEqual([]);
  });

  it('lists capability flags that are false as missingCapabilities, without affecting level', () => {
    const capabilities: Capabilities = {
      ...emptyCapabilities(),
      hasStateMachines: true,
    };
    const result = aggregateReadiness(baseInput({ capabilities }));
    expect(result.level).toBe('ready');
    expect(result.missingCapabilities).toContain('constraints');
    expect(result.missingCapabilities).toContain('actionFlows');
    expect(result.missingCapabilities).not.toContain('stateMachines');
  });

  it('drill entries carry file/severity/message with elementId left undefined', () => {
    const result = aggregateReadiness(
      baseInput({
        diagnostics: [
          {
            uri: 'file:///parent.sysml',
            diagnostic: {
              severity: 'error',
              message: 'unresolved reference',
              span: { file: 'file:///other.sysml', start: 0, end: 1 },
            },
          },
        ],
      }),
    );
    expect(result.drill[0]).toEqual({
      file: 'file:///other.sysml',
      severity: 'error',
      message: 'unresolved reference',
      elementId: undefined,
    });
  });
});
