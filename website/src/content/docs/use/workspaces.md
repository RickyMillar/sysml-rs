---
title: Workspaces
description: Organising multiple related projects side by side — what discovery, nested boundaries, and the [workspace] section actually do today.
scope:
  - sysml-rs tooling
  - Experimental / partial support
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - crates/lang/sysml-manifest/src/discovery.rs
  - crates/lang/sysml-project/src/discovery.rs
  - crates/tooling/sysml-cli/src/info.rs
  - crates/lang/sysml-project/src/workspace.rs
---

As a model grows you split it: shared vocabulary in one project, the system model in another, side by side in one repository. This page covers how sysml-rs finds projects, where one project ends and the next begins, and — honestly — how much of the `[workspace]` manifest section is wired up today. Everything here is sysml-rs tooling behaviour, not SysML language semantics.

## The layout that works today

```text
beverage-workspace/
    sysml.toml              # root manifest with [workspace]
    beverage-types/
        sysml.toml
        src/types.sysml
    coffee-machine/
        sysml.toml          # depends on beverage-types by path
        src/main.sysml
```

Members reference each other with ordinary [path dependencies](/sysml-rs/use/dependencies/) — that, not the `[workspace]` member list, is what makes cross-member imports resolve:

```toml
# coffee-machine/sysml.toml
[project]
name = "coffee-machine"
version = "0.1.0"

[dependencies]
beverage-types = { path = "../beverage-types" }
```

Verified from inside the member: `sysml lock` resolves the sibling, and `sysml inspect --workspace . --focus main.sysml --diagnostics` reports zero diagnostics for a model that imports `BeverageTypes` from the sibling project.

## How commands find your project

Commands that need a manifest (`info`, `lock`, `tree`, …) walk **up** the directory tree from where you run them and use the **nearest** `sysml.toml`. Run from `beverage-workspace/coffee-machine/src`, `sysml info` reports the `coffee-machine` member — not the workspace root:

```console
$ cd beverage-workspace/coffee-machine/src && sysml info
Project: coffee-machine
Version: 0.1.0
...
Dependencies:
  beverage-types (path: ../beverage-types)
```

The nearest manifest wins even when an ancestor manifest has a `[workspace]` section; a workspace root does **not** take priority over a member. To address a specific manifest regardless of your shell location, pass it explicitly: `sysml info --manifest-path /path/to/sysml.toml`.

## Nested-project boundaries

File discovery is Cargo-style isolated: scanning a project collects its `.sysml`/`.kerml` files but **stops at any nested directory containing its own `sysml.toml`** — a nested manifest marks a separate project, not more files of yours. You can see the boundary from the workspace root, which owns no source files itself:

```console
$ cd beverage-workspace && sysml inspect --workspace . --focus main.sysml --diagnostics
error: no .sysml files found in '.'
```

The members' files belong to the members. Analysis therefore runs per project (from inside a member), with dependency resolution pulling in the other projects' sources.

## The `[workspace]` section — Experimental / partial support

The root manifest may declare:

```toml
[project]
name = "beverage-workspace"
version = "0.1.0"

[workspace]
members = ["beverage-types", "coffee-machine"]
default-members = ["coffee-machine"]

[workspace.project]
sysml-edition = "2025"
license = "MIT"
```

What this does **today**, verified against the CLI and the crate sources:

- The schema parses: `members`, `exclude`, `default-members`, `[workspace.project]` (`sysml-edition`, `license`, `version`).
- `sysml info` at the root reports `"is_workspace": true` and lists the members:

  ```console
  $ sysml info
  Project: beverage-workspace
  ...
  Workspace Members:
    - beverage-types
    - coffee-machine
  ```

What it does **not** do yet: the member list does not drive resolution, discovery, or command routing (running a command in a member neither consults nor requires the root's `members`), `default-members` selects nothing, and `[workspace.project]` defaults are not inherited into member manifests. Treat the section as declarative metadata for now and rely on path dependencies for actual cross-member resolution.

## Legacy interchange: `.project.json` / `.workspace.json`

Separately from `sysml.toml`, sysml-rs implements the KerML model-interchange project format (KerML Clause 10): `.project.json` and `.meta.json` describe a project, `.workspace.json` lists multiple projects (each entry a relative `path` plus its `iris`), and [`.kpar` archives](/sysml-rs/use/kpar/) bundle them. These are interchange artifacts that tooling generates and consumes — day-to-day authoring uses `sysml.toml`, and the two do not mix:

```console
$ sysml project info        # legacy command group, in a sysml.toml project
error: no SysML project found (missing .project.json)
```

The legacy `sysml project` command group (`init`, `info`, `stdlib`) operates on the JSON format; `sysml project stdlib` lists the embedded standard-library projects. Use it only when working with interchange artifacts from other tools — for everything else, the top-level commands (`init`, `info`, `add`, `lock`, `package`, …) and `sysml.toml` are the supported path.
