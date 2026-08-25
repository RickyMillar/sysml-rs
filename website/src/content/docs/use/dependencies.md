---
title: Dependencies
description: The four dependency sources — path, git, KPAR, registry — with their current support status, caching, and integrity rules.
scope:
  - sysml-rs tooling
  - Experimental / partial support
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/tooling/sysml-resolve/src/providers.rs
  - crates/tooling/sysml-resolve/src/registry.rs
  - crates/tooling/sysml-resolve/tests/
known_limitations: /sysml-rs/reference/known-limitations/
---

Your model needs definitions that live in another project. Declaring that project under `[dependencies]` in `sysml.toml` makes the toolchain fetch its sources, cache them, load them alongside yours, and pin the resolved result in [`sysml.lock`](/sysml-rs/use/lock-and-cache/). Resolution is transitive (a dependency's dependencies are resolved too), deduplicates by package identity, and refuses cycles:

```console
$ sysml lock        # cyc-a depends on cyc-b, cyc-b depends on cyc-a
error: dependency resolution failed: dependency cycle detected: cyc-a -> cyc-b -> cyc-a
```

Dependency manifests and lock files are sysml-rs conventions, not OMG-standard behaviour. Four source forms exist; their current status differs, so each is labelled below.

## Path dependencies — supported

The workhorse during development. Points at another project directory (which must contain its own `sysml.toml`), relative to the manifest that declares it:

```toml
[dependencies]
beverage-types = { path = "../beverage-types" }
```

```console
$ sysml add beverage-types --path ../beverage-types
Adding dependency 'beverage-types'
Added 'beverage-types' to .../coffee-machine/sysml.toml
$ sysml tree
coffee-machine
└── beverage-types 0.1.0 (path:../beverage-types)
```

Path sources are read in place — nothing is copied — so the lock entry has no checksum. Chains of path dependencies resolve transitively:

```console
$ sysml why thermal-model
coffee-machine -> beverage-types -> thermal-model
```

## Git dependencies — supported (uses your system `git`)

A repository URL pinned by `tag`, `branch`, or `rev`; with none of the three, the default branch's head is used:

```toml
[dependencies]
thermal-model = { git = "file:///path/to/thermal-model-repo", tag = "v1.0.0" }
```

Resolution shells out to the `git` binary on your `PATH`: the repository is mirrored once into the cache, and each resolved commit gets its own detached checkout under it. Whatever the reference form, the lock records the concrete commit:

```toml
[[package]]
name = "thermal-model"
version = "1.0.0"
source = "git:file:///path/to/thermal-model-repo#0b22d28aa64e35f59b6d53d7800de8471cbcafd3"
```

This example uses a local `file://` repository, which is also how the demonstration above was actually run; remote URLs go through the same `git clone --mirror` path, so anything your `git` can reach and authenticate against should work, but network fetching was not exercised for this page.

## KPAR dependencies — supported for local paths and `file://`/`http(s)://` URLs

A pre-built [`.kpar` archive](/sysml-rs/use/kpar/) consumed directly:

```toml
[dependencies]
certified-parts = { kpar = "../vendor/certified-parts-2.0.0.kpar" }
```

Local relative paths and absolute `file://` URLs are read from disk; `http://`/`https://` URLs are downloaded (verified by the provider test suite; the local-path flow is demonstrated end-to-end on the [KPAR page](/sysml-rs/use/kpar/)). Other URL schemes are rejected. The archive's SHA-256 is computed, the archive is cached and extracted under that checksum, and the checksum lands in the lock:

```toml
[[package]]
name = "beverage-types"
version = "0.1.0"
source = "kpar:../beverage-types/target/package/beverage-types-0.1.0.kpar"
checksum = "sha256:e1e91302b64a705e00be0b3f9a611821ec7a83ee3844134e9c6d271e03ccc6bb"
```

**Watch out:** an archive whose `.project.json` carries `usage[]` entries has those entries mapped to further dependencies when it is consumed. Archives built by `sysml package` with the default standard-library selection currently fail to resolve because of this — see [the known gap on the KPAR page](/sysml-rs/use/kpar/#known-gap-default-archives-are-not-consumable-as-dependencies).

## Registry dependencies — Experimental / partial support

A version constraint against a package registry, in Cargo-style short or long form:

```toml
[dependencies]
common-patterns = "^1.0"
# equivalently: common-patterns = { version = "^1.0", registry = "sysand" }
```

Current status, from `crates/tooling/sysml-resolve/src/registry.rs` and its tests:

- Exactly one backend exists, **`sysand`**, and it is the default; any other `registry = "..."` value fails with `registry-backend-<name>`.
- The Sysand index is found via the `SYSML_REGISTRY_SYSAND_INDEX` environment variable (a local path or URL), or by walking up from the project for `.sysml/registries/sysand/index.json`. A remote index is cached for five minutes.
- Exact versions and semver ranges both resolve; artifacts are KPAR archives fetched from the index's `artifact` URL and verified against the index's `sha256:` checksum before extraction.

With a local index mapping `common-patterns` 1.2.0 to an archive, `^1.0` resolves and the lock records both the request and the pinned result:

```console
$ sysml lock
Resolved 1 packages, wrote .../reg-consumer/sysml.lock
  common-patterns 1.2.0 (registry:sysand:common-patterns@1.2.0)
```

```toml
[[package]]
name = "common-patterns"
version = "1.2.0"
source = "registry:sysand:common-patterns@1.2.0"
checksum = "sha256:43440f4efb5b423c088d002d5afcff3b6fc83a40bf1185eb0795529aff3a07c1"
requested = "^1.0"
```

Without a configured index, any registry dependency fails hard — there is no public default registry yet:

```console
$ sysml lock
error: dependency resolution failed: dependency source type not yet supported: registry-sysand-unconfigured (dependency 'registry')
```

`sysml add` cannot add registry dependencies (`--path`, `--git`, `--kpar` only); edit `sysml.toml` directly.

## Cache location and integrity

Fetched sources land in a per-user cache, keyed by a SHA-256 of the source:

```text
~/.cache/sysml-rs/dependencies/
├── git/<hash-of-url>/{objects, checkouts/<commit>}
├── kpar/<hash-of-source>/{archives/<sha256>.kpar, extracted/<sha256>/}
└── registry/<backend>/<hash-of-request>/{artifacts, extracted}
```

Set `SYSML_RS_CACHE_DIR` to relocate the whole cache root (useful in CI). `sysml cache clean` deletes the dependency cache; everything re-fetches on the next resolve.

KPAR and registry artifacts are checksum-verified on every use. A corrupted cache entry is reported, never silently used:

```console
$ sysml fetch
error: dependency resolution failed: checksum mismatch at .../archives/e1e91302....kpar: expected sha256:e1e91302..., got sha256:c5d1a275.... Cached KPAR archive is corrupted; remove cache and retry
```

Recovery is exactly what the message says: `sysml cache clean`, then fetch again. Path dependencies carry no checksum (they are read live from disk), and git integrity rests on the commit hash recorded in the lock.
