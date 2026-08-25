---
title: The sysml.toml manifest
description: Every section and field of a sysml-rs project manifest, with validated examples.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/lang/sysml-manifest/src/manifest.rs
  - crates/lang/sysml-manifest/src/stdlib.rs
  - crates/tooling/sysml-cli/tests/project_init.rs
---

`sysml.toml` is what turns a directory of `.sysml` files into a project: it names the project, selects the standard libraries it uses, and declares its dependencies. The format is a sysml-rs convention (Cargo-style TOML), not part of the SysML v2 standard — another SysML tool will not read it. Every example on this page was validated by running the CLI against it.

`sysml init` creates the minimal manifest — this is also the only content a fresh project has besides `src/main.sysml`:

```toml
[project]
name = "probe"
version = "0.1.0"
```

`sysml info` (add `--json` for scripting) is the quickest way to see how the tool interprets a manifest — it prints the project identity, effective IRI, and effective standard-library selection.

## `[project]` — identity

```toml
[project]
name = "coffee-machine"
version = "0.1.0"
description = "Smart coffee machine system model"
license = "MIT"
sysml-edition = "2025"
authors = ["Alice <alice@example.com>"]
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Project name; other projects refer to it by this name. |
| `version` | yes | Semantic version of the project. |
| `description` | no | One-line human summary. |
| `license` | no | SPDX identifier (`"MIT"`, `"Apache-2.0"`, …). |
| `sysml-edition` | no | Targeted SysML edition; defaults to `"2025"`. |
| `authors` | no | List of author strings. |

## `[package]` — IRI

Every project has an IRI used to identify it in interchange metadata. Without a `[package]` section it is auto-generated as `urn:sysml:<name>`; `sysml info` shows the effective value:

```console
$ sysml info
Project: probe
Version: 0.1.0
SysML Edition: 2025
IRI: urn:sysml:probe
...
```

To publish under a stable identifier that is independent of the project name, set it explicitly:

```toml
[package]
iri = "urn:acme:probe"
```

## `[stdlib]` — standard-library selection

This section is sysml-rs selection policy, not a SysML language rule. With no `[stdlib]` section, **every** library in the embedded standard-library set is enabled. `sysml info --json` reports the effective selection and is the authoritative list; on a fresh project it prints:

```json
"stdlib": [
  "semantic",
  "data-type",
  "function",
  "systems",
  "analysis",
  "cause-and-effect",
  "geometry",
  "metadata",
  "quantities-and-units",
  "requirement-derivation"
]
```

Two keys narrow the set — `include_only` is an allowlist, and `exclude` removes entries after the allowlist is applied:

```toml
[stdlib]
include_only = ["semantic", "data-type", "function", "systems", "analysis"]
exclude = ["analysis"]
```

With that manifest, `sysml info --json` reports `["semantic", "data-type", "function", "systems"]`.

An `exclude` entry of `"all"` or `"*"` disables every standard library (`"stdlib": []`) — useful when [packaging a KPAR that must not carry standard-library usage entries](/sysml-rs/use/kpar/#known-gap-default-archives-are-not-consumable-as-dependencies).

Only these two keys are accepted. Older boolean-keyed selections are rejected outright:

```console
$ sysml info --manifest-path legacy-test.toml    # contains: [stdlib] systems = true
error: failed to load legacy-test.toml: failed to parse legacy-test.toml: unknown field `systems`, expected `include_only` or `exclude`
```

Standard libraries never appear under `[dependencies]` — they ship with the tooling and are only ever selected here.

## `[dependencies]` — other projects

Each entry maps a dependency name to a source. Four source forms exist; see [Dependencies](/sysml-rs/use/dependencies/) for what each supports today:

```toml
[dependencies]
beverage-types = { path = "../beverage-types" }
thermal-model = { git = "file:///path/to/thermal-model-repo", tag = "v1.0.0" }
certified-parts = { kpar = "../beverage-types/target/package/beverage-types-0.1.0.kpar" }
common-patterns = "^1.0"
```

- `path` — another project directory on disk (must contain its own `sysml.toml`).
- `git` — a repository URL plus at most one of `tag`, `branch`, or `rev`; with none, the default branch is used.
- `kpar` — a `.kpar` archive by relative path, `file://` URL, or `http(s)://` URL.
- A bare version string (or `version = "..."` with an optional `registry = "..."` backend key) — a registry constraint, resolved against a configured registry index.

`sysml add <name> --path|--git|--kpar ...` edits this section for you and refreshes `sysml.lock`; registry entries are added by editing the manifest directly. `sysml remove <name>` deletes an entry. After hand-editing, run [`sysml lock`](/sysml-rs/use/lock-and-cache/) to re-resolve.

## `[workspace]` — multi-project layout

A manifest may declare a workspace listing member project directories:

```toml
[project]
name = "beverage-workspace"
version = "0.1.0"

[workspace]
members = ["beverage-types", "coffee-machine"]
exclude = []
default-members = ["coffee-machine"]

[workspace.project]
sysml-edition = "2025"
license = "MIT"
```

The schema accepts `members`, `exclude`, `default-members`, and a `[workspace.project]` table of shared defaults (`sysml-edition`, `license`, `version`).

**Experimental / partial support** — today the workspace section is parsed and surfaced (`sysml info` lists the members and reports `"is_workspace": true`), but member lists do not yet drive resolution or command routing, and `[workspace.project]` defaults are not yet inherited by member manifests. Cross-member resolution works through ordinary `path` dependencies between the members. See [Workspaces](/sysml-rs/use/workspaces/) for what is real today.

## A complete example

This manifest exercises every section and was validated with `sysml info --json` and `sysml lock` (the path dependency resolving for real):

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

[dependencies]
beverage-types = { path = "../beverage-types" }
```
