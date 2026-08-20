/**
 * Suspect wire types + view-model mapping.
 *
 * `sysml.workspace.requirement_suspects` returns `SuspectRecord[]` —
 * per-requirement causes attributed backend-side (nearest-requirement
 * owner walk + transitive Derive propagation; sysml-query/src/suspect.rs
 * is the one home for that logic). This module only reshapes records for
 * rendering; it NEVER re-derives or smooths attribution.
 *
 * ADR-009 honesty: `not_in_baseline` means the row's identity does not
 * exist in the baseline — newly authored OR a scope-rename replacement,
 * indistinguishable by the id-strict diff. Render as "identity changed";
 * never name-match.
 */

/** Wire mirror of `sysml_query::suspect::SuspectCause` (tag = "kind"). */
export type SuspectCauseWire =
  | { kind: 'text_changed'; element: string; from: string; to: string }
  | {
      kind: 'prop_text_changed';
      element: string;
      element_kind: string;
      key: string;
      from: string;
      to: string;
    }
  | { kind: 'content_changed'; element: string; element_kind: string }
  | { kind: 'child_added'; element: string; element_kind: string }
  | { kind: 'child_removed'; element: string; element_kind: string }
  | { kind: 'not_in_baseline' }
  | { kind: 'upstream_suspect'; via: string };

/** Wire mirror of `sysml_service::workflow::SuspectRecordView` (v1.5b:
 *  `cleared_by` = seq of the newest non-superseded clearing attestation;
 *  cleared rows are NOT suspect for display but stay in the response). */
export interface SuspectRecordWire {
  requirement: string;
  causes: SuspectCauseWire[];
  cleared_by?: number | null;
}

/** View model consumed by the ⚑ column and the suspect popover. */
export interface SuspectRecord {
  requirement: string;
  /** 'identity-changed' when the row's id is absent from the baseline. */
  kind: 'changed' | 'identity-changed';
  /** Before/after statement-text pairs (may be empty for non-text changes). */
  textDeltas: Array<{ from: string | null; to: string | null }>;
  /** Before/after pairs for other scalar prop edits (constraint bodies,
   *  attribute values — W4). `key` names the prop, `elementKind` the
   *  carrying element, for the delta's label line. */
  propDeltas: Array<{ elementKind: string; key: string; from: string; to: string }>;
  /** Human summary of non-text causes (used when both delta lists are empty). */
  changeSummary: string;
  /** Requirement ids whose upstream change propagated to this row. */
  upstreamVia: string[];
  raw: SuspectCauseWire[];
}

function summarize(causes: SuspectCauseWire[]): string {
  const parts: string[] = [];
  const count = (k: SuspectCauseWire['kind']) => causes.filter((c) => c.kind === k).length;
  const content = count('content_changed');
  if (content > 0) parts.push(`${content} element${content === 1 ? '' : 's'} changed`);
  const props = count('prop_text_changed');
  if (props > 0) parts.push(`${props} value${props === 1 ? '' : 's'} changed`);
  const added = count('child_added');
  if (added > 0) parts.push(`${added} nested element${added === 1 ? '' : 's'} added`);
  const removed = count('child_removed');
  if (removed > 0) parts.push(`${removed} nested element${removed === 1 ? '' : 's'} removed`);
  const upstream = causes.filter(
    (c): c is Extract<SuspectCauseWire, { kind: 'upstream_suspect' }> =>
      c.kind === 'upstream_suspect',
  );
  if (upstream.length > 0) parts.push('an upstream requirement it derives from changed');
  return parts.length > 0 ? `Changed since baseline: ${parts.join(', ')}.` : 'Changed since baseline.';
}

export function toSuspectRecord(wire: SuspectRecordWire): SuspectRecord {
  const identityChanged = wire.causes.some((c) => c.kind === 'not_in_baseline');
  const textDeltas = wire.causes
    .filter(
      (c): c is Extract<SuspectCauseWire, { kind: 'text_changed' }> => c.kind === 'text_changed',
    )
    .map((c) => ({ from: c.from, to: c.to }));
  const propDeltas = wire.causes
    .filter(
      (c): c is Extract<SuspectCauseWire, { kind: 'prop_text_changed' }> =>
        c.kind === 'prop_text_changed',
    )
    .map((c) => ({ elementKind: c.element_kind, key: c.key, from: c.from, to: c.to }));
  const upstreamVia = wire.causes
    .filter(
      (c): c is Extract<SuspectCauseWire, { kind: 'upstream_suspect' }> =>
        c.kind === 'upstream_suspect',
    )
    .map((c) => c.via);
  return {
    requirement: wire.requirement,
    kind: identityChanged ? 'identity-changed' : 'changed',
    textDeltas,
    propDeltas,
    changeSummary: summarize(wire.causes),
    upstreamVia,
    raw: wire.causes,
  };
}

/** Index records by requirement id for O(1) row lookups. */
export function suspectsById(wires: SuspectRecordWire[]): Map<string, SuspectRecord> {
  const map = new Map<string, SuspectRecord>();
  for (const wire of wires) {
    map.set(wire.requirement, toSuspectRecord(wire));
  }
  return map;
}
