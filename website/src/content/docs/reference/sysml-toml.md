---
title: sysml.toml schema
description: The complete sysml.toml manifest schema — every table and field with its type, default, and required status.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 11bd751
source_of_truth:
  - crates/lang/sysml-manifest/src/manifest.rs
  - crates/lang/sysml-manifest/src/dependency.rs
  - crates/lang/sysml-manifest/src/stdlib.rs
known_limitations: /sysml-rs/reference/known-limitations/
---

This is the schema reference for `sysml.toml`: every table and field the parser accepts, with types, defaults, and strictness rules, sourced from the `sysml-manifest` crate and its unit tests. For what the manifest is *for* and how to work with it day to day, read [The sysml.toml manifest](/sysml-rs/use/sysml-toml/) first. The format is a sysml-rs convention (Cargo-style TOML), not part of the SysML v2 standard.

Every fenced TOML example on this page was validated by running `sysml info --manifest-path <file> --json` against the release CLI (`sysml 0.1.0`) at the commit in this page's footer; the stated rejection behaviours were reproduced the same way.

## Top-level tables

| Table | Required | Purpose |
|---|---|---|
| `[project]` | yes | Project identity and metadata. |
| `[package]` | no | Publishing / interchange identity (IRI). |
| `[stdlib]` | no | Standard-library selection policy. |
| `[dependencies]` | no | Project dependencies, one entry per name. |
| `[workspace]` | no | Multi-project workspace declaration (root manifest only). |

### Parser strictness

- A missing required field is a hard parse error (`missing field \`version\``).
- Unknown keys inside `[stdlib]` are **rejected** (`unknown field \`systems\`, expected \`include_only\` or \`exclude\``) — this is what catches the retired boolean-keyed stdlib form.
- Unknown keys elsewhere — including unrecognized keys in `[project]` and entire unrecognized top-level tables — are currently **ignored**, not rejected. Do not rely on the parser to catch typos outside `[stdlib]`.

## `[project]`

```toml
[project]
name = "coffee-machine"
version = "0.1.0"
description = "Smart coffee machine system model"
license = "MIT"
sysml-edition = "2025"
authors = ["Alice <alice@example.com>"]
```

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `name` | string | yes | — | Project name; dependents refer to the project by this name. |
| `version` | string | yes | — | Project version (semver string). |
| `description` | string | no | none | One-line human summary. |
| `license` | string | no | none | SPDX license identifier. |
| `sysml-edition` | string | no | `"2025"` | Targeted SysML spec edition year; determines the standard-library version. |
| `authors` | array of strings | no | `[]` | Author strings. |

## `[package]`

```toml
[package]
iri = "urn:acme:coffee-machine"
```

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `iri` | string | no | `urn:sysml:<name>` | Canonical IRI used in interchange metadata (`.project.json` `usage[]` entries). When omitted, the effective IRI is auto-generated from the project name. |

## `[stdlib]`

```toml
[stdlib]
include_only = ["semantic", "data-type", "function", "systems", "analysis"]
exclude = ["analysis"]
```

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `include_only` | array of strings | no | `[]` (= all libraries) | If non-empty, restrict the enabled standard libraries to this allowlist. |
| `exclude` | array of strings | no | `[]` | Remove libraries from the enabled set, applied **after** `include_only`. The special values `"all"` (case-insensitive) or `"*"` disable every standard library. |

These are the only two keys `[stdlib]` accepts; anything else is a parse error. With no `[stdlib]` section at all, every standard library is enabled.

The library names accepted in both lists (from `StdlibConfig::known_library_names()`): `semantic`, `data-type`, `function`, `systems`, `analysis`, `cause-and-effect`, `geometry`, `metadata`, `quantities-and-units`, `requirement-derivation`. `sysml info --json` reports the effective selection and is the authoritative view of how a given manifest resolves.

Validated: a manifest whose `[stdlib]` is `exclude = ["all"]` reports `"stdlib": []` from `sysml info --json`.

## `[dependencies]`

Each entry maps a dependency name to a source, in one of two syntactic forms:

