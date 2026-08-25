#!/usr/bin/env node
// Generates the committed reference artifacts under src/generated/ and the
// generated reference pages under src/content/docs/reference/.
//
// This is a maintainer-run script: it needs the sysml-rs Rust workspace
// (release binaries + cargo), which the docs CI deliberately does not build.
// The outputs ARE committed; portal builds only render them.
//
// Usage (from website/):
//   node scripts/generate-reference.mjs           # regenerate artifacts + pages
//   node scripts/generate-reference.mjs --check   # regenerate to a temp dir and
//                                                 # diff against the committed
//                                                 # copies; exit 1 on drift
//
// Environment:
//   SYSML_BIN       path to the release CLI      (default <repo>/target/release/sysml)
//   SYSML_API_BIN   path to the release API bin  (default <repo>/target/release/sysml-api)
//   CARGO           cargo executable             (default "cargo"; used for the
//                   spec-index diagnostics-registry dump)
//
// Inputs and their revisions are recorded in src/generated/generated-meta.json.

import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const websiteDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = path.resolve(websiteDir, '..');
const SYSML_BIN = process.env.SYSML_BIN ?? path.join(repoRoot, 'target', 'release', 'sysml');
const SYSML_API_BIN = process.env.SYSML_API_BIN ?? path.join(repoRoot, 'target', 'release', 'sysml-api');
const CARGO = process.env.CARGO ?? 'cargo';

const GENERATED_DIR = path.join(websiteDir, 'src', 'generated');
const PAGES_DIR = path.join(websiteDir, 'src', 'content', 'docs', 'reference');
const PACK_DIR = path.join(websiteDir, '.learn-src', 'src', 'language-pack');

const checkMode = process.argv.includes('--check');

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function run(cmd, args, opts = {}) {
  return execFileSync(cmd, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, ...opts });
}

function gitShortHead() {
  return run('git', ['rev-parse', '--short', 'HEAD'], { cwd: repoRoot }).trim();
}

function stableJson(value) {
  return JSON.stringify(value, null, 2) + '\n';
}

/** Escape text destined for a Markdown table cell. */
function mdCell(text) {
  return String(text ?? '')
    .replace(/\|/g, '\\|')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\r?\n/g, ' ')
    .trim();
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, '127.0.0.1', () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
    srv.on('error', reject);
  });
}

// ---------------------------------------------------------------------------
// 1. CLI command catalogue: recursive `--help` parsing
// ---------------------------------------------------------------------------

/**
 * Parse one clap help text into { description, usage, arguments, options, subcommands }.
 * Section entries may wrap onto deeper-indented continuation lines.
 */
function parseHelp(helpText) {
  const lines = helpText.split('\n');
  const result = { description: '', usage: '', arguments: [], options: [], subcommands: [] };

  // Description: everything before the Usage: line.
  const usageIdx = lines.findIndex((l) => l.startsWith('Usage:'));
  if (usageIdx > 0) {
    result.description = lines.slice(0, usageIdx).join('\n').trim();
  }
  if (usageIdx >= 0) {
    result.usage = lines[usageIdx].replace(/^Usage:\s*/, '').trim();
  }

  let section = null;
  let current = null;
  const flush = () => {
    current = null;
  };
  for (let i = usageIdx + 1; i < lines.length; i++) {
    const line = lines[i];
    if (/^Commands:\s*$/.test(line)) { section = 'commands'; flush(); continue; }
    if (/^Arguments:\s*$/.test(line)) { section = 'arguments'; flush(); continue; }
    if (/^Options:\s*$/.test(line)) { section = 'options'; flush(); continue; }
    if (/^\S/.test(line) && line.trim() !== '') { section = null; flush(); continue; }
    if (!section) continue;
    if (line.trim() === '') { flush(); continue; }

    if (section === 'commands') {
      const m = line.match(/^ {2}(\S+)\s*(.*)$/);
      if (m && !line.startsWith('    ')) {
        current = { name: m[1], description: m[2].trim() };
        if (m[1] !== 'help') result.subcommands.push(current);
      } else if (current) {
        current.description = `${current.description} ${line.trim()}`.trim();
      }
    } else if (section === 'arguments') {
      const m = line.match(/^ {2}(\S+)\s*(.*)$/);
      if (m && !line.startsWith('    ')) {
        current = { name: m[1], description: m[2].trim() };
        result.arguments.push(current);
      } else if (current) {
        current.description = `${current.description} ${line.trim()}`.trim();
      }
    } else if (section === 'options') {
      const m = line.match(/^ {2,6}(-{1,2}\S.*?)( {2,}(.*))?$/);
      if (m && /^\s{2,6}-/.test(line)) {
        current = { flags: m[1].trim(), description: (m[3] ?? '').trim() };
        result.options.push(current);
      } else if (current) {
        current.description = `${current.description} ${line.trim()}`.trim();
      }
    }
  }
  return result;
}

