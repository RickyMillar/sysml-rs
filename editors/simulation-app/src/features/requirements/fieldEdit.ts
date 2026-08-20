/**
 * Field-edit splice logic — the client half of the buffer-writeback
 * contract (workbench design §7.2/§7.5). The service COMPUTES a guarded
 * `TextEdit` (`sysml.workspace.edit_*` / `create_requirement` / `add_*`);
 * this module VERIFIES the staleness guard against the local buffer and
 * splices. Pure functions only — orchestration (buffer seeding, the
 * `sysml.load_source` sync, query invalidation) lives in `useFieldEdit`.
 *
 * Positions are UTF-16 line/col, 0-indexed — the wire encoding shared by
 * every TextEdit producer (Monaco-native; design §7.2 rejected byte
 * offsets). JS strings ARE UTF-16, so column arithmetic is native
 * `String` indexing; no encoding conversion happens here.
 */

/** Mirrors `sysml_service::text_edit::TextEdit` (the ONE edit shape). */
export interface WireTextEdit {
  line_start: number;
  col_start: number;
  line_end: number;
  col_end: number;
  new_text: string;
  /** Staleness guard: the exact text the producer saw in the edited
   *  range. BINDING (§7.2): apply ONLY if the buffer slice matches;
   *  fail loudly on mismatch — never a silent mis-splice. Absent for
   *  pure insertions with no meaningful prior text. */
  expected_old_text?: string | null;
}

/** Mirrors `sysml_service::field_edit::FieldEditComputed`. */
export interface FieldEditComputed {
  uri: string;
  element_id: string;
  /** `doc` | `attribute_value` | `maturity` | `create`. */
  field: string;
  edit: WireTextEdit;
}

/** The spec's closed StatusKind vocabulary (ModelingMetadata library).
 *  Mirrors `field_edit::STATUS_KINDS`; the write boundary re-enforces it
 *  server-side — this copy only feeds the select's option list. */
export const STATUS_KINDS = ['open', 'tbd', 'tbr', 'tbc', 'done', 'closed'] as const;

/**
 * Resolve a UTF-16 (line, col) position to a string index, or null when
 * the position lies outside the text (more lines than the buffer has, or
 * a column past the end of its line — both mean the buffer diverged from
 * what the edit was computed against).
 */
export function lineColToOffset(text: string, line: number, col: number): number | null {
  let lineStart = 0;
  for (let i = 0; i < line; i++) {
    const nl = text.indexOf('\n', lineStart);
    if (nl === -1) return null;
    lineStart = nl + 1;
  }
  const nextNl = text.indexOf('\n', lineStart);
  const lineEnd = nextNl === -1 ? text.length : nextNl;
  if (lineStart + col > lineEnd) return null;
  return lineStart + col;
}

export type SpliceResult =
  | { ok: true; next: string }
  | { ok: false; reason: string };

const EXCERPT = 60;
const excerpt = (s: string) =>
  s.length > EXCERPT ? `${s.slice(0, EXCERPT)}…` : s;

/**
 * Verify the staleness guard and splice. Never writes on mismatch: a
 * failed result means the buffer is byte-identical to before the call.
 */
export function applyGuardedEdit(source: string, edit: WireTextEdit): SpliceResult {
  const start = lineColToOffset(source, edit.line_start, edit.col_start);
  const end = lineColToOffset(source, edit.line_end, edit.col_end);
  if (start === null || end === null || end < start) {
    return {
      ok: false,
      reason:
        'stale buffer — the edit range no longer exists in the file. ' +
        'The file changed since the edit was computed; reload and retry.',
    };
  }
  const actual = source.slice(start, end);
  if (edit.expected_old_text != null && actual !== edit.expected_old_text) {
    return {
      ok: false,
      reason:
        `stale buffer — expected "${excerpt(edit.expected_old_text)}" at the edit ` +
        `range but found "${excerpt(actual)}". The file changed since the edit ` +
        'was computed; reload and retry.',
    };
  }
  return { ok: true, next: source.slice(0, start) + edit.new_text + source.slice(end) };
}

/** Identifier check for the guided-create popover — client-side UX
 *  mirror only; `compute_create_requirement` re-validates at the write
 *  boundary. Matches the parser's basic-name shape. */
export function isValidRequirementName(name: string): boolean {
  return /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(name);
}
