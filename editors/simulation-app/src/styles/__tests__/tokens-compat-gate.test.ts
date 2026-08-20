import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';

/**
 * ninebar Phase 0 — compat-alias CI gate.
 *
 * `tokens-compat.css` (audit F17) re-points every pre-ninebar token name at
 * the new primitives so the legacy shell keeps rendering while Phases 1-7
 * land behind the flag. Its own header rule is explicit: "New code must
 * NEVER reference these names — semantic tokens only," and the file is
 * slated for deletion in Phase 8 once references hit zero.
 *
 * This test is the enforcement mechanism for that rule during the interim:
 * it counts every `--<legacy-name>` reference left in `src/` and fails the
 * suite if that count creeps above BASELINE. BASELINE is not a target —
 * it's a ratchet. It should only ever move down (when a legacy reference is
 * migrated to a semantic token) and must never be raised to make room for a
 * new legacy reference. If you hit this failure:
 *
 *   1. Do NOT reach for a --<legacy-name> alias in new code. Use the
 *      semantic token from tokens.css instead (e.g. --surface-panel, not
 *      --surface-container; --text-primary, not --on-surface).
 *   2. If you removed legacy references, lower BASELINE to match the new
 *      (smaller) count this test reports on failure.
 */

const STYLES_DIR = path.resolve(__dirname, '..');
const REPO_SRC = path.resolve(__dirname, '../../..', 'src');
const TOKENS_CSS = path.join(STYLES_DIR, 'tokens.css');
const TOKENS_COMPAT_CSS = path.join(STYLES_DIR, 'tokens-compat.css');
const THIS_FILE = path.resolve(__filename);

// Ratchet: current measured count of legacy `--<name>` references under
// src/ (excluding tokens.css, tokens-compat.css, this file, and
// __snapshots__ dirs). Lower this when references are removed; never
// raise it to accommodate new legacy usage.
const BASELINE = 1064;

/** Extract the legacy alias custom-property names tokens-compat.css defines. */
function extractAliasNames(cssSource: string): string[] {
  const names = new Set<string>();
  for (const rawLine of cssSource.split('\n')) {
    const line = rawLine.trim();
    if (line.startsWith('/*') || line.startsWith('*')) continue;
    const m = line.match(/^--([a-zA-Z0-9-]+)\s*:/);
    if (m) names.add(m[1]);
  }
  return [...names];
}

const SCAN_EXTENSIONS = new Set(['.ts', '.tsx', '.css']);
const EXCLUDED_DIRS = new Set(['__snapshots__', 'node_modules']);
const EXCLUDED_FILES = new Set([TOKENS_CSS, TOKENS_COMPAT_CSS, THIS_FILE]);

/** Recursively collect every scannable source file under `dir`. */
function walk(dir: string, acc: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (EXCLUDED_DIRS.has(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, acc);
      continue;
    }
    if (!SCAN_EXTENSIONS.has(path.extname(entry.name))) continue;
    if (EXCLUDED_FILES.has(path.resolve(full))) continue;
    acc.push(full);
  }
  return acc;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Count occurrences of `--<name>` in `content`, where `<name>` is not
 * immediately followed by another identifier character or hyphen (so
 * `--surface` doesn't match `--surface-panel`).
 */
function countRefs(content: string, name: string): number {
  const re = new RegExp(`--${escapeRegExp(name)}(?![\\w-])`, 'g');
  const matches = content.match(re);
  return matches ? matches.length : 0;
}

describe('ninebar Phase 0 — compat-alias CI gate', () => {
  const compatSource = fs.readFileSync(TOKENS_COMPAT_CSS, 'utf8');
  const aliasNames = extractAliasNames(compatSource);
  const files = walk(REPO_SRC);

  it('parses at least one legacy alias out of tokens-compat.css', () => {
    // Sanity check on the parser itself — if this ever reports 0, the
    // regex above stopped matching tokens-compat.css's declaration shape
    // and the whole gate is silently vacuous.
    expect(aliasNames.length).toBeGreaterThan(0);
  });

  it('keeps legacy compat-alias references at or below the recorded baseline', () => {
    const perAlias: Record<string, number> = {};
    let total = 0;
    for (const name of aliasNames) {
      let count = 0;
      for (const file of files) {
        const content = fs.readFileSync(file, 'utf8');
        count += countRefs(content, name);
      }
      if (count > 0) perAlias[name] = count;
      total += count;
    }

    if (total > BASELINE) {
      const breakdown = Object.entries(perAlias)
        .sort((a, b) => b[1] - a[1])
        .map(([name, count]) => `  --${name}: ${count}`)
        .join('\n');
      throw new Error(
        `Legacy compat-alias references grew from ${BASELINE} to ${total}.\n` +
          `tokens-compat.css aliases (audit F17) are a deprecated bridge layer — ` +
          `new code must use semantic tokens from tokens.css (e.g. --surface-panel, ` +
          `--text-primary, --border-default), not the legacy --<name> aliases below. ` +
          `Replace the new reference(s) with the semantic token, or if this failure ` +
          `is a false positive, investigate before touching BASELINE.\n\n` +
          `Current per-alias reference counts:\n${breakdown}`,
      );
    }

    // Also record when the count has genuinely gone down, so a future
    // author knows to ratchet BASELINE down rather than leaving slack.
    if (total < BASELINE) {
      // eslint-disable-next-line no-console
      console.warn(
        `[tokens-compat-gate] Legacy alias references dropped to ${total} ` +
          `(baseline is ${BASELINE}). Lower BASELINE in tokens-compat-gate.test.ts ` +
          `to close the gap.`,
      );
    }

    expect(total).toBeLessThanOrEqual(BASELINE);
  });

  it('has zero references to the deliberately-deleted glass tokens', () => {
    // --glass-bg / --glass-blur / --glass-border are NOT defined in
    // tokens-compat.css (see its header comment) — their consumers
    // (.glass-hud / .gradient-cta / .sim-glow) died in Phase 0. Nothing in
    // src/ should reference them going forward.
    const glassNames = ['glass-bg', 'glass-blur', 'glass-border'];
    const hits: string[] = [];
    for (const name of glassNames) {
      for (const file of files) {
        const content = fs.readFileSync(file, 'utf8');
        const count = countRefs(content, name);
        if (count > 0) hits.push(`  --${name}: ${count} in ${path.relative(REPO_SRC, file)}`);
      }
    }
    expect(hits, `Found dead glass-token references:\n${hits.join('\n')}`).toEqual([]);
  });
});
