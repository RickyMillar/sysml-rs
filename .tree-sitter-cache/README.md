# .tree-sitter-cache/

Content-addressable cache for generated `parser.c` files keyed by a SHA-256
hash of the tree-sitter grammar surface (`grammar.js` + `rules/*.js` +
`helpers/*.js` + `generated/*.js`).

## Layout

```
.tree-sitter-cache/
  README.md                   - this file
  baseline-metrics.json       - last-known-good grammar metrics (written by measure.sh)
  <grammar-sha256>/
    parser.c                  - the cached generated parser (~50 MB)
    grammar.json              - tree-sitter intermediate (optional)
    node-types.json           - tree-sitter intermediate (optional)
    provenance.txt            - when/how this entry was generated
```

## Why this exists

A clean `tree-sitter generate --abi 14` + downstream Rust compile of
`parser.c` costs ~50 minutes on this grammar. With this cache, two agents
working on different grammar experiments that converge on the same
intermediate grammar.js share the cost - only the rebuild that genuinely
changed the grammar surface pays the bill.

See `tools/ts-grammar/README.md` for the workflow and the scripts that
read/write this directory.

## Eviction policy

Manual. Each entry is ~50 MB; after a few weeks of grammar churn the
directory will balloon. Prune with:

```bash
tools/ts-grammar/cache-prune.sh --older-than 30d --keep-latest 3
tools/ts-grammar/cache-prune.sh --older-than 7d --dry-run    # see what would go
```

The pruner always retains the N most-recent entries (default 3) regardless
of age, so a careless `--older-than 1h` cannot wipe everything.

## What is gitignored

The directory itself is tracked (this README + the eventual
`baseline-metrics.json`). Everything *inside* the per-hash subdirectories
is gitignored - see the root `.gitignore`.
