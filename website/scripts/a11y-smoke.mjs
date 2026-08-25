#!/usr/bin/env node
/**
 * Accessibility smoke check over the built site (dist/).
 *
 * Deliberately dependency-free and conservative: it enforces the invariants
 * the documentation policy cares about without a headless browser.
 *
 *   - every page has a non-empty <title> and <html lang>
 *   - every page has exactly one <h1>
 *   - every <iframe> has a non-empty title attribute
 *   - every <img> has an alt attribute (empty alt allowed for decorative)
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = process.argv[2] ?? 'dist';

// The Book under learn/ is built by mdBook from its own pinned repository and
// has its own quality process; its theme chrome (menu-title h1, sidebar TOC
// iframe) is upstream mdBook behaviour, tracked Book-side rather than gated
// here. This smoke check covers portal-authored pages.
const SKIP_DIRS = new Set([join(root, 'learn')]);

function* htmlFiles(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (!SKIP_DIRS.has(path)) yield* htmlFiles(path);
    } else if (name.endsWith('.html')) yield path;
  }
}

const failures = [];
let checked = 0;

for (const file of htmlFiles(root)) {
  const rel = relative(root, file);
  const html = readFileSync(file, 'utf8');
  checked += 1;

  if (!/<html[^>]*\slang=["'][^"']+["']/i.test(html)) {
    failures.push(`${rel}: <html> is missing a lang attribute`);
  }
  const title = html.match(/<title[^>]*>([\s\S]*?)<\/title>/i);
  if (!title || !title[1].trim()) {
    failures.push(`${rel}: missing or empty <title>`);
  }
  const h1Count = (html.match(/<h1[\s>]/gi) ?? []).length;
  if (h1Count !== 1) {
    failures.push(`${rel}: expected exactly one <h1>, found ${h1Count}`);
  }
  for (const iframe of html.match(/<iframe[^>]*>/gi) ?? []) {
    if (!/\stitle=["'][^"']+["']/i.test(iframe)) {
      failures.push(`${rel}: <iframe> without a title attribute`);
    }
  }
  for (const img of html.match(/<img[^>]*>/gi) ?? []) {
    if (!/\salt=/i.test(img)) {
      failures.push(`${rel}: <img> without an alt attribute`);
    }
  }
}

if (checked === 0) {
  console.error(`a11y-smoke: no HTML files found under ${root}`);
  process.exit(1);
}

if (failures.length) {
  console.error(`a11y-smoke: ${failures.length} failure(s) across ${checked} page(s):`);
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}

console.log(`a11y-smoke: ${checked} page(s) pass`);
