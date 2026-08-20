# sysml-parser-incremental

The canonical SysML v2 parser — a Tree-sitter CST front end that also builds a full `ModelGraph`. Fast, incremental, error-recovering; the sole parser for the entire workspace.

`Layer 2 · lang` · `CST parser` · `canonical parser` · `crate-type: rlib` · `grammar: Tree-sitter ABI 14`

## Overview

`sysml-parser-incremental` wraps the in-tree `tree-sitter-sysml` grammar and exposes it to the rest of the workspace. It is the **only** parser used by production code, tests, benches, and examples — the former Pest PEG crate (`sysml-parser-batch`) has been **deleted**. Tree-sitter's incremental, error-recovering nature makes it ideal for IDE workloads: it keeps parsing past syntax errors so valid regions of a half-typed file still yield useful structure.

The crate has two distinct surfaces, gated by the `semantic` cargo feature:

**Default (no features).**

CST-only. Depends on nothing semantic. Produces a `SyntaxNode` tree (or the raw `tree_sitter::Tree`) for syntax highlighting, folding, bracket matching, and outline. **No `sysml-core` dependency.**

**`semantic` feature.**

Adds `sysml-core` + `sysml-parser-trait`. Enables `build_model_graph` (CST → `ModelGraph`) and the `sysml_parser_trait::Parser` impl on `TreeSitterParser`. Drives semantic IDE features with graceful degradation on syntax errors.

>  **The old README was a stub.** Earlier docs described this crate as pre-grammar scaffolding (`StubTreeSitterParser` as the headline API, "does not depend on sysml-core", "Future Work: when a grammar is available"). All of that is obsolete: a full modular grammar (11 rule modules, ~45 conflict declarations, 334 corpus cases) ships today, `TreeSitterParser` is the production API, and the `semantic` feature pulls in `sysml-core`.

## Where it sits

```text
consumers sysml-lsp-server sysml-ide-db sysml-cli sysml-spec-tests
▲ parse() · parse_tree() · build_model_graph()
this crate sysml-parser-incremental tree-sitter-sysml (grammar)
▼ depends on
always sysml-span tree-sitter
semantic sysml-core sysml-parser-trait
codegen sysml-codegen
```

## Two-mode parse flow

```text
input source text + path
▼ tree_sitter::Parser (grammar = ABI 14)
CST tree_sitter::Tree
default → SyntaxNode tree → extract_outline · highlighting · folding
semantic → ast_builder dispatch → ModelGraph + diagnostics
```

## Public API

#### `— *struct TreeSitterParser` — *always*

The production parser. `Clone + Default + Debug` — cheap to copy because it holds no state (a `tree_sitter::Parser` is created on demand per call, since that type is not `Clone`).

```
impl TreeSitterParser {
    pub fn new() -> Self;
    // Raw CST — the entry points the crate is named for.
    pub fn parse_tree(&self, source: &str) -> Option<tree_sitter::Tree>;
    pub fn parse_tree_incremental(
        &self,
        source: &str,
        old_tree: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree>;
}
```

Implements `FastParser` (always) and, under the `semantic` feature, `sysml_parser_trait::Parser` (`name()` → `"tree-sitter"`).

#### `— *trait FastParser` — *always*

The CST-level contract. No `sysml-core` required.

```
pub trait FastParser {
    fn parse_cst(&self, file: &SysmlFile) -> SyntaxNode;
    fn supports_incremental(&self) -> bool { false }
}
```

`TreeSitterParser::supports_incremental()` returns `true`.

#### `— *struct SyntaxNode` — *always*

A simplified, owned CST node (named nodes only — anonymous punctuation/keywords are dropped during conversion). Fields: `kind: String`, `span: Span`, `children: Vec<SyntaxNode>`, `is_error: bool`.

```
node.text(source);                 // &str slice for this node's span
node.child_by_kind("identifier");  // first child of a kind
node.find_by_kind("part_usage");   // all descendants of a kind
node.has_errors();                 // self-or-descendant error?
node.errors();                     // Vec<&SyntaxNode> of error nodes
```

#### `— *fn build_model_graph · struct ModelGraphResult` — *semantic*

CST → semantic model. Walks the Tree-sitter tree and produces a (possibly partial) `ModelGraph` plus diagnostics. Per-ERROR-node diagnostics are capped (`MAX_DIAGNOSTICS_PER_ERROR_NODE = 3`) to avoid flooding.