/** Recursively collect the CLI command tree via `--help`. */
function collectCliCommands() {
  const commands = [];
  const visit = (cmdPath) => {
    const help = run(SYSML_BIN, [...cmdPath, '--help']);
    const parsed = parseHelp(help);
    commands.push({
      name: cmdPath.length === 0 ? 'sysml' : cmdPath.join(' '),
      path: cmdPath,
      description: parsed.description,
      usage: parsed.usage,
      arguments: parsed.arguments,
      options: parsed.options,
      subcommands: parsed.subcommands.map((s) => s.name),
      help,
    });
    for (const sub of parsed.subcommands) visit([...cmdPath, sub.name]);
  };
  visit([]);
  return commands;
}

// ---------------------------------------------------------------------------
// 2. Service command catalogue: live /commands from sysml-api
// ---------------------------------------------------------------------------

async function collectServiceCommands() {
  const port = await freePort();
  const addr = `127.0.0.1:${port}`;
  const server = spawn(SYSML_API_BIN, [addr], { stdio: 'ignore' });
  try {
    const base = `http://${addr}`;
    let health = null;
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      try {
        const res = await fetch(`${base}/health`);
        if (res.ok) { health = await res.json(); break; }
      } catch { /* not up yet */ }
      await new Promise((r) => setTimeout(r, 250));
    }
    if (!health) throw new Error(`sysml-api on ${addr} never became healthy`);
    const res = await fetch(`${base}/commands`);
    if (!res.ok) throw new Error(`GET /commands failed: ${res.status}`);
    const catalog = await res.json();
    catalog.sort((a, b) => a.name.localeCompare(b.name));
    return { catalog, apiVersion: health.version };
  } finally {
    server.kill('SIGTERM');
  }
}

// ---------------------------------------------------------------------------
// 3. Core diagnostics registry: spec-index dump
// ---------------------------------------------------------------------------

