#!/usr/bin/env node
/**
 * Documentation-policy frontmatter gate.
 *
 * Every page in a claim-bearing section (start-here/, use/, reference/) must
 * carry the policy frontmatter: scope, status, last_verified_against, and
 * source_of_truth. The schema in content.config.ts validates the SHAPE of
 * these fields wherever they appear; this check enforces their PRESENCE where
 * the policy requires them.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const DOCS_ROOT = new URL('../src/content/docs/', import.meta.url).pathname;
const REQUIRED_SECTIONS = ['start-here', 'use', 'reference'];
const REQUIRED_KEYS = ['scope', 'status', 'last_verified_against', 'source_of_truth'];

function* pages(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) yield* pages(path);
    else if (/\.mdx?$/.test(name)) yield path;
  }
}

const failures = [];
let checked = 0;

for (const section of REQUIRED_SECTIONS) {
  for (const file of pages(join(DOCS_ROOT, section))) {
    checked += 1;
    const rel = relative(DOCS_ROOT, file);
    const text = readFileSync(file, 'utf8');
    const fm = text.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) {
      failures.push(`${rel}: no frontmatter block`);
      continue;
    }
    for (const key of REQUIRED_KEYS) {
      if (!new RegExp(`^${key}:`, 'm').test(fm[1])) {
        failures.push(`${rel}: missing required frontmatter key '${key}'`);
      }
    }
  }
}

if (failures.length) {
  console.error(`frontmatter-check: ${failures.length} failure(s) across ${checked} page(s):`);
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(`frontmatter-check: ${checked} page(s) carry the required policy frontmatter`);