```
pub fn build_model_graph(
    tree: &tree_sitter::Tree,
    source: &str,
    file_path: &str,
) -> ModelGraphResult;

pub struct ModelGraphResult {
    pub graph: ModelGraph,                 // sysml_core::ModelGraph
    pub diagnostics: Vec<Diagnostic>,      // sysml_span::Diagnostic
}
// ModelGraphResult::has_errors() -> bool
```

#### `— *struct ExpressionBuilder<'s>` — *semantic*

Re-exported from `expression_elements`. Converts expression CST subtrees into expression elements during `ModelGraph` construction (literals, operators, feature chains, invocations, etc.).

#### `— *struct OutlineItem · fn extract_outline` — *always*

Lightweight navigation outline derived from a `SyntaxNode` tree, for IDE symbol/outline views without needing the semantic model.

```
pub fn extract_outline(root: &SyntaxNode, source: &str) -> Vec<OutlineItem>;
// OutlineItem { name, kind, span, children }
```

#### `— *struct SysmlFile` — *always*

The parse-input wire type (`path` + `text`). Under the `semantic` feature this re-exports `sysml_parser_trait::SysmlFile`; otherwise a structurally identical local copy is used so `FastParser` stays usable without dragging in `sysml-core`.

#### `— *struct StubTreeSitterParser` — *legacy / test-only*

A string-scanning fallback that recognises only `package X {}`. Predates the real grammar; retained for tests and degenerate fallback. **Not** the production path — use `TreeSitterParser`.

## Usage

### CST only (default features)

```
use sysml_parser_incremental::{TreeSitterParser, FastParser, SysmlFile, extract_outline};

let parser = TreeSitterParser::new();
let source = "package Vehicle { part engine; }";
let file = SysmlFile::new("model.sysml", source);

let cst = parser.parse_cst(&file);
if cst.has_errors() {
    for err in cst.errors() {
        eprintln!("syntax error at {:?}", err.span);
    }
}
for item in extract_outline(&cst, source) {
    println!("{}: {}", item.kind, item.name);
}
```

### Incremental re-parse (default features)

```
use sysml_parser_incremental::TreeSitterParser;

let parser = TreeSitterParser::new();
let tree = parser.parse_tree("package P { part a; }").unwrap();
// after an edit, reuse unchanged regions:
let tree2 = parser
    .parse_tree_incremental("package P { part a; part b; }", Some(&tree))
    .unwrap();
```

### Semantic model (`--features semantic`)

```
use sysml_parser_incremental::{TreeSitterParser, build_model_graph};

let parser = TreeSitterParser::new();
let source = "package Vehicle { part def Engine; }";
if let Some(tree) = parser.parse_tree(source) {
    let result = build_model_graph(&tree, source, "model.sysml");
    // result.graph: sysml_core::ModelGraph (partial if there were errors)
    // result.diagnostics: Vec<sysml_span::Diagnostic>
    println!("{} elements", result.graph.elements.len());
}
```

## Cargo features

| Feature | Adds deps | Enables | Used by |
|---|---|---|---|
| `default` (empty) | — | CST surface: `TreeSitterParser`, `SyntaxNode`, `FastParser`, outline | syntax-only consumers |
| `semantic` | `sysml-core`, `sysml-parser-trait` | `ast_builder`, `build_model_graph`, `ExpressionBuilder`, `Parser` impl | sysml-lsp-server, sysml-ide-db, sysml-cli, sysml-spec-tests |
| `codegen` | `sysml-codegen` | `generate_ts_tokens` & `validate_ts_coverage` binaries | build/maintenance tooling |
| `tracing` | `tracing` (opt) | trace spans around parse calls (byte counts, timing) | profiling / diagnostics |

## Source modules

