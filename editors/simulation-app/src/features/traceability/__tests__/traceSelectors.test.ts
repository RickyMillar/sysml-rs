/**
 * Wire-contract guard for `DEFAULT_TRACE_SELECTORS` (ninebar Phase 4 warm-up).
 *
 * `sysml.trace_matrix` deserializes its kind args into `sysml-core` enums:
 *   - `source_kind` / `target_kind` → `ElementKind` (serde default → PascalCase)
 *   - `rel_kind`                     → `RelationshipKind`, which is
 *     `#[serde(rename_all = "camelCase")]` in
 *     `crates/lang/sysml-core/src/relationship.rs`.
 *
 * So the relationship kind MUST be camelCase on the wire (`satisfy`), while the
 * element kinds MUST stay PascalCase. A PascalCase `Satisfy` regresses the
 * request to `400 "unknown variant Satisfy"`. This test pins the split so a
 * future edit can't silently reintroduce the mismatch.
 */

import { describe, expect, it } from 'vitest';
import { DEFAULT_TRACE_SELECTORS } from '../types';

describe('DEFAULT_TRACE_SELECTORS wire contract', () => {
  it('sends the relationship kind as camelCase (RelationshipKind serde)', () => {
    // camelCase per `RelationshipKind`'s `rename_all = "camelCase"`; a leading
    // uppercase letter is exactly the shape the backend rejects.
    expect(DEFAULT_TRACE_SELECTORS.relation_kind).toBe('satisfy');
    expect(DEFAULT_TRACE_SELECTORS.relation_kind[0]).toBe(
      DEFAULT_TRACE_SELECTORS.relation_kind[0].toLowerCase(),
    );
  });

  it('keeps the element kinds PascalCase (ElementKind serde default)', () => {
    // Satisfy edges mint source=satisfier → target=requirement (B1a
    // direction flip) — the requirement is the TARGET endpoint.
    expect(DEFAULT_TRACE_SELECTORS.source_kind).toBe('PartUsage');
    expect(DEFAULT_TRACE_SELECTORS.target_kind).toBe('RequirementUsage');
  });
});