```toml
[dependencies]
# Short form: a bare string is a registry version constraint.
common-patterns = "^1.0"

# Table form: explicit source keys.
beverage-types = { path = "../beverage-types" }
thermal-model = { git = "https://github.com/acme/thermal-model", tag = "v1.0.0" }
certified-parts = { kpar = "https://example.com/certified-parts-1.2.0.kpar" }
pinned-lib = { version = "^2.1", registry = "internal" }
```

Table-form keys:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | string | one source key | Another project directory on disk (must contain its own `sysml.toml`). |
| `git` | string | one source key | Git repository URL. |
| `tag` | string | no (git only) | Git tag to resolve. |
| `branch` | string | no (git only) | Git branch to resolve. |
| `rev` | string | no (git only) | Git commit hash to resolve. |
| `kpar` | string | one source key | A `.kpar` archive by relative path, `file://` URL, or `http(s)://` URL. |
| `version` | string | one source key | Registry version constraint (table-form equivalent of the bare string). |
| `registry` | string | no (registry only) | Registry backend identifier; omitted means the default registry. |

Rules the schema itself encodes:

- A git dependency with none of `tag` / `branch` / `rev` resolves the repository's default branch. When more than one is present, the schema's resolution order is `tag`, then `branch`, then `rev` (first present wins).
- The parser accepts a table that names more than one source key (for example both `path` and `git`); which source wins is decided by the resolution layer, not the schema — do not write such entries. See [Dependencies](/sysml-rs/use/dependencies/) for the operational behaviour and current limitations of each source kind.
- Standard libraries never appear under `[dependencies]`; they are selected in `[stdlib]`.

## `[workspace]`

```toml
[project]
name = "beverage-workspace"
version = "0.1.0"

[workspace]
members = ["beverage-types", "coffee-machine"]
exclude = ["legacy"]
default-members = ["coffee-machine"]

[workspace.project]
sysml-edition = "2025"
license = "MIT"
version = "0.1.0"
```

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `members` | array of strings | no | `[]` | Member project directories, relative to the workspace root. |
| `exclude` | array of strings | no | `[]` | Paths excluded from member expansion (applied after `members`, mirroring Cargo). |
| `default-members` | array of strings | no | `[]` | Members targeted by root-level commands. |
| `project` | table | no | none | Shared `[workspace.project]` defaults members can inherit. |

`[workspace.project]` accepts exactly three optional string fields: `sysml-edition`, `license`, and `version`.

**Experimental / partial support** — the workspace section is parsed and reported (`sysml info` shows `"is_workspace": true`), but member lists do not yet drive resolution or command routing, and `[workspace.project]` defaults are not yet inherited. See [Workspaces](/sysml-rs/use/workspaces/).

## Minimal and complete examples

The minimal valid manifest (what `sysml init` writes):

```toml
[project]
name = "probe"
version = "0.1.0"
```

A manifest exercising every non-workspace table and field, validated as above:

```toml
[project]
name = "coffee-machine"
version = "0.1.0"
description = "Smart coffee machine system model"
license = "MIT"
sysml-edition = "2025"
authors = ["Alice <alice@example.com>"]

[package]
iri = "urn:acme:coffee-machine"

[stdlib]
include_only = ["semantic", "data-type", "function", "systems", "analysis"]
exclude = ["analysis"]

[dependencies]
beverage-types = { path = "../beverage-types" }
thermal-model = { git = "https://github.com/acme/thermal-model", tag = "v1.0.0" }
certified-parts = { kpar = "https://example.com/certified-parts-1.2.0.kpar" }
common-patterns = "^1.0"
pinned-lib = { version = "^2.1", registry = "internal" }
```

For this manifest, `sysml info --json` reports the five dependencies, `"iri": "urn:acme:coffee-machine"`, and `"stdlib": ["semantic", "data-type", "function", "systems"]` (the allowlist minus the exclusion). Parsing does not resolve dependencies — run [`sysml lock`](/sysml-rs/use/lock-and-cache/) for that.