| Path | Responsibility | Gate |
|---|---|---|
| `src/lib.rs` | TreeSitterParser, SyntaxNode, FastParser, CST conversion, outline, StubTreeSitterParser | always |
| `src/ast_builder/` | CST → ModelGraph (directory module, 13 files: mod, dispatch, definitions, usages, connectors, states, requirements, imports, packages, typings, keying, node_helpers, tests) | semantic |
| `src/ast_builder/dispatch.rs` | Keyword-field dispatch for merged grammar rules | semantic |
| `src/expression_elements.rs` | ExpressionBuilder — expression CST → expression elements | semantic |
| `src/bin/generate_ts_tokens.rs` | Codegen: keyword/operator/enum token tables from Xtext specs | codegen |
| `src/bin/validate_ts_coverage.rs` | Grammar-rule vs Xtext coverage report | codegen |
| `tree-sitter/grammar.js` | Grammar entry point: config, extras, word, conflicts, rule assembly | build |
| `tree-sitter/rules/*.js` | 11 rule modules: namespaces, common, definitions, usages, actions, states, connectors, requirements, expressions, types, kerml | build |
| `tree-sitter/helpers/patterns.js` | Factory functions (defRule, binaryExpr, …) that reduce rule boilerplate | build |
| `tree-sitter/helpers/conflicts.js` | ~45 programmatic LR conflict declarations | build |
| `tree-sitter/generated/*.js` | Auto-generated keyword/operator/enum tables (committed for CI) | build |
| `tree-sitter/queries/` | highlights.scm, folds.scm, locals.scm, brackets.scm, indents.scm | build |
| `tree-sitter/test/corpus/` | ~334 internal CST test cases across 11 .txt files | test |
| `tree-sitter/src/parser.c` | Generated parser (~52 MB, gitignored, built at CI/dev time) | build |

## Grammar pipeline

```text
spec SysML.xtext (references/)
▼ generate_ts_tokens (codegen)
tokens generated/*.js (keywords, operators, enums)
▼ grammar.js assembles rules/*.js + helpers/*
grammar grammar.js
▼ tree-sitter generate --abi 14 (~57 min — batch ALL edits first)
artifact parser.c (~52 MB, gitignored)
```

## Invariants & pitfalls

- **ABI 14 is mandatory.** ABI 15 SIGSEGVs with the Rust tree-sitter crate. Always generate with `--abi 14`.

- **Keep all optionals inline in grammar rules.** Extracting hidden subrules (`_usage_header`, `_def_header`) caused >65k state explosion. The single most important grammar constraint — do not re-attempt.

- **Generation is expensive (~57 min).** Batch every grammar edit before running `tree-sitter generate`; never edit-generate iteratively.

- **Rule merging for state reduction.** Structurally identical rules are merged with a `keyword` field; `ast_builder` dispatch re-splits on that field.

- **Error-recovery contract.** `ast_builder` skips ERROR nodes but keeps processing siblings — valid regions still produce elements; errors become capped diagnostics.

- **Conversion drops anonymous nodes.** `SyntaxNode` keeps only *named* nodes, so punctuation/keywords are not in the tree.

- **Renamed crate.** Formerly `sysml-ts`; some comments/docs may still use the old name and refer to a single-file `ast_builder.rs` (now a directory module).

## Testing

```
# Internal Tree-sitter corpus (~334 cases) — run from the grammar dir
cd crates/lang/sysml-parser-incremental/tree-sitter && npx tree-sitter test

# Library-syntax coverage (scripts live in tree-sitter/)
cd crates/lang/sysml-parser-incremental/tree-sitter && ./test_library.sh --fail-only
cd crates/lang/sysml-parser-incremental/tree-sitter && ./update_status.sh

# Rust unit tests (includes the embedded 13-pattern stage gate)
cargo test -p sysml-parser-incremental
cargo test -p sysml-parser-incremental --features semantic

# Grammar-rule coverage vs Xtext
cargo run -p sysml-parser-incremental --features codegen --bin validate_ts_coverage
```

Live grammar metrics (case counts, coverage) are tracked in `tree-sitter/TREE_SITTER_STATUS.md` (regenerated by `update_status.sh`) — treat that file as the source of truth, not any number hard-coded in docs.

## Dependencies

**Upstream.**

- `sysml-span` — Span / Diagnostic (always)

- `tree-sitter` — runtime (always)

- `tree-sitter-sysml` — in-tree grammar crate (always)

- `sysml-core`, `sysml-parser-trait` — `semantic` only

- `sysml-codegen` — `codegen` only

- `tracing` — `tracing` only

**Downstream.**

- `sysml-lsp-server`

- `sysml-ide-db`

- `sysml-cli`

- `sysml-spec-tests`

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
