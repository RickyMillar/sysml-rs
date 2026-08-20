# Parsing Architecture

This guide covers how SysML v2 source text is turned into a `ModelGraph`. For an architecture-level overview of where parsing fits, see [00-architecture.md](00-architecture.md).

## One parser: tree-sitter

sysml-rs has a **single parser**: `sysml-parser-incremental`, a tree-sitter
implementation. It implements `sysml_parser_trait::Parser`
(`fn parse(&self, &[SysmlFile]) -> ParseResult`) at
`crates/lang/sysml-parser-incremental/src/lib.rs:266`, so every consumer — LSP,
ide-db, CLI, runtime, library loading — goes through the same code path and
produces the same canonical IR (`sysml_core::ModelGraph`).

> **Historical note.** sysml-rs used to ship a second parser, `sysml-parser-batch`
> (a Pest PEG implementation) behind the same `Parser` trait, with cross-parser
> equivalence gates keeping the two in sync. Once tree-sitter implemented
> `Parser` and reached parity, the Pest crate was **deleted** (commits
> `054ea91a`, `079cb801`) along with its equivalence tests. If you find docs or
> `use sysml_parser_batch::…` statements that still reference it, they are stale.

## Pipeline

```
SysmlFile (path + text)
        │
        ▼  tree_sitter::Parser::parse  (incremental, error-recovering)
   tree_sitter::Tree (CST, !Send)
        │
        ▼  build_model_graph (ast_builder/, behind the "semantic" feature)
   ModelGraph (elements + relationships) + Diagnostics
        │
        ▼  ParseResult  (→ into_resolved() / into_validated() chaining)
```

Key implementation facts:

- Entry: `TreeSitterParser::parse(&[SysmlFile])` (the `Parser` impl) at
  `crates/lang/sysml-parser-incremental/src/lib.rs`.
- `tree_sitter::Tree` is `!Send`, so trees are produced and consumed inside the
  same call. CLI inspection (`sysml inspect <file> --cst`) is the only consumer
  that exposes the raw tree.
- `OwningMembership` elements are synthesized for every nested element with the
  correct visibility. Relationship targets (`:>`, `:`, `:>>`) are stored as
  `unresolved_*` properties on the parent element for later resolution (see
  [03-resolution.md](03-resolution.md)).
- Error contract: `ERROR` nodes are skipped but siblings are still processed, so
  a syntactically invalid region degrades gracefully (capped at 3 diagnostics
  per `ERROR` node to avoid flooding). This error tolerance is the reason
  tree-sitter is the right fit for an IDE-first toolchain.

## Two operating modes (feature flags)

`sysml-parser-incremental` is split by the `semantic` Cargo feature so that the
fast syntax-only path stays a leaf crate:

- **Default (no `semantic` feature)** — `TreeSitterParser` only exposes CST-level
  access via the `FastParser` trait (`parse_tree`, `parse_tree_incremental`). The
  crate does **not** depend on `sysml-core` in this mode; a local `SysmlFile`
  fallback type is used so syntax highlighters can stay lean.
- **`semantic` feature** — pulls in `sysml-core`, re-exports the canonical
  `SysmlFile` from `sysml-parser-trait`, and provides `build_model_graph`
  (CST → `ModelGraph`) plus the `impl Parser for TreeSitterParser`. The LSP,
  ide-db, CLI, runtime, and library loading all enable this feature.

The cfg-gated `SysmlFile` re-export is intentional: `sysml-parser-trait` depends
on `sysml-core`, so a flat re-export would break the no-`semantic` layering
invariant. Don't flatten it.

## Editing the grammar (tree-sitter rules)

The tree-sitter grammar is modular JS under
`crates/lang/sysml-parser-incremental/tree-sitter/rules/*.js` — 11 rule modules
plus factory helpers in `helpers/patterns.js` and LR conflict declarations in
`helpers/conflicts.js`. Hard constraints (these are *architectural*, not
preferences):

- **Optionals must stay inline.** Every attempt to extract
  `_usage_name`/`_usage_header`/`_def_header` into a shared rule has caused
  >65k-state explosion. SysML's optional-heavy, order-insensitive shape multiplies
  LR/GLR parse states fast; rule merges keep the generator tractable but reduce
  CST precision, which the AST builder then compensates for with keyword scans and
  sibling-augmentation heuristics.
- **ABI 14 is mandatory.** ABI 15 SIGSEGVs the Rust tree-sitter crate. CI
  generates `parser.c` with `tree-sitter generate --abi 14`.
