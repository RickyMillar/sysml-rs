---
title: Lock file and cache
description: How sysml.lock pins your dependency graph, the commands that manage it, and a reproducible team workflow.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/lang/sysml-manifest/src/lock.rs
  - crates/tooling/sysml-resolve/src/lock.rs
  - crates/tooling/sysml-cli/tests/project_lock.rs
---

You and a colleague should resolve *exactly* the same dependency versions from the same `sysml.toml`. That is what `sysml.lock` is for: it records the concrete result of resolution — pinned git commits, archive checksums, resolved registry versions — so the next resolve, on any machine, reproduces it. `sysml.lock` and its commands are sysml-rs conventions, not OMG-standard behaviour.

## The lock file

`sysml lock` (and `sysml add`/`sysml remove`, which refresh it) writes one `[[package]]` entry per resolved package, sorted by name then source for deterministic output:

```toml
lock_version = 1

[[package]]
name = "beverage-types"
version = "0.1.0"
source = "path:../beverage-types"

[[package]]
name = "thermal-model"
version = "1.0.0"
source = "git:file:///path/to/thermal-model-repo#0b22d28aa64e35f59b6d53d7800de8471cbcafd3"
```

The `source` prefix says where a package came from, and how firmly it is pinned:

| Prefix | Example | Pinning |
|---|---|---|
| `path:` | `path:../beverage-types` | Not pinned — read live from disk, no checksum. |
| `git:` | `git:<url>#<commit>` | Exact commit hash, even when the manifest said `tag` or `branch`. |
| `kpar:` | `kpar:<url-or-path>` | `checksum = "sha256:..."` of the archive bytes. |
| `registry:` | `registry:sysand:common-patterns@1.2.0` | Resolved version + `checksum`, with `requested = "^1.0"` recording the original constraint. |
| `stdlib` | — | Reserved in the format for standard-library entries. |

## The commands

All of these run against the nearest `sysml.toml` (walking up from the current directory) and take `--quiet` and `--json`. The outputs below are real runs against a project with one path and one transitive path dependency.

**`sysml lock`** — resolve and write `sysml.lock`, skipping the write when nothing changed. `--force` re-resolves regardless.

```console
$ sysml lock
Lock file is up to date (1 packages)
$ sysml lock --json
{"packages":2,"status":"up_to_date"}
$ sysml lock --force --json
{"lock_path":".../sysml.lock","packages":[{"checksum":null,"name":"beverage-types","requested_requirement":null,"resolved_version":null,"source":"path:../beverage-types","source_detail":null,"version":"0.1.0"},...],"status":"updated"}
```

**`sysml fetch`** — resolve and populate the cache *without* touching `sysml.lock`. Useful for warming a CI cache.

```console
$ sysml fetch
Fetched 1 packages into cache
  beverage-types 0.1.0 (path:../beverage-types)
```

**`sysml update`** — force re-resolution and rewrite the lock; this is how you deliberately move a `branch` dependency to a newer commit or a registry range to a newer release.

```console
$ sysml update
Resolved 1 packages, wrote .../coffee-machine/sysml.lock
  beverage-types 0.1.0 (path:../beverage-types)
```

**`sysml tree`** — the resolved graph, nested; `--json` adds the edge list plus requested/resolved versions.

```console
$ sysml tree
coffee-machine
└── beverage-types 0.1.0 (path:../beverage-types)
    └── thermal-model 1.0.0 (path:../thermal-model-repo)
```

**`sysml why <name>`** — the dependency chain that pulls a package in; the fastest answer to "why is this in my graph?".

```console
$ sysml why thermal-model
coffee-machine -> beverage-types -> thermal-model
```

**`sysml cache clean`** — delete the dependency cache (add `--all` to remove other cache files under the cache root too). Nothing is lost that a re-fetch cannot restore.

```console
$ sysml cache clean
Removed cache: ~/.cache/sysml-rs/dependencies
```

## The cache

Fetched sources live outside your project in a per-user cache — by default `~/.cache/sysml-rs/dependencies/` (Linux), keyed by hashes of the source, with git mirrors/checkouts and checksum-named KPAR archives beneath. Set `SYSML_RS_CACHE_DIR` to move the root, which this page's demos used to isolate runs:

```console
$ SYSML_RS_CACHE_DIR=/tmp/ci-cache sysml fetch
Fetched 2 packages into cache
  beverage-types 0.1.0 (path:../beverage-types)
  thermal-model 1.0.0 (git:...#0b22d28aa64e35f59b6d53d7800de8471cbcafd3)
```

Cached KPAR and registry artifacts are re-verified against their SHA-256 on use; corruption fails loudly with a `checksum mismatch ... remove cache and retry` error rather than loading bad bytes. Details and the failure output are on the [Dependencies page](/sysml-rs/use/dependencies/#cache-location-and-integrity).

## A reproducible team workflow

1. One person edits `sysml.toml` (or runs `sysml add ...`) and runs `sysml lock`.
2. **Commit `sysml.toml` and `sysml.lock` together.** The lock is the reproducibility artifact; a manifest without it lets every machine resolve differently.
3. Everyone else pulls and just works — resolution honours the lock, and `sysml lock` prints `Lock file is up to date`.
4. To move to newer versions on purpose, run `sysml update` and commit the resulting lock diff, reviewed like any other change.

Two habits worth keeping: after any *manual* edit to `sysml.toml`, run `sysml lock` (only `sysml add`/`remove` refresh it automatically); and treat lock diffs in review as real changes — a moved git commit or registry version is a change to what your model means.

## CI recommendation

There is currently no `--locked`/`--check` mode that fails when the lock is stale (verified against `sysml lock --help`), so enforce freshness with a diff check:

```bash
sysml lock          # resolves; rewrites sysml.lock only if stale
git diff --exit-code sysml.lock   # fail the build if it changed
```

`sysml lock --json` exits non-zero on resolution failure and reports `"status": "up_to_date" | "updated"` on success, so the pair above catches both a broken graph and a stale lock. To keep CI fast, cache the directory `SYSML_RS_CACHE_DIR` points at between runs and warm it with `sysml fetch`.
