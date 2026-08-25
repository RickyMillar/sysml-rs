---
title: KPAR archives
description: Building .kpar interchange archives with sysml package, what they contain, and the current gaps.
scope:
  - SysML v2 / KerML
  - sysml-rs tooling
  - Experimental / partial support
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/tooling/sysml-cli/src/package.rs
  - crates/lang/sysml-project/src/kpar/
  - crates/tooling/sysml-resolve/tests/kpar_resolution.rs
known_limitations: /sysml-rs/reference/known-limitations/
---

You want to hand your model to someone — or some tool — as a single file. That is a `.kpar` archive: the KerML model-interchange package format (KerML Clause 10), a zip bundling your source files with JSON project metadata. The *format* is OMG-standard; the workflow around it here is sysml-rs tooling.

The division of labour: you **author** in `sysml.toml` and `.sysml` files, and the tool **generates** the interchange artifacts. `.project.json` and `.meta.json` are not files you maintain by hand — `sysml package` derives them from your manifest at build time, and consuming tools read them back out of the archive.

## Building an archive

```console
$ sysml package
Packaged beverage-types (1 files, 867 B)
  .../beverage-types/target/package/beverage-types-0.1.0.kpar
```

The archive is written to `target/package/<name>-<version>.kpar`; `--output <dir>` (`-o`) redirects it, and `--manifest-path` selects a manifest explicitly. The file count refers to your source files; metadata files are added on top.

## What is inside

```console
$ unzip -l target/package/beverage-types-0.1.0.kpar
  Length      Date    Time    Name
---------  ---------- -----   ----
        0  1980-01-01 00:00   beverage-types/
     1448  1980-01-01 00:00   beverage-types/.project.json
      150  1980-01-01 00:00   beverage-types/.meta.json
      209  1980-01-01 00:00   beverage-types/types.sysml
```

Everything sits under a root directory named after the project. `.project.json` carries the identity fields from `[project]` (name, version, description, license) plus a `usage[]` array of resources the project uses. `.meta.json` maps top-level model namespaces to the files defining them, and records the creation time and metamodel:

```json
{
  "index": { "BeverageTypes": "types.sysml" },
  "created": "2026-08-25T03:40:54Z",
  "metamodel": "https://www.omg.org/spec/SysML/20250201"
}
```

With the default standard-library selection, the effective [`[stdlib]`](/sysml-rs/use/sysml-toml/) set is emitted into `usage[]` as the OMG library archive URLs with version constraints, for example:

```json
{
  "resource": "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
  "versionConstraint": "1.0.0"
}
```

## Consuming an archive

A `.kpar` is consumed as a [KPAR dependency](/sysml-rs/use/dependencies/): the archive's SHA-256 is computed, it is cached and extracted under that checksum, and the checksum is pinned in `sysml.lock`:

```console
$ sysml add beverage-types --kpar ../beverage-types/target/package/beverage-types-0.1.0.kpar
$ sysml tree
kpar-consumer
└── beverage-types 0.1.0 (kpar:../beverage-types/target/package/beverage-types-0.1.0.kpar)
```

On extraction the tool synthesizes a `sysml.toml` for the archive from its `.project.json`, and maps each non-`urn:` `usage[]` resource to a further dependency (`.kpar` URLs become kpar dependencies; git-like URLs become branch dependencies). That mapping is what makes the next section bite.

## Known gap: default archives are not consumable as dependencies

**Experimental / partial support.** An archive built with the default standard-library selection carries the ten OMG library URLs in `usage[]`; a consumer maps those to dependencies and tries to resolve the OMG archives themselves, which currently ends in a cycle:

```console
$ sysml add beverage-types --kpar ../beverage-types/target/package/beverage-types-0.1.0.kpar
Adding dependency 'beverage-types'
error: dependency resolution failed: dependency cycle detected: kpar-consumer -> beverage-types -> SysML Analysis Library -> Kernel Data Type Library -> Kernel Semantic Library -> data-type-library
```

(The failed `add` rolls your manifest back — the entry is not left behind.)

Until this is fixed, build archives you intend to distribute as dependencies with an empty standard-library usage set:

```toml
[stdlib]
exclude = ["all"]
```

which empties `usage[]` and makes the archive consumable (verified end-to-end above — the checksum-pinned lock entry comes from exactly this flow). The consumer's own `[stdlib]` selection still controls which standard libraries load for *its* models, so excluding them from the producer's packaging metadata does not disable them downstream.

## Reproducibility

`sysml package` is **not** byte-reproducible: zip entry timestamps are fixed (1980-01-01), but `.meta.json` embeds a `created` wall-clock stamp, so packaging the same input twice yields different bytes — verified with two builds a second apart producing different SHA-256s. Consequences:

- Re-packaging churns the archive checksum, which churns kpar `checksum` entries in consumers' lock files even when no model content changed.
- Distribute a *built artifact* (and let the lock pin its checksum); do not expect to regenerate an identical archive from source later.

## Authoring vs interchange, in one table

| | Authoring (what you edit) | Interchange (what tools exchange) |
|---|---|---|
| Files | `sysml.toml`, `src/*.sysml` | `.kpar` containing `.project.json`, `.meta.json`, sources |
| Standard | sysml-rs convention | KerML Clause 10 format |
| Produced by | you | `sysml package` |
| Consumed by | sysml-rs commands | kpar dependencies, the legacy `sysml project` group, other SysML v2 tools |
