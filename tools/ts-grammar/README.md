# tools/ts-grammar/

Grammar iteration infrastructure for the SysML tree-sitter parser.

## Why this exists

A clean `tree-sitter generate --abi 14` on this grammar produces a ~50 MB
`parser.c` and takes about 50 minutes wall-clock once you include the
downstream Rust compile of that C file. Multiplied across a handful of
parallel grammar experiments, that's a full day of wasted CPU per loop.

The scripts in this directory turn grammar iteration into a parallelizable
workflow:

* **One canonical cache.** `parser.c` files are keyed by `sha256(grammar
  surface)` and stored in `.tree-sitter-cache/<hash>/parser.c`. Different
  worktrees that arrive at the same intermediate grammar share a single
  generate.
* **Disposable worktrees.** `new-experiment.sh` spins up a fresh git
  worktree on its own branch, seeded with a cached `parser.c` so the agent
  can start compiling Rust immediately.
* **Pre-build sanity check.** `measure.sh` reads `parser.c` and flags
  build-time cliffs (>10% growth in any of STATE/SYMBOL/ALIAS counts vs the
  recorded baseline). Catches state explosions before a 10-minute Rust
  compile burns.

## The workflow

A grammar agent picks a gap from
(produced by a sibling agent), then:

```bash
# 1. Spin a fresh worktree, seeded with the current parser.c from cache.
tools/ts-grammar/new-experiment.sh G05
cd .worktrees/grammar-G05

# 2. Edit grammar.js / rules/*.js / helpers/*.js.

# 3. Generate + cache the new parser.c. ~50 min on cache MISS, ~1 s on HIT
#    (if your edits happened to converge on a grammar another agent already
#    tried).
tools/ts-grammar/cache-build.sh

# 4. Sub-second sanity check on the generated parser before paying for the
#    Rust compile.
tools/ts-grammar/measure.sh

# 5. Build + test downstream Rust if metrics look sane.
cargo build --release -p sysml-parser-incremental    # ~10 min
cargo test  --release -p sysml-parser-incremental    # CST corpus tests

# 6. If green, commit the grammar edits (NOT parser.c - it stays gitignored).
git add tree-sitter/rules/<file>.js tree-sitter/grammar.js
git commit -m "grammar: <gap>: <what changed>"
```

## Scripts

| Script | Purpose |
|--------|---------|
| `cache-build.sh`       | Hash grammar surface; restore cached parser.c (HIT) or generate + cache (MISS). |
| `cache-prune.sh`       | Evict stale cache entries; keeps N most recent regardless of age. |
| `new-experiment.sh`    | Create a worktree at `.worktrees/grammar-<name>` on a fresh branch and seed parser.c. |
| `measure.sh`           | Report STATE_COUNT / SYMBOL_COUNT / parser.c size; warn on >10% growth vs baseline. |

All scripts are idempotent and safe to re-run.

## Anti-patterns

* **Do not edit `parser.c` files inside `.tree-sitter-cache/<hash>/`.**
  They are content-addressed; tampering invalidates the cache contract
  silently. Edit `grammar.js` / `rules/*.js`, then run `cache-build.sh`.
* **Do not run `tree-sitter generate` directly in a worktree** without
  going through `cache-build.sh`. You'll pay 50 min for a result that may
  already be cached, and the cache will never see your output.
* **Do not commit `parser.c`.** It is gitignored. CI generates it from
  scratch (no cache). Local cache only.
* **Do not `git stash` / `git checkout --` inside a worktree** - another
  parallel agent likely owns uncommitted changes. The repo-level CLAUDE.md
  rule applies inside worktrees too.

## Tree-sitter facts (locked)

* **CLI version**: `tree-sitter-cli` 0.26.5. Other versions are untested
  and have caused regressions.
* **ABI**: 14 only. ABI 15 SIGSEGVs the Rust tree-sitter crate.
* **Canonical patch**: `SMALL_STATE_THRESHOLD=200` saves ~19 MB off
  `parser.c`. See `crates/lang/sysml-parser-incremental/tree-sitter/OPTIMIZATION_GUIDE.md`.
* **Grammar lives at**: `crates/lang/sysml-parser-incremental/tree-sitter/`.
  Despite the simple `tree-sitter/grammar.js` references in some docs,
  there is no top-level `tree-sitter/` directory in this repo.

## Cross-references

  - per-gap intake for the future grammar agent (built by a sibling agent
  in parallel with this infrastructure work).
* `Architectural-cleanup/tree-sitter-canonical-plan/PROGRESS.md`
  - bucket rollup.
* `crates/lang/sysml-parser-incremental/CLAUDE.md` - grammar architecture
  and pitfalls (the "ALL optionals INLINE" rule, conflict declarations,
  rule merging).
* `crates/lang/sysml-parser-incremental/tree-sitter/OPTIMIZATION_GUIDE.md`
  - historical optimization notes including SMALL_STATE_THRESHOLD.
* `.tree-sitter-cache/README.md` - cache layout and eviction policy.

## Cold start

If `crates/lang/sysml-parser-incremental/tree-sitter/src/parser.c` is
missing locally (e.g. fresh checkout), the first `cache-build.sh` will run
`tree-sitter generate` and pay the ~50 min cost. After that the result is
cached in `.tree-sitter-cache/<hash>/` and every future worktree on the
same grammar gets the hit. Run `tools/ts-grammar/measure.sh
--capture-baseline` after the first successful generate to seed
`.tree-sitter-cache/baseline-metrics.json`.
