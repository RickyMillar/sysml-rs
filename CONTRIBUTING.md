# Contributing to sysml-rs

Thanks for considering it. This document covers getting a working environment,
how to run the tests that matter for your change, the rules around generated
files, and what a reviewable pull request looks like.

Two things are worth knowing before you start:

- **The specification is the authority.** sysml-rs implements OMG SysML v2 and
  KerML. Where the implementation and the specification disagree, the
  implementation is wrong. A change to language behaviour needs a citation, not
  an argument from taste — see [Changing language behaviour](#changing-language-behaviour).
- **Prefer failing hard to degrading quietly.** When something cannot be done —
  an unresolved name, a missing manifest, an unsupported construct — the right
  answer is a precise diagnostic, not a fallback that produces a plausible
  wrong result. Silent fallbacks become invisible bugs.

Please also read the [Code of Conduct](CODE_OF_CONDUCT.md).

## Environment setup

### Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | CI builds on 1.92.0 | No MSRV is declared yet; a recent stable is fine |
| Node.js | 20 | The tree-sitter CLI, and the packages under `editors/` |
| `tree-sitter-cli` | **exactly 0.26.5** | Parser generation; see the pinning note below |
| C compiler | any working `cc` | Compiles the generated parser |
| `git`, `curl`, `awk`, `sha256sum` | any | Used by the reference fetch script |
| Git LFS | not required | Public clones carry no LFS objects |

### 1. Fetch the specification sources

The OMG materials are not vendored in this repository. A pinned fetch script
reconstructs `references/sysmlv2/` from upstream at fixed revisions, verifying
each item against a recorded checksum:

```bash
tools/fetch-references/fetch.sh
```

It is a POSIX shell script, not a Rust tool, precisely because it has to run on
a checkout that cannot compile yet. Budget about 210 MB of disk and a few
minutes on a first run — most of that is cloning the pilot implementation.
Re-runs are idempotent: anything that already verifies is skipped, so a second
run finishes in seconds without touching the network.

Two other modes are useful once you have a tree: `fetch.sh verify` re-checks an
existing tree offline, and `fetch.sh list` prints the pinned inventory.

The references root is resolved as `--root DIR`, then `$SYSML_REFS_DIR`, then
`<repo>/references/sysmlv2`. That is deliberately the same order `sysml-core`'s
build script and `generate_from_xtext.sh` use, so a tree fetched with
`SYSML_REFS_DIR` set is found by `cargo build` with no further configuration.

Do this before anything else. `crates/lang/sysml-core/build.rs` generates the
element-kind enum, typed property accessors, cross-reference tables, and the
semantic-validation dispatcher from the fetched TTL and Xtext files. Without
them `cargo build` fails in `build.rs`, not at link time — if you see a build
error naming `references/sysmlv2`, this step is the fix.

### 2. Generate the tree-sitter parser

```bash
npm install -g tree-sitter-cli@0.26.5
cd crates/lang/sysml-parser-incremental/tree-sitter
./generate_from_xtext.sh          # emits generated/{keywords,operators,enums}.js
tree-sitter generate --abi 14     # emits src/parser.c
```

Both outputs are gitignored. `generate_from_xtext.sh` derives the keyword,
operator, and enum tables directly from the OMG Xtext grammars, so it must run
after step 1 and before `tree-sitter generate`. It honours `SYSML_REFS_DIR` if
your reference checkout lives outside the repository.

**Why `--abi 14`, exactly.** The Rust `tree-sitter` crate this workspace builds
against reads ABI 14. A parser generated at ABI 15 compiles cleanly and then
segfaults at parse time — the failure looks like a memory-corruption bug in
your change, not a version mismatch, which is why the flag is called out
everywhere it appears. The CLI version is pinned for the same class of reason:
0.26.5 is the version the committed grammar and the regeneration diffs are
known-good against.

Generation takes a long time — tens of minutes is normal for a grammar this
size. Batch your grammar edits; do not iterate one rule at a time.

### 3. Build and smoke-test

```bash
cargo build --release
./target/release/sysml check examples/espresso-pump-hybrid/Physics/HydraulicConstraints.sysml
```

Always build and run in **release**. The physics solver is slow enough in a
debug build to look hung.

`cargo build` builds the workspace's default members, which excludes the Tauri
desktop shell — roughly a third of the dependency graph exists only for it.
Build it explicitly with `cargo build -p sysml-desktop` or
`cargo build --workspace` if you are working on the app.

### 4. Front end, only if you are touching `editors/`

The editor packages each manage their own `node_modules` and lockfile; there is
no npm workspace and no bootstrap script. Order matters for one pair:

```bash
cd editors/expression-view && npm ci    # must come first
cd ../simulation-app        && npm ci
```

`simulation-app` consumes `@sysml-rs/expression-view` as a `file:` dependency.
Its `lib/` is committed but its `node_modules` is not, and rollup resolves
`katex` through the real path rather than through `simulation-app`'s own tree —
so installing `simulation-app` first leaves the build unable to resolve `katex`.
Install `expression-view` first and it works. The VS Code extension is
independent: `cd editors/vscode && npm ci`.

If you ever regenerate a `package-lock.json`, do it with **npm 10**
(`npx -y npm@10 install --package-lock-only`), which is what CI runs. npm 11
silently drops top-level optional-peer entries (`@emnapi/*`) that npm 10 fails
closed on — a lock generated by the older major satisfies both, the reverse
does not, and the breakage only shows up in CI.

## Workspace orientation

The workspace is layered, and dependencies only ever point **down**. Layer 0
holds identity and source-location primitives (`sysml-id`, `sysml-span`,
`sysml-project`, `sysml-manifest`, `codegen`). Layer 1 is `sysml-core` — the
`Element` / `Relationship` / `ModelGraph` semantic model, plus name resolution,
validation, and elaboration. Layer 2 is the text frontend
(`sysml-parser-trait`, and `sysml-parser-incremental`, which is the sole
parser). Layer 3 adds the language features that consume a complete graph:
`sysml-runtime` (execution, constraints, physics/DAE) and `sysml-diagram`.
Layer 4 is tooling infrastructure — `sysml-ide-db` (the salsa incremental
database), `sysml-resolve`, `sysml-query`, `sysml-store`. Layer
5 is `sysml-service` and the four transports that dispatch through it:
`sysml-cli`, `sysml-lsp-server`, `sysml-api`, `sysml-mcp`. Everything under
`crates/lang/` implements the language; everything under `crates/tooling/`
wraps it.

[`docs/developer_guide/00-architecture.md`](docs/developer_guide/00-architecture.md)
has the full rules. Two of them decide most review questions:

- **Code lives in the lowest reasonable crate.** If a helper exists only to
  paper over a missing primitive one layer down, build the primitive instead.
- **Functionality has one home.** A capability is implemented once in the
  service layer and reached from every transport. A transport that calls
  `sysml-ide-db` or `sysml-runtime` directly is reintroducing a bypass path
  that a large refactor already removed, and will be sent back.

## Testing

### Focused runs

Test the crate you changed, not the workspace, while you iterate:

```bash
cargo test -p sysml-core
cargo test -p sysml-runtime
cargo test -p sysml-service
cargo test -p sysml-core resolution     # filter by test-name substring
```

`crates/testing/sysml-spec-tests` is the conformance and cross-transport
regression net: parser corpus coverage, spec-derived element and property
gates, frozen service-command fixtures, and full parse → elaborate → execute
pipeline tests. It is the crate most likely to catch a change you did not
expect to be behavioural:

```bash
cargo test -p sysml-spec-tests --release
```

Some of its baselines step long trajectories and are `#[ignore]`d, so a plain
`cargo test` does not run them. `scripts/run-gates.sh` exists so nobody has to
rediscover the invocations:

```bash
scripts/run-gates.sh            # quick standing gates (default)
scripts/run-gates.sh --full     # standing gates plus full crate suites
```

Run the full form before opening a PR that touches the runtime or language
semantics.

A handful of tests write into the user cache directory or bind sockets and will
fail under a sandboxed shell for environmental reasons rather than because you
broke something. `sysml-api`'s tests share a service instance and need
`--test-threads=1`.

### Snapshots

Snapshot tests use [`insta`](https://insta.rs). Never hand-edit a `.snap` file.

```bash
cargo insta review    # walk each diff and accept or reject it
```

Accepting a snapshot is a **claim that the new output is correct**, so it comes
with two obligations: commit the updated `.snap` alongside the source change in
the same commit, and say in the PR description why the output changed. A
snapshot-only commit with no source change and no rationale is the signature of
drift that nobody looked at, and reviewers will block it. If a snapshot changed
and you cannot explain why, that is the bug — do not accept it.

## Generated files

Several artifacts in this tree are generated. Editing the output instead of the
input is the most common first-contribution mistake.

| Artifact | Generated from | Rule |
|---|---|---|
| `crates/lang/sysml-parser-incremental/tree-sitter/src/parser.c` | `grammar.js` via `tree-sitter generate --abi 14` | Gitignored. Never committed — it is ~80 MB. Regenerate locally; CI generates its own. |
| `.../tree-sitter/generated/*.js` | OMG Xtext grammars via `generate_from_xtext.sh` | Gitignored. Regenerate rather than hand-editing keyword tables. |
| `*.generated.rs` — the `ElementKind` enum, value enums, typed property accessors, cross-reference tables, and the semantic-validation dispatcher | TTL / XMI / Xtext via `crates/lang/sysml-core/build.rs` | Not in the source tree at all; they are written into `OUT_DIR` under `target/` and `include!`d. To change one, change the generator in `crates/lang/codegen/` or the spec input, then rebuild. |
| `references/sysmlv2/**` (everything else) | Upstream, via `tools/fetch-references` | Not ours. Never edit in place; a local edit will be silently overwritten and will not reproduce for anyone else. |

### When you change the grammar

Grammar work has a specific loop, because regeneration is expensive:

1. Reproduce the parse first with `tree-sitter parse` on a minimal file, and
   confirm the actual CST shape. The rule you suspect is often not the rule at
   fault.
2. Edit `grammar.js`, batching every change you intend to make.
3. Regenerate once (`tree-sitter generate --abi 14`).
4. Run the grammar's own corpus (`npx tree-sitter test`) and then
   `cargo test -p sysml-parser-incremental`.
5. Run `cargo test -p sysml-spec-tests --release`. Grammar changes move
   coverage snapshots; expect to review and explain the diff.

## Changing language behaviour

Anything that changes how a SysML construct is parsed, resolved, validated, or
executed must cite its source, in the code comment or the PR description or
both. Cite the highest applicable source:

1. **The OMG specification document** — prose, abstract syntax, and notation
   tables. Cite the clause number.
2. **The normative standard model library.** In SysML v2 these models *are* the
   semantics of library constructs like `VerdictKind` or `RequirementCheck`.
   Cite the library file and definition.
3. **The metamodel TTL** (`SysML-vocab.ttl`, `KerML-shapes.ttl`, …).
4. **The Xtext grammars.** Cite rules **by name**, not by line number — line
   numbers rot the moment upstream renumbers.

Explicitly **not** normative, and not acceptable as the only justification: the
pilot implementation's example models, and our own `examples/` corpus. Both are
fallible illustrations. A change that makes the corpus byte-identical is a
no-regression signal; it is not evidence of conformance. If the specification
is genuinely silent on a question, say so in the PR and propose behaviour
explicitly — do not quietly invent semantics and let the corpus ratify them.

## Pull requests

Before opening one:

- `cargo build --release` is clean, and `cargo clippy` produces no new
  warnings. The workspace denies `unwrap`, `expect`, `panic`, `todo`,
  `unimplemented`, indexing/slicing, and `unsafe` — if you need an escape
  hatch, justify it in the PR rather than adding a blanket `allow`.
- `cargo fmt` has been run.
- The tests for the crates you touched pass, plus `scripts/run-gates.sh --full`
  for runtime or semantics changes.
- New behaviour has a test. A bug fix has a test that fails without the fix.

In the description, please cover: what changed and why; the specification
citation if language behaviour moved; which tests you ran; whether any
generated artifact or snapshot changed and why; and anything a user would
notice, since we are pre-1.0 and breaking changes are allowed but must be
visible.

Keep commits focused. A refactor and a behaviour change in one commit is hard
to review and harder to revert.

### Good first contributions

Bug reports with a minimal reproducing `.sysml` file are genuinely valuable and
are the lowest-friction way to help. Beyond that: improving a diagnostic's
message or span, adding a test that pins behaviour currently only covered
incidentally, fixing documentation that contradicts the code, or adding a small
model under `examples/` that exercises a construct the corpus does not reach.
If you want to attempt a language gap, open an issue first with the construct
and the specification clause — it lets us tell you whether the gap is in the
grammar, the lowering, or the runtime before you spend a regeneration cycle on
it.

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([`LICENSE-MIT`](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
