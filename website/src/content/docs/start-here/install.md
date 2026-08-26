---
title: Install from source
description: Build the sysml CLI from a fresh clone — prerequisites, the specification fetch, parser generation, and the release build.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - README.md
  - CONTRIBUTING.md
  - tools/fetch-references/fetch.sh
  - .github/workflows/release.yml
known_limitations: /sysml-rs/reference/known-limitations/
---

The goal of this page is a working `sysml` binary on your machine. There are
two routes: **download a prebuilt binary** (fastest), or **build from source**
(needed on Windows, and for developing sysml-rs).

## Download a prebuilt binary

The [latest release](https://github.com/RickyMillar/sysml-rs/releases/latest)
publishes standalone binaries. They are self-contained: no separate toolchain,
no specification fetch, no parser generation.

| Platform | CLI asset |
|---|---|
| Linux x86-64 | `sysml-x86_64-unknown-linux-gnu` |
| Linux ARM64 | `sysml-aarch64-unknown-linux-gnu` |
| macOS Apple Silicon | `sysml-aarch64-apple-darwin` |
| macOS Intel | `sysml-x86_64-apple-darwin` |

```bash
curl -L -o sysml \
  https://github.com/RickyMillar/sysml-rs/releases/latest/download/sysml-x86_64-unknown-linux-gnu
chmod +x sysml
./sysml --version
# sysml 0.1.0
```

On macOS, Gatekeeper blocks unsigned downloaded binaries: the release is not
notarised, so clear the quarantine attribute yourself
(`xattr -d com.apple.quarantine sysml`) if you trust it, or build from source.

Two things worth knowing before you file a bug:

- **The binary reports `0.1.0` even in the `v0.1.1` release.** The crate
  version has not been bumped alongside the tag. Use the release tag, not
  `--version`, to say which build you have.
- **There is no Windows CLI binary.** The release ships a Windows *language
  server* (inside the VS Code extension), but not `sysml.exe`. On Windows,
  build from source below, or use WSL with the Linux binary.

The language server and the VS Code extension packages are published in the
same release — see [editor setup](/sysml-rs/use/editors/).

## Build from source

The build has two unusual steps before `cargo build` — fetching the OMG
specification sources and generating the parser — and it genuinely does not
work without them. Budget tens of minutes for the first build; every later
build is an ordinary incremental Rust build.

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust toolchain | CI builds on 1.92.0; no MSRV declared yet | The workspace itself |
| Node.js | 20 | The tree-sitter CLI and the editor packages |
| `tree-sitter-cli` | **exactly 0.26.5** | Parser generation (see the ABI note below) |
| C compiler | any working `cc` | Compiles the generated parser |
| `git`, `curl`, `awk`, `sha256sum` | any | Used by the reference fetch script |
| Disk | several GB free for `target/` | Rust build artifacts |

Git LFS is **not** required — public clones carry no LFS objects.

## 1. Clone

```bash
git clone https://github.com/RickyMillar/sysml-rs
cd sysml-rs
```

## 2. Fetch the specification sources

The OMG SysML v2 and KerML materials (specification documents, Xtext grammars,
TTL metamodel) are fetched, not vendored — they are published upstream under
their own terms. A pinned script reconstructs `references/sysmlv2/` at the
exact revisions this tree is built against, verifying each item against a
recorded checksum:

```bash
tools/fetch-references/fetch.sh
```

Expect about 210 MB and a few minutes on a first run. Re-runs are idempotent:
anything that already verifies is skipped. Two other modes are useful later:
`fetch.sh verify` re-checks an existing tree offline, and `fetch.sh list`
prints the pinned inventory.

This step is not optional and not just for spec lookups: `sysml-core`'s build
script generates element kinds, property accessors, and the validation
dispatcher from the fetched TTL and Xtext files, so `cargo build` fails in
`build.rs` without them.

## 3. Generate the parser

The tree-sitter parser is generated, not committed — `src/parser.c` is a
single ~80 MB table-driven C file:

```bash
npm install -g tree-sitter-cli@0.26.5
cd crates/lang/sysml-parser-incremental/tree-sitter
./generate_from_xtext.sh          # keyword/operator/enum tables, from the Xtext grammars
tree-sitter generate --abi 14     # ABI 14 exactly — see below
cd -
```

`generate_from_xtext.sh` reads the grammars fetched in step 2, so the order
matters. `--abi 14` is not a style preference: the Rust `tree-sitter` crate
this workspace builds against reads ABI 14, and a parser generated at ABI 15
compiles cleanly and then **segfaults at parse time**, looking like a memory
bug rather than a version mismatch.

**This is why the first build is slow.** Generation takes tens of minutes for
a grammar this size (the project's own CI notes ~50 minutes for a cold
regeneration, and shares the generated parser between CI jobs as a checksummed
artifact — but no pre-generated parser is published for local builds today).
It is a one-time cost until you change the grammar.

## 4. Build

```bash
cargo build --release
```

Release, not debug — the physics solver is unusably slow unoptimised. This
builds the workspace's default members, which is everything except the Tauri
desktop shell. The binary lands at `target/release/sysml`.

## 5. Smoke test

Check a real model from the repository's example corpus:

```bash
./target/release/sysml check examples/espresso-pump-hybrid/Physics/HydraulicConstraints.sysml
```

```text
[PASS] NonNegativeThresholds: pWarning >= 0.0 and exposureTrip > 0.0
[PASS] PositiveConductance: restrictionConductance > 0.0
[PASS] RegularizedRoot: epsRoot > 0.0

3/3 constraints passed, 0 failed
```

If you see that, you have a working install — continue with
[Your first model](/sysml-rs/start-here/first-model/). `sysml --help` is the
authoritative list of what else the CLI offers.

## Troubleshooting

- **A build error naming `references/sysmlv2`** means step 2 was skipped or is
  incomplete. Run `tools/fetch-references/fetch.sh` again; it only fetches
  what fails verification.
- **A segfault while parsing** almost always means the parser was generated at
  ABI 15. Regenerate with `tree-sitter generate --abi 14` using
  `tree-sitter-cli` 0.26.5 exactly.
- **The tool looks hung on physics or simulation commands** — check you built
  and are running `target/release/sysml`, not a debug binary.
- **The first build taking a very long time is normal** — parser generation
  dominates it and does not repeat.

If you are setting up to modify sysml-rs itself (tests, grammar work, the
editor packages and their install-order gotcha), follow
[CONTRIBUTING.md](https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md)
instead — it is the maintained developer setup.

## What was verified

The smoke-test command and its output were executed against a release build of
the commit this page is stamped with. The cold-clone steps (fetch, parser
generation, first build) follow the maintained README and CONTRIBUTING text
and the CI workflow definitions rather than a fresh end-to-end run for this
page.