- **Generation is expensive (tens of minutes).** Batch *all* grammar edits before
  running `npx tree-sitter generate`. Never edit-generate-edit-generate.
- `parser.c` is gitignored (~48MB); CI regenerates it.
- **Grammar changes break the query layer.** Node/field renames ripple into
  `highlights.scm`/`folds`/`locals` queries and the AST builder. Change the
  grammar and the queries together.

The single source of truth for grammar metrics (`STATE_COUNT`, `parser.c` size,
conflict count, corpus case count) is
`crates/lang/sysml-parser-incremental/tree-sitter/TREE_SITTER_STATUS.md`
(updated by `update_status.sh`) — read it rather than hard-coding numbers here.
The constraint playbook is
`crates/lang/sysml-parser-incremental/tree-sitter/OPTIMIZATION_GUIDE.md`.

## AST conversion (CST → ModelGraph)

`build_model_graph(&tree, &source, &path)` in
`crates/lang/sysml-parser-incremental/src/ast_builder/` walks the CST and emits
`Element` + `Relationship` shapes. Because the grammar deliberately under-merges
some structures to keep the state count down, the builder uses keyword scans and
sibling-augmentation heuristics to recover semantics the CST does not encode
directly (see the merge/heuristic comments in `ast_builder/`).

## Grammar debugging workflow

### 1. Identify the failing file

```bash
SYSML_CORPUS_PATH=references/sysmlv2 \
cargo test -p sysml-spec-tests corpus_coverage -- --ignored --nocapture 2>&1 | head -100
```

### 2. Look up the xtext rule

```bash
grep -n "StateDefinition" \
  references/sysmlv2/SysML-v2-Pilot-Implementation/org.omg.sysml.xtext/src/org/omg/sysml/xtext/SysML.xtext
```

Or delegate via the research agent: `Task(subagent_type=sysml-research, prompt="Find the xtext grammar rule for StateDefinition")`.

### 3. Patch the rule module

Edit the relevant `tree-sitter/rules/*.js` module (and any affected query files).
Respect the "optionals stay inline" constraint above.

### 4. Test the fix

```bash
# Tree-sitter internal corpus (~137 cases, a few seconds)
cd crates/lang/sysml-parser-incremental/tree-sitter && npx tree-sitter test

# Semantic-feature Rust tests (CST → ModelGraph)
cargo test -p sysml-parser-incremental --features semantic

# Full corpus
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests corpus_coverage -- --ignored
```

## Expected failures

`crates/testing/sysml-spec-tests/data/expected_failures.txt` tracks corpus files
that don't parse yet. Format:

```
# Comment lines start with #
**/SomeFile.sysml
specific/path/to/file.sysml
```

This list should shrink over time. When you fix a parser bug, remove the
now-passing file from the list — coverage tests will fail loudly if you forget.

## Performance notes

- **Incremental.** `parse_tree_incremental` reuses the previous tree to reparse
  only changed regions; the typical LSP keystroke reparse is sub-millisecond.
- **Per-file / per-edit.** Parsing is not internally parallelised across files;
  the salsa layer (`sysml-ide-db`) provides caching and incrementality instead.

## Where to look in the code

| Job | File |
|-----|------|
| `Parser` impl (tree-sitter) | `crates/lang/sysml-parser-incremental/src/lib.rs` (semantic-feature gated, line 266) |
| `FastParser` (CST-only) trait | `crates/lang/sysml-parser-incremental/src/lib.rs` (line 180) |
| Tree-sitter grammar | `crates/lang/sysml-parser-incremental/tree-sitter/rules/*.js` |
| LR conflict declarations | `crates/lang/sysml-parser-incremental/tree-sitter/helpers/conflicts.js` |
| CST → ModelGraph | `crates/lang/sysml-parser-incremental/src/ast_builder/` |
| Grammar metrics / status | `crates/lang/sysml-parser-incremental/tree-sitter/TREE_SITTER_STATUS.md` |
| Grammar constraint playbook | `crates/lang/sysml-parser-incremental/tree-sitter/OPTIMIZATION_GUIDE.md` |
| Parser trait + wire types | `crates/lang/sysml-parser-trait/src/lib.rs` |

## Related documentation

- [00-architecture.md](00-architecture.md) — where parsing sits in the overall layering.
- [03-resolution.md](03-resolution.md) — what happens to `unresolved_*` properties after parsing.
- `crates/lang/sysml-parser-incremental/CLAUDE.md` — full tree-sitter crate playbook.