function collectDiagnostics() {
  const out = run(CARGO, ['run', '--release', '-p', 'spec-index', '--', 'diagnostics-registry', '--json'], {
    cwd: repoRoot,
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  return JSON.parse(out);
}

// ---------------------------------------------------------------------------
// 4. Capability matrix: aggregate the pinned Book's language-pack cards
// ---------------------------------------------------------------------------

const SUPPORT_STAGES = ['parse', 'lower', 'resolve', 'elaborate', 'validate', 'execute', 'format', 'lsp'];

function collectCapabilityMatrix() {
  if (!existsSync(PACK_DIR)) {
    throw new Error(
      `${PACK_DIR} not found — run \`bash scripts/build-learn.sh\` first to materialize the pinned Book checkout.`,
    );
  }
  const packManifest = JSON.parse(readFileSync(path.join(PACK_DIR, 'manifest.json'), 'utf8'));
  const lock = JSON.parse(readFileSync(path.join(websiteDir, 'content-lock.json'), 'utf8'));

  const cardsDir = path.join(PACK_DIR, 'cards');
  const cardFiles = readdirSync(cardsDir).filter((f) => f.endsWith('.json')).sort();
  const cards = cardFiles.map((f) => JSON.parse(readFileSync(path.join(cardsDir, f), 'utf8')));

  const categories = {};
  for (const card of cards) {
    const validatedStages = SUPPORT_STAGES.filter((s) => card.support?.[s] === 'validated');
    const pos = card.examples?.positive?.length ?? 0;
    const neg = card.examples?.negative?.length ?? 0;
    const evidence =
      pos > 0 && neg > 0 ? 'positive + negative fixtures'
      : pos > 0 ? 'positive fixtures'
      : neg > 0 ? 'negative fixtures'
      : 'none';
    const row = {
      id: card.id,
      title: card.title,
      language: card.language,
      validated_stages: validatedStages,
      evidence,
      known_gaps: card.known_gaps?.length ?? 0,
    };
    for (const cat of card.category ?? []) {
      categories[cat] ??= { concepts: [], stage_validated_counts: Object.fromEntries(SUPPORT_STAGES.map((s) => [s, 0])) };
      categories[cat].concepts.push(row);
      for (const s of validatedStages) categories[cat].stage_validated_counts[s] += 1;
    }
  }
  for (const cat of Object.values(categories)) {
    cat.concepts.sort((a, b) => a.id.localeCompare(b.id));
    cat.card_count = cat.concepts.length;
  }

  const totals = {
    cards: cards.length,
    cards_with_any_validated_stage: cards.filter((c) => SUPPORT_STAGES.some((s) => c.support?.[s] === 'validated')).length,
    stage_validated_counts: Object.fromEntries(
      SUPPORT_STAGES.map((s) => [s, cards.filter((c) => c.support?.[s] === 'validated').length]),
    ),
  };

  return {
    pack: {
      spec_drop: packManifest.spec_drop,
      metamodel_drop: packManifest.metamodel_drop,
      generator_version: packManifest.generator_version,
      book_pin: lock.book.commit,
      book_repository: lock.book.repository,
    },
    support_stages: SUPPORT_STAGES,
    totals,
    categories: Object.fromEntries(Object.entries(categories).sort(([a], [b]) => a.localeCompare(b))),
  };
}

// ---------------------------------------------------------------------------
// Page rendering (Markdown; the whole body is emitted here so the page can
// never diverge from the committed artifact it renders)
// ---------------------------------------------------------------------------

const GENERATED_BANNER = (artifact) => `<!--
GENERATED — do not edit.
Regenerate (with the artifact it renders, ${artifact}) via:
  cd website && node scripts/generate-reference.mjs
-->`;

function frontmatter({ title, description, scope, status, commit, sourceOfTruth, knownLimitations }) {
  const lines = [
    '---',
    `title: ${title}`,
    `description: ${description}`,
    'scope:',
    ...scope.map((s) => `  - ${s}`),
    `status: ${status}`,
    `last_verified_against: ${commit}`,
    'source_of_truth:',
    ...sourceOfTruth.map((s) => `  - ${s}`),
  ];
  if (knownLimitations) lines.push(`known_limitations: ${knownLimitations}`);
  lines.push('---');
  return lines.join('\n');
}

function provenanceFooter(commit, dateIso, inputLine) {
  return [
    '## How this page is generated',
    '',
    `This page and its data artifact were generated by \`node scripts/generate-reference.mjs\` (run from \`website/\`) at sysml-rs commit \`${commit}\` on ${dateIso.slice(0, 10)}. ${inputLine}`,
    'Do not edit the page by hand — regenerate it. `npm run gen-check` reports drift between the committed artifacts and a fresh generation.',
  ].join('\n');
}

function renderCliPage(cli, commit, dateIso, cliVersion) {
  const top = cli.commands.find((c) => c.path.length === 0);
  const byName = new Map(cli.commands.map((c) => [c.name, c]));

  const lines = [];
  lines.push(frontmatter({
    title: 'CLI command reference',
    description: 'Every sysml CLI command and its options, generated from the live --help output.',
    scope: ['sysml-rs tooling'],
    status: 'pre-alpha',
    commit,
    sourceOfTruth: ['website/src/generated/cli-commands.json', 'crates/tooling/sysml-cli'],
  }));
  lines.push('');
  lines.push(GENERATED_BANNER('src/generated/cli-commands.json'));
  lines.push('');
  lines.push(`This is the complete command catalogue of the \`sysml\` CLI (\`${mdCell(cliVersion)}\`), captured from the binary's own \`--help\` output — nothing here is hand-maintained. For task-oriented walkthroughs, start at [CLI workflows](/sysml-rs/use/cli-workflows/) instead.`);
  lines.push('');

  const renderCommand = (cmd, depth) => {
    const h = '#'.repeat(Math.min(depth + 2, 4));
    lines.push(`${h} \`sysml ${cmd.name}\``);
    lines.push('');
    if (cmd.description) lines.push(mdCell(cmd.description.split('\n')[0]), '');
    lines.push('```text', `Usage: ${cmd.usage}`, '```', '');
    if (cmd.arguments.length > 0) {
      lines.push('| Argument | Description |', '|---|---|');
      for (const a of cmd.arguments) lines.push(`| \`${mdCell(a.name)}\` | ${mdCell(a.description)} |`);
      lines.push('');
    }
    const options = cmd.options.filter((o) => !/^-h, --help/.test(o.flags) && !/^-V, --version/.test(o.flags));
    if (options.length > 0) {
      lines.push('| Option | Description |', '|---|---|');
      for (const o of options) lines.push(`| \`${mdCell(o.flags)}\` | ${mdCell(o.description)} |`);
      lines.push('');
    }
    for (const sub of cmd.subcommands) {
      const child = byName.get(cmd.path.length === 0 ? sub : `${cmd.name} ${sub}`);
      if (child) renderCommand(child, depth + 1);
    }
  };

  lines.push('## Global usage');
  lines.push('');
  lines.push('```text', `Usage: ${top.usage}`, '```', '');
  const globalOptions = top.options.filter((o) => !/^-h, --help/.test(o.flags) && !/^-V, --version/.test(o.flags));
  if (globalOptions.length > 0) {
    lines.push('| Global option | Description |', '|---|---|');
    for (const o of globalOptions) lines.push(`| \`${mdCell(o.flags)}\` | ${mdCell(o.description)} |`);
    lines.push('');
  }
  lines.push('| Command | Summary |', '|---|---|');
  for (const sub of top.subcommands) {
    const child = byName.get(sub);
    lines.push(`| [\`sysml ${sub}\`](/sysml-rs/reference/cli-commands/#sysml-${sub}) | ${mdCell(child?.description.split('\n')[0] ?? '')} |`);
  }
  lines.push('');

  for (const sub of top.subcommands) {
    const child = byName.get(sub);
    if (child) renderCommand(child, 0);
  }

  lines.push(provenanceFooter(commit, dateIso, `Input: \`${mdCell(cliVersion)}\` at \`target/release/sysml\`; the full raw help text per command is stored in the artifact.`));
  lines.push('');
  return lines.join('\n');
}

function renderApiCatalogPage(service, commit, dateIso) {
  const { catalog, apiVersion } = service;
  const categories = {};
  for (const cmd of catalog) (categories[cmd.category] ??= []).push(cmd);

  const lines = [];
  lines.push(frontmatter({
    title: 'API and MCP command catalogue',
    description: 'Every native service command exposed over the HTTP API and as MCP tools, from the live /commands catalogue.',
    scope: ['sysml-rs tooling'],
    status: 'pre-alpha',
    commit,
    sourceOfTruth: ['website/src/generated/service-commands.json', 'crates/tooling/sysml-service'],
  }));
  lines.push('');
  lines.push(GENERATED_BANNER('src/generated/service-commands.json'));
  lines.push('');
  lines.push(`These are the native service commands of \`sysml-api\` version ${mdCell(apiVersion)}, captured from the server's own \`GET /commands\` catalogue — the live catalogue is authoritative. Each command is callable three ways:`);
  lines.push('');
  lines.push('- `POST /api/commands/<name>` with the parameters as a JSON body;');
  lines.push('- `POST /api/command` with `{ "command": "<name>", "params": { ... } }`;');
  lines.push('- as an MCP tool, where the tool name is the command name with dots replaced by underscores (`sysml.sessions.step` → `sysml_sessions_step`).');
  lines.push('');
  lines.push('Transport, security posture, and client setup are covered in [Integrations](/sysml-rs/use/integrations/). These native commands are a sysml-rs convention, not the OMG Systems Modeling API.');
  lines.push('');
  const deprecated = catalog.filter((c) => c.deprecated);
  lines.push(`The catalogue currently carries ${catalog.length} commands${deprecated.length > 0 ? `, of which ${deprecated.length} are flagged deprecated (marked below)` : '; none are flagged deprecated'}.`);
  lines.push('');

  for (const cat of Object.keys(categories).sort()) {
    lines.push(`## ${cat}`);
    lines.push('');
    lines.push('| Command | Description | Parameters |', '|---|---|---|');
    for (const cmd of categories[cat]) {
      const params = (cmd.params ?? [])
        .map((p) => `\`${p.name}\`${p.required ? '' : '?'}`)
        .join(', ');
      const flags = [cmd.deprecated ? '**deprecated**' : null, cmd.stateful ? 'stateful' : null]
        .filter(Boolean).join(', ');
      const desc = `${mdCell(cmd.description)}${flags ? ` *(${flags})*` : ''}`;
      lines.push(`| \`${mdCell(cmd.name)}\` | ${desc} | ${params || '—'} |`);
    }
    lines.push('');
  }
  lines.push('Parameter names marked `?` are optional. Full parameter types and per-parameter descriptions are in the committed artifact `src/generated/service-commands.json` and from the live `GET /commands` endpoint.');
  lines.push('');
  lines.push(provenanceFooter(commit, dateIso, `Input: \`GET /commands\` and \`GET /health\` on a locally started \`target/release/sysml-api\` (version ${mdCell(apiVersion)}).`));
  lines.push('');
  return lines.join('\n');
}

const RUNTIME_FAMILIES = [
  ['AX', 'Action execution health'],
  ['SM', 'State-machine execution health'],
  ['FL', 'Flow / transfer execution health'],
  ['VC', 'Verification-case execution health'],
  ['CN', 'Constraint evaluation health'],
  ['RQ', 'Requirement evaluation health'],
  ['PH', 'Physics / hybrid-simulation execution health'],
];

function renderDiagnosticsPage(diagnostics, commit, dateIso) {
  const categoryOrder = ['Structural', 'Resolution', 'Semantic', 'Validation'];
  const categoryBlurbs = {
    Structural: 'Structural integrity of the semantic graph (E-series): ownership, memberships, and relationship endpoints.',
    Resolution: 'Name resolution and import health (E2xx and IM-series).',
    Semantic: 'Semantic validation (S-series) plus the specialised semantic families: physics (PH), flows (FL), variability (VR), runtime semantic core (RS), and quantities/dimensions (UQ).',
    Validation: 'Property-level validation (V-series).',
  };

  const lines = [];
  lines.push(frontmatter({
    title: 'Diagnostics reference',
    description: 'Every registered core diagnostic code, generated from the sysml-core error-code registry, plus the runtime health families.',
    scope: ['sysml-rs implementation'],
    status: 'pre-alpha',
    commit,
    sourceOfTruth: ['website/src/generated/diagnostics-core.json', 'crates/lang/sysml-core/src/error_codes.rs'],
    knownLimitations: '/sysml-rs/reference/known-limitations/',
  }));
  lines.push('');
  lines.push(GENERATED_BANNER('src/generated/diagnostics-core.json'));
  lines.push('');
  lines.push(`This page lists every diagnostic code registered in the sysml-core error-code registry (${diagnostics.codes.length} codes), generated directly from that registry. These codes appear in editor diagnostics, \`sysml check\`/\`sysml inspect\` output, and API diagnostic responses. For the conceptual framing of the code families, see the Book's [Appendix F — diagnostic codes](/sysml-rs/learn/appendix-f-diagnostic-codes.html).`);
  lines.push('');
  lines.push('**Scope**: the per-code tables below cover the core registry only. Runtime health codes are a separate surface, documented at family level [further down](/sysml-rs/reference/diagnostics/#runtime-health-families) — they are produced during execution, not by static analysis, and are not part of this registry.');
  lines.push('');

  for (const cat of categoryOrder) {
    const codes = diagnostics.codes.filter((c) => c.category === cat);
    if (codes.length === 0) continue;
    lines.push(`## ${cat} (${codes.length} codes)`);
    lines.push('');
    lines.push(categoryBlurbs[cat], '');
    lines.push('| Code | Meaning |', '|---|---|');
    for (const c of codes) lines.push(`| \`${c.code}\` | ${mdCell(c.short_description)} |`);
    lines.push('');
  }

  lines.push('## Runtime health families');
  lines.push('');
  lines.push('Execution surfaces (simulation sessions, verification runs, flow simulation) report **runtime health codes** in these families. They are emitted by the runtime, carry run-specific context, and are intentionally *not* part of the static registry above, so they are documented here at family level only — the authoritative per-code source is the runtime output itself.');
  lines.push('');
  lines.push('| Family | Covers |', '|---|---|');
  for (const [fam, desc] of RUNTIME_FAMILIES) lines.push(`| \`${fam}\` | ${desc} |`);
  lines.push('');
  lines.push('Note that `PH` appears in both worlds: the static registry has `PH001`–`PH006` physics lints (listed above under Semantic), while runtime physics health codes in the `PH` family are separate. Known gaps in diagnostic coverage are tracked in [Known limitations](/sysml-rs/reference/known-limitations/).');
  lines.push('');
  lines.push(provenanceFooter(commit, dateIso, 'Input: `cargo run --release -p spec-index -- diagnostics-registry --json` (the `sysml-core::error_codes` registry).'));
  lines.push('');
  return lines.join('\n');
}

function renderCapabilityMatrixPage(matrix, commit, dateIso) {
  const lines = [];
  lines.push(frontmatter({
    title: 'Language capability matrix',
    description: 'Measured per-concept support of the SysML v2 / KerML language surface, aggregated from the language pack.',
    scope: ['sysml-rs implementation', 'Experimental / partial support'],
    status: 'pre-alpha',
    commit,
    sourceOfTruth: ['website/src/generated/capability-matrix.json', 'website/.learn-src/src/language-pack/'],
    knownLimitations: '/sysml-rs/reference/known-limitations/',
  }));
  lines.push('');
  lines.push(GENERATED_BANNER('src/generated/capability-matrix.json'));
  lines.push('');
  lines.push(`This matrix aggregates the [language pack](/sysml-rs/learn/language-pack/) — one card per language concept, each carrying evidence-gated support statements for the pipeline stages ${matrix.support_stages.map((s) => `\`${s}\``).join(', ')}. A stage is **validated** only when a purpose-built fixture proves it; everything else is reported as **unknown**, never assumed. Absence of a "validated" mark is absence of evidence, not necessarily absence of support.`);
  lines.push('');
  lines.push(`Measured from the language pack shipped in the pinned Book checkout (Book pin \`${matrix.pack.book_pin.slice(0, 7)}\`), pack generator version ${matrix.pack.generator_version}, built against OMG spec drop ${matrix.pack.spec_drop} / metamodel drop ${matrix.pack.metamodel_drop}.`);
  lines.push('');
  lines.push('## Totals');
  lines.push('');
  lines.push(`${matrix.totals.cards} concept cards; ${matrix.totals.cards_with_any_validated_stage} have at least one validated stage.`);
  lines.push('');
  lines.push('| Stage | Cards validated |', '|---|---|');
  for (const s of matrix.support_stages) lines.push(`| \`${s}\` | ${matrix.totals.stage_validated_counts[s]} |`);
  lines.push('');
  lines.push('## Per-category support');
  lines.push('');
  lines.push('A concept card can belong to more than one category, so category counts overlap. Evidence kinds refer to the pack\'s committed example fixtures.');
  lines.push('');

  for (const [cat, data] of Object.entries(matrix.categories)) {
    lines.push(`### ${cat} (${data.card_count} concepts)`);
    lines.push('');
    const counts = matrix.support_stages
      .map((s) => `\`${s}\` ${data.stage_validated_counts[s]}`)
      .join(' · ');
    lines.push(`Validated-stage counts: ${counts}`);
    lines.push('');
    lines.push('| Concept | Language | Validated stages | Evidence | Known gaps |', '|---|---|---|---|---|');
    for (const c of data.concepts) {
      const stages = c.validated_stages.length > 0 ? c.validated_stages.map((s) => `\`${s}\``).join(' ') : '—';
      lines.push(`| ${mdCell(c.title)} | ${mdCell(c.language)} | ${stages} | ${mdCell(c.evidence)} | ${c.known_gaps > 0 ? c.known_gaps : '—'} |`);
    }
    lines.push('');
  }

  lines.push(provenanceFooter(commit, dateIso, `Input: the language-pack cards in the pinned Book checkout \`website/.learn-src/src/language-pack/\` (Book pin \`${matrix.pack.book_pin}\`).`));
  lines.push('');
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

async function generate(outGeneratedDir, outPagesDir) {
  mkdirSync(outGeneratedDir, { recursive: true });
  mkdirSync(outPagesDir, { recursive: true });

  const commit = gitShortHead();
  const dateIso = new Date().toISOString();
  const cliVersion = run(SYSML_BIN, ['--version']).trim();

  console.error('· collecting CLI command tree from --help …');
  const cliCommands = collectCliCommands();
  const cli = { tool_version: cliVersion, commands: cliCommands };

  console.error('· starting sysml-api for the /commands catalogue …');
  const service = await collectServiceCommands();

  console.error('· dumping the core diagnostics registry via spec-index …');
  const diagnostics = collectDiagnostics();

  console.error('· aggregating the language pack into the capability matrix …');
  const matrix = collectCapabilityMatrix();

  writeFileSync(path.join(outGeneratedDir, 'cli-commands.json'), stableJson(cli));
  writeFileSync(path.join(outGeneratedDir, 'service-commands.json'), stableJson({ api_version: service.apiVersion, commands: service.catalog }));
  writeFileSync(path.join(outGeneratedDir, 'diagnostics-core.json'), stableJson(diagnostics));
  writeFileSync(path.join(outGeneratedDir, 'capability-matrix.json'), stableJson(matrix));

  const meta = {
    generator: 'node scripts/generate-reference.mjs (run from website/)',
    generated_at: dateIso,
    sysml_rs_commit: commit,
    artifacts: {
      'cli-commands.json': {
        command: `${path.relative(repoRoot, SYSML_BIN)} <subcommand…> --help (recursive)`,
        tool_version: cliVersion,
        input_revision: commit,
      },
      'service-commands.json': {
        command: 'GET /commands + GET /health on a locally started target/release/sysml-api',
        tool_version: `sysml-api ${service.apiVersion}`,
        input_revision: commit,
      },
      'diagnostics-core.json': {
        command: 'cargo run --release -p spec-index -- diagnostics-registry --json',
        tool_version: 'sysml-core error_codes registry',
        input_revision: commit,
      },
      'capability-matrix.json': {
        command: 'aggregate website/.learn-src/src/language-pack/cards/*.json',
        tool_version: `language-pack generator_version ${matrix.pack.generator_version} (spec drop ${matrix.pack.spec_drop})`,
        input_revision: `Book pin ${matrix.pack.book_pin}`,
      },
    },
  };
  writeFileSync(path.join(outGeneratedDir, 'generated-meta.json'), stableJson(meta));

  writeFileSync(path.join(outPagesDir, 'cli-commands.md'), renderCliPage(cli, commit, dateIso, cliVersion));
  writeFileSync(path.join(outPagesDir, 'api-mcp-catalog.md'), renderApiCatalogPage(service, commit, dateIso));
  writeFileSync(path.join(outPagesDir, 'diagnostics.md'), renderDiagnosticsPage(diagnostics, commit, dateIso));
  writeFileSync(path.join(outPagesDir, 'capability-matrix.md'), renderCapabilityMatrixPage(matrix, commit, dateIso));
}

const ARTIFACT_FILES = ['cli-commands.json', 'service-commands.json', 'diagnostics-core.json', 'capability-matrix.json'];
const PAGE_FILES = ['cli-commands.md', 'api-mcp-catalog.md', 'diagnostics.md', 'capability-matrix.md'];

/** Strip generation-time-volatile lines (commit stamp, date) before diffing. */
function normalizeVolatile(text) {
  return text
    .replace(/^last_verified_against: .*$/m, 'last_verified_against: <volatile>')
    .replace(/at sysml-rs commit `[0-9a-f]+` on \d{4}-\d{2}-\d{2}/g, 'at sysml-rs commit <volatile>');
}

async function check() {
  const tmp = mkdtempSync(path.join(os.tmpdir(), 'genref-'));
  const tmpGenerated = path.join(tmp, 'generated');
  const tmpPages = path.join(tmp, 'pages');
  try {
    await generate(tmpGenerated, tmpPages);
    const drifted = [];
    const compare = (committedPath, freshPath, label, normalize) => {
      if (!existsSync(committedPath)) { drifted.push(`${label} (missing from the repo)`); return; }
      let committed = readFileSync(committedPath, 'utf8');
      let fresh = readFileSync(freshPath, 'utf8');
      if (normalize) { committed = normalize(committed); fresh = normalize(fresh); }
      if (committed !== fresh) drifted.push(label);
    };
    for (const f of ARTIFACT_FILES) compare(path.join(GENERATED_DIR, f), path.join(tmpGenerated, f), `src/generated/${f}`);
    for (const f of PAGE_FILES) compare(path.join(PAGES_DIR, f), path.join(tmpPages, f), `src/content/docs/reference/${f}`, normalizeVolatile);
    if (drifted.length > 0) {
      console.error('DRIFT: committed reference artifacts differ from a fresh generation:');
      for (const d of drifted) console.error(`  - ${d}`);
      console.error('Run `node scripts/generate-reference.mjs` and commit the result.');
      process.exit(1);
    }
    console.error('gen-check: committed reference artifacts match a fresh generation.');
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

if (checkMode) {
  await check();
} else {
  await generate(GENERATED_DIR, PAGES_DIR);
  console.error('generate-reference: wrote src/generated/ artifacts and reference pages.');
}
